//! Compare packed SDR and the existing float GPU route through the real image exporter.
//! One scenario runner owns the process-global renderer registry; no media fixtures are needed.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use filmcraft_export::{ExportSettings, Format, FrameRenderer, GpuRendering, OutputSink, Progress, export, register_frame_renderer};
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource};
use filmcraft_project::{
    Caption, CaptionFormat, CaptionTrack, ClipId, ItemId, ItemKind, Label, MediaClip, MediaRef, ParamValue, Project, SequenceSettings, TrackId, TrackKind,
};
use filmcraft_render::{Image, RenderOptions, SourceMap, SourceProvider};
use filmcraft_time::{FrameRate, Tick, TimeRange};

static PACKED: AtomicBool = AtomicBool::new(false);
static PACKED_CALLS: AtomicU64 = AtomicU64::new(0);

struct Renderer(filmcraft_gpu::ExportRenderer, bool);
impl FrameRenderer for Renderer {
    fn render_rgba8(&mut self, project: &Project, seq: ItemId, t: Tick, opts: RenderOptions, sources: &dyn SourceProvider) -> Option<(u32, u32, Vec<u8>)> {
        if !self.1 {
            return None;
        }
        PACKED_CALLS.fetch_add(1, Ordering::Relaxed);
        self.0.render_rgba8(project, seq, t, opts, sources)
    }
    fn render(&mut self, project: &Project, seq: ItemId, t: Tick, opts: RenderOptions, sources: &dyn SourceProvider) -> Option<Image> {
        self.0.render(project, seq, t, opts, sources)
    }
}

fn scene() -> (Arc<Project>, ItemId, SourceMap) {
    let mut p = Project::new("packed parity");
    let rate = FrameRate::FPS_24;
    let duration = rate.tick_of(3);
    let seq = p.new_sequence("layers", SequenceSettings { width: 130, height: 74, frame_rate: rate, ..Default::default() }, 2, 1, None);
    let mut sources = SourceMap::default();
    for (track, generator) in [Generator::BarsAndTone, Generator::ColorMatte { color: [0.8, 0.2, 0.6, 0.7] }].into_iter().enumerate() {
        let source = GeneratorSource::new(generator, 130, 74, rate, duration);
        let info = source.info().clone();
        let id = p.add_item(
            "layer",
            Label::Iris,
            ItemKind::Media(MediaClip {
                media: MediaRef::Generator(source.generator.clone()),
                info,
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
        let mut clip = p.make_track_item(id, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, duration), rate).unwrap();
        if track == 1 {
            let motion = clip.effect_mut("motion").unwrap();
            motion.param_mut("scale").unwrap().value = ParamValue::Float(65.0);
            motion.param_mut("rotation").unwrap().value = ParamValue::Float(17.0);
            clip.effect_mut("opacity").unwrap().param_mut("opacity").unwrap().value = ParamValue::Float(55.0);
            let mut blur = filmcraft_project::find_effect("gaussian_blur").unwrap().instance();
            blur.params.get_mut("blurriness").unwrap().value = ParamValue::Float(5.0);
            clip.effects.push(blur);
        }
        p.sequence_mut(seq).unwrap().video_tracks[track].items.push(clip);
        sources.0.insert(id, Arc::new(source));
    }
    let mut captions = CaptionTrack::new(TrackId(p.alloc_id()), "captions".into(), CaptionFormat::Subtitle);
    captions.style.size = 12.0;
    captions.captions.push(Caption {
        id: ClipId(p.alloc_id()),
        start: Tick::ZERO,
        duration,
        text: "Exact SDR".into(),
        speaker: None,
        cue_id: None,
        settings: String::new(),
    });
    p.sequence_mut(seq).unwrap().caption_tracks.push(captions);
    (Arc::new(p), seq, sources)
}

fn frames(p: &Arc<Project>, seq: ItemId, sources: &SourceMap, settings: &ExportSettings, packed: bool) -> Vec<Vec<u8>> {
    PACKED.store(packed, Ordering::Relaxed);
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = output.clone();
    let mut settings = settings.clone();
    settings.sink = Some(OutputSink(Arc::new(move |_, bytes| {
        sink.lock().unwrap().push(bytes);
        Ok(())
    })));
    export(p, seq, &settings, sources, &Progress::default()).unwrap();
    let result = output.lock().unwrap().clone();
    assert_eq!(result.len(), 3);
    result
}

#[test]
fn packed_export_preserves_layers_effects_captions_and_export_settings() {
    let Some(_) = filmcraft_gpu::ExportRenderer::new() else {
        eprintln!("SKIPPED (packed pipeline): no GPU adapter");
        return;
    };
    register_frame_renderer(|| filmcraft_gpu::ExportRenderer::new().map(|r| Box::new(Renderer(r, PACKED.load(Ordering::Relaxed))) as Box<dyn FrameRenderer>));
    let (p, seq, sources) = scene();
    let settings = ExportSettings {
        format: Format::PngSequence,
        path: "packed-parity.png".into(),
        gpu_rendering: GpuRendering::Auto,
        range: Some(TimeRange::new(Tick::ZERO, FrameRate::FPS_24.tick_of(3))),
        burn_captions: true,
        ..Default::default()
    };
    let reference = frames(&p, seq, &sources, &settings, false);
    let before = PACKED_CALLS.load(Ordering::Relaxed);
    assert_eq!(frames(&p, seq, &sources, &settings, true), reference, "layer/effect/caption PNGs must match exactly");
    assert_eq!(PACKED_CALLS.load(Ordering::Relaxed) - before, 3, "exercise packed rendering on every frame");
    let mut no_captions = settings.clone();
    no_captions.burn_captions = false;
    assert_ne!(frames(&p, seq, &sources, &no_captions, true), reference, "captions must actually be drawn");

    let mut alpha = settings.clone();
    alpha.alpha = true;
    let mut limiter = settings.clone();
    limiter.effects.video_limiter.enabled = true;
    let mut overlay = settings.clone();
    overlay.effects.name_overlay.enabled = true;
    overlay.effects.name_overlay.text = "Export overlay".into();
    let mut resize = settings.clone();
    resize.frame_size = Some((96, 96));
    resize.max_render_quality = true;
    for settings in [alpha, limiter, overlay, resize] {
        let float = frames(&p, seq, &sources, &settings, false);
        let before = PACKED_CALLS.load(Ordering::Relaxed);
        assert_eq!(frames(&p, seq, &sources, &settings, true), float, "settings must retain the float route");
        assert_eq!(PACKED_CALLS.load(Ordering::Relaxed), before, "packed hook must not bypass export processing");
    }
}
