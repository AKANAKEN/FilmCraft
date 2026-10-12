//! #1137: MP3 imports must not play late. LAME records the encoder delay/padding in its tag and
//! symphonia trims it when `enable_gapless` is on; without trimming every decoded MP3 starts
//! ~1105 source samples (~25 ms) late against ffmpeg. A click placed at an exact second in the
//! source must land at that second after decode, as ffmpeg decodes it.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use filmcraft_codecs::audio::AudioFileSource;
use filmcraft_media::MediaSource;

/// 4 s of silence at 44.1 kHz with a single loud sample at exactly 1.000, 2.000 and 3.000 s.
fn click_wav() -> Vec<u8> {
    let rate = 44_100usize;
    let mut s = vec![0.0f32; 4 * rate];
    for sec in [1usize, 2, 3] {
        s[sec * rate] = 0.9;
    }
    let mut data = Vec::with_capacity(2 * s.len());
    for v in s {
        data.extend_from_slice(&((v * 32_767.0) as i16).to_le_bytes());
    }
    // minimal mono 16-bit WAV
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&(rate as u32).to_le_bytes());
    w.extend_from_slice(&(rate as u32 * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    w
}

fn ffmpeg() -> Option<PathBuf> {
    let ok = Command::new("ffmpeg").args(["-version"]).stdout(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
    ok.then(|| PathBuf::from("ffmpeg"))
}

/// First sample index of each click group in a mono f32 slice.
fn clicks(x: &[f32]) -> Vec<usize> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (i, v) in x.iter().enumerate() {
        if v.abs() > 0.5 {
            match out.last_mut() {
                Some(g) if i - *g.last().unwrap() <= 50 => g.push(i),
                _ => out.push(vec![i]),
            }
        }
    }
    out.iter().map(|g| g[0]).collect()
}

/// Cached LAME encode of the click fixture (same pattern as the other codec oracle tests).
fn tagged_mp3(ff: &PathBuf) -> Option<PathBuf> {
    let dir = filmcraft_testkit::fixtures_dir("codecs");
    let out = dir.join("clicks_lame_v1.mp3");
    let wav = dir.join("clicks_source_v1.wav");
    std::fs::write(&wav, click_wav()).expect("write wav");
    filmcraft_testkit::fixtures::generate(&out, |tmp| {
        // `-f mp3` is required: the cached temp name does not end in .mp3 and ffmpeg cannot
        // infer the muxer from it.
        Command::new(ff)
            .args(["-y", "-v", "error", "-i"])
            .arg(&wav)
            .args(["-c:a", "libmp3lame", "-b:a", "128k", "-f", "mp3"])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[test]
fn mp3_encoder_delay_is_trimmed() {
    let Some(ff) = ffmpeg() else {
        eprintln!("ffmpeg not available; skipping");
        return;
    };
    let Some(mp3) = tagged_mp3(&ff) else {
        eprintln!("could not generate the LAME fixture; skipping");
        return;
    };
    let bytes: Arc<[u8]> = std::fs::read(&mp3).unwrap().into();

    // Oracle: ffmpeg decodes the clicks at exactly 1/2/3 s at the source rate.
    let raw = Command::new(&ff).args(["-v", "error", "-i"]).arg(&mp3).args(["-map", "0:a:0", "-f", "f32le", "-"]).output().unwrap();
    assert!(raw.status.success());
    let oracle: Vec<f32> = raw.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let want = clicks(&oracle);
    assert_eq!(want.len(), 3, "fixture: ffmpeg should show three clicks, got {want:?}");

    let src = AudioFileSource::decode("tagged.mp3", bytes).expect("decode");
    // Read the decoded file back through the public source API (native rate, no resample). The
    // fixture is 4 s; asking for 8 s of frames just zero-pads past the end, which the click scan
    // and the length compare against ffmpeg handle fine.
    let buf = src.audio(0, 8 * 44_100, 44_100).expect("audio");
    let ours: Vec<f32> = buf.channels[0].clone();
    let got = clicks(&ours);
    assert_eq!(got.len(), 3, "our decode should show three clicks, got {got:?}");
    for (w, g) in want.iter().zip(&got) {
        assert!((*g as i64 - *w as i64).abs() <= 2, "#1137: click at {w} (ffmpeg) decoded at {g} — encoder delay not trimmed");
    }
}
