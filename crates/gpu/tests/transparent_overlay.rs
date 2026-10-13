//! Compare planned overlays with the original full source, including export readback.
//! Run the synthetic, decode/encode-free benchmark separately with:
//! `cargo test --release -p filmcraft-gpu --test transparent_overlay overlay_benchmark -- --ignored --nocapture`

use std::sync::Arc;
use std::time::Instant;

use filmcraft_frame::{AudioBuffer, Chroma, PixelData, VideoFrame};
use filmcraft_geom::Affine;
use filmcraft_gpu::{GpuCompositor, prepare};
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{FrameRequest, Generator, MediaInfo, MediaSource};
use filmcraft_project::{ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use filmcraft_render::plan::{FramePlan, PlanLayer, plan_frame};
use filmcraft_render::{Blend, Image, RenderOptions, SourceMap};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

const FPS: FrameRate = FrameRate::FPS_30;

struct Frames {
    info: MediaInfo,
    frames: Vec<Arc<VideoFrame>>,
}

impl MediaSource for Frames {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, req: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        let i = FPS.frame_at(req.time).max(0) as usize % self.frames.len();
        Ok(self.frames[i].clone())
    }
    fn audio(&self, _start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        Ok(AudioBuffer::silence(sample_rate, 2, frames))
    }
}

fn overlay(w: usize, h: usize, bits: u32, rect: (usize, usize, usize, usize), opaque: bool) -> Arc<VideoFrame> {
    let n = w * h;
    let scale = 1u16 << (bits - 8);
    let max = ((1u32 << bits) - 1) as u16;
    let (x0, y0, x1, y1) = rect;
    let alpha = (0..n)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            if opaque {
                max
            } else if x >= x0 && x < x1 && y >= y0 && y < y1 {
                if (x + y) % 3 == 0 { max / 2 } else { max }
            } else {
                0
            }
        })
        .collect();
    Arc::new(VideoFrame {
        width: w as u32,
        height: h as u32,
        data: PixelData::Yuv16 {
            planes: [
                Arc::new((0..n).map(|i| (32 + i % 180) as u16 * scale).collect()),
                Arc::new((0..n).map(|i| (80 + i % 80) as u16 * scale).collect()),
                Arc::new((0..n).map(|i| (96 + i % 64) as u16 * scale).collect()),
            ],
            chroma: Chroma::C444,
            bits,
            alpha: Some(Arc::new(alpha)),
        },
        color: filmcraft_color::ColorInfo::REC709,
        par: (1, 1),
        pts: Tick::ZERO,
    })
}

fn project(frame: Arc<VideoFrame>, moving: bool) -> (Project, filmcraft_project::ItemId, SourceMap) {
    let mut p = Project::new("synthetic transparent overlay");
    let (w, h) = (frame.width, frame.height);
    let seq = p.new_sequence("overlay", SequenceSettings { width: w, height: h, frame_rate: FPS, ..Default::default() }, 2, 1, None);
    let mut map = SourceMap::default();
    for track in 0..2 {
        let g = GeneratorSource::new(Generator::ColorMatte { color: [0.2, 0.4, 0.6, 1.0] }, w, h, FPS, Tick(60 * TICKS_PER_SECOND));
        let info = g.info().clone();
        let item = p.add_item(
            "synthetic",
            Label::Iris,
            ItemKind::Media(MediaClip {
                media: MediaRef::Generator(g.generator.clone()),
                info: info.clone(),
                interpret: Default::default(),
                mark_in: None,
                mark_out: None,
                markers: vec![],
                offline: false,
                proxy: None,
                identity: None,
            }),
            None,
        );
        let clip = p.make_track_item(item, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, FPS.tick_of(1800)), FPS).unwrap();
        p.sequence_mut(seq).unwrap().video_tracks[track].items.push(clip);
        if track == 0 {
            map.0.insert(item, Arc::new(g));
        } else {
            // More distinct decoded-frame allocations than the GPU's 24-entry upload cache.
            // Only Y needs duplicating: the cache keys on its allocation, just as with video.
            let frames = (0..if moving { 32 } else { 1 })
                .map(|_| {
                    let mut f = (*frame).clone();
                    if let PixelData::Yuv16 { planes, .. } = &mut f.data {
                        planes[0] = Arc::new((*planes[0]).clone());
                    }
                    Arc::new(f)
                })
                .collect();
            map.0.insert(item, Arc::new(Frames { info, frames }));
        }
    }
    (p, seq, map)
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    eprintln!("adapter: {:?}", adapter.get_info());
    pollster::block_on(adapter.request_device(&Default::default())).ok()
}

fn draw(c: &mut GpuCompositor, plan: &FramePlan) -> Image {
    let prep = prepare(plan);
    let (w, h, px) = c.render_export_prepared(plan, Some(&prep)).expect("GPU export readback");
    Image { w: w as usize, h: h as usize, px }
}

#[test]
fn planned_overlay_matches_full_source_on_gpu() {
    let Some((d, q)) = device() else {
        eprintln!("SKIPPED: no GPU adapter");
        return;
    };
    for (w, h) in [(127, 73), (320, 180), (1920, 1080), (3840, 2160)] {
        for bits in [10, 12, 16] {
            let mut c = GpuCompositor::new(&d, &q);
            for rect in [(3, 5, w / 2, h / 5), (0, 0, w, 3), (w - 3, h - 5, w, h), (0, 0, 0, 0)] {
                let frame = overlay(w, h, bits, rect, false);
                let (p, seq, map) = project(frame.clone(), false);
                let planned = plan_frame(&p, seq, Tick::ZERO, RenderOptions::default(), &map);
                let FramePlan::Layers { layers, .. } = &planned else { panic!("expected layers") };
                let original =
                    FramePlan::Layers { width: w, height: h, layers: vec![layers[0].clone(), PlanLayer::new(frame, Affine::IDENTITY, 1.0, Blend::Normal)] };
                let expected = draw(&mut c, &original);
                let got = draw(&mut c, &planned);
                let max = expected.px.iter().zip(&got.px).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                let (a, b) = (got.over_black_rgba8(), expected.over_black_rgba8());
                let max_code = a.iter().zip(&b).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
                let changed = a.iter().zip(&b).filter(|(a, b)| a != b).count();
                let mean = a.iter().zip(&b).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / a.len() as f64;
                eprintln!("{w}x{h} {bits}-bit {rect:?}: max_linear={max}, max_code={max_code}, mean={mean:.8}, changed_channels={changed}");
                // A different quad can round interpolated coordinates differently. Allow at
                // most one output code on rare samples, substantially tighter than GPU/CPU
                // golden parity (p99 <= 6, mean < 1.5). The CPU equivalence test is exact.
                assert!(max_code <= 1 && mean < 0.001, "GPU crop parity: max_code={max_code}, mean={mean}, max_linear={max}");
            }
        }
    }
}

#[test]
#[ignore = "manual timing benchmark; run alone in release mode"]
fn overlay_benchmark() {
    let (d, q) = device().expect("a GPU is required for this benchmark");
    println!("rayon threads: {}", rayon::current_num_threads());
    for (name, moving, opaque) in [("sparse-video", true, false), ("opaque-video", true, true), ("sparse-still", false, false)] {
        let frame = overlay(1920, 1080, 12, (100, 30, 1820, 105), opaque);
        let (p, seq, map) = project(frame, moving);
        let mut c = GpuCompositor::new(&d, &q);
        let mut times = Vec::new();
        let mut last = Vec::new();
        let mut source_bytes = 0;
        for i in 0..70 {
            let start = Instant::now();
            let plan = plan_frame(&p, seq, FPS.tick_of(i), RenderOptions::default(), &map);
            source_bytes = plan.source_bytes();
            let image = draw(&mut c, &plan);
            last = image.over_black_rgba8();
            filmcraft_frame::pool::recycle_f32(image.px);
            if i >= 10 {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        times.sort_by(f64::total_cmp);
        let checksum = last.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3));
        println!("{name}: median_ms={:.3} p95_ms={:.3} source_bytes={source_bytes} checksum={checksum:016x}", times[30], times[57]);
    }
}
