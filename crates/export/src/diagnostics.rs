//! Observations, not capability probes. Reading diagnostics never creates a device or encoder.

use std::cell::RefCell;
use std::sync::{Mutex, TryLockError};

use serde::Serialize;

use crate::{ExportSettings, Format, HardwareEncoding, Result, VideoEncoder};

/// Most recently completed video encoder selection in this process. It may belong to a finished
/// export or preview; this is not the currently active encoder and says nothing about other jobs.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderSelection {
    pub format: Format,
    pub backend: Option<&'static str>,
    pub kind: &'static str,
    pub fallback_reason: Option<String>,
}

static LATEST: Mutex<Option<EncoderSelection>> = Mutex::new(None);

/// `None` means the snapshot is busy; `Some(None)` means no selection has been observed.
pub fn latest_encoder_selection() -> Option<Option<EncoderSelection>> {
    snapshot(&LATEST)
}

fn snapshot(value: &Mutex<Option<EncoderSelection>>) -> Option<Option<EncoderSelection>> {
    match value.try_lock() {
        Ok(g) => Some(g.clone()),
        Err(TryLockError::Poisoned(e)) => Some(e.into_inner().clone()),
        Err(TryLockError::WouldBlock) => None,
    }
}

thread_local! {
    // Factories run synchronously on the selecting thread. Isolating the reasons here keeps
    // simultaneous exports and background capability probes from attributing each other's errors.
    static REASONS: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// Hardware factory declined this configuration. Call only with a diagnostic, never media paths.
/// Has no effect outside a video encoder selection (including startup capability probes).
pub fn note_encoder_declined(backend: &'static str, reason: &str) {
    REASONS.with(|cell| {
        if let Some(reasons) = cell.borrow_mut().as_mut()
            && reasons.len() < 8
        {
            reasons.push(format!("{backend}: {}", reason.chars().take(512).collect::<String>()));
        }
    });
}

struct Capture(Option<Vec<String>>);

impl Drop for Capture {
    fn drop(&mut self) {
        REASONS.with(|cell| *cell.borrow_mut() = self.0.take());
    }
}

pub(crate) fn observe_selection(settings: &ExportSettings, create: impl FnOnce() -> Result<Box<dyn VideoEncoder>>) -> Result<Box<dyn VideoEncoder>> {
    let _capture = Capture(REASONS.with(|cell| cell.replace(Some(Vec::new()))));
    let result = create();
    let reasons = REASONS.with(|cell| cell.borrow_mut().take().unwrap_or_default());
    let selection = describe(settings, &result, reasons);
    // No driver calls, encoder methods or formatting while holding this lock.
    *LATEST.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(selection);
    result
}

fn describe(settings: &ExportSettings, result: &Result<Box<dyn VideoEncoder>>, reasons: Vec<String>) -> EncoderSelection {
    let format = settings.video_format();
    let (backend, kind) = match result {
        Ok(encoder) => match encoder.diagnostic_backend() {
            Some((name, true)) => (Some(name), "hardware"),
            Some((name, false)) => (Some(name), "software"),
            None => (None, "unknown"),
        },
        Err(_) => (None, "unavailable"),
    };
    let fallback_reason = if result.is_err() {
        // Third-party factories may put private paths into errors. The normal export error/log
        // retains that detail; this shareable report keeps only bounded platform diagnostics.
        let mut message = "Encoder initialization failed; see the export error or application log for details.".to_owned();
        if !reasons.is_empty() {
            message.push(' ');
            message.push_str(&reasons.join("; "));
        }
        Some(message)
    } else if !reasons.is_empty() {
        Some(reasons.join("; "))
    } else if kind == "software" && format == Format::H264 {
        Some(if settings.hardware_encoding == HardwareEncoding::Off {
            "Hardware encoding is off for this export.".into()
        } else {
            "No registered hardware encoder accepted this configuration.".into()
        })
    } else {
        None
    };
    EncoderSelection { format, backend, kind, fallback_reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestEncoder(Option<(&'static str, bool)>);

    impl VideoEncoder for TestEncoder {
        fn diagnostic_backend(&self) -> Option<(&'static str, bool)> {
            self.0
        }
        fn sample_entry(&self) -> filmcraft_isobmff::SampleEntry {
            filmcraft_isobmff::SampleEntry::jpeg(16, 16)
        }
        fn timescale(&self) -> u32 {
            24
        }
        fn encode(&mut self, _: &crate::EncoderFrame) -> Result<Vec<crate::EncodedPacket>> {
            Ok(Vec::new())
        }
        fn flush(&mut self) -> Result<Vec<crate::EncodedPacket>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn hardware_and_unidentified_encoders_are_reported_honestly() {
        let settings = ExportSettings::default();
        let hardware: Result<Box<dyn VideoEncoder>> = Ok(Box::new(TestEncoder(Some(("test hardware", true)))));
        let description = describe(&settings, &hardware, Vec::new());
        assert_eq!(description.kind, "hardware");
        assert!(description.fallback_reason.is_none());
        let unknown: Result<Box<dyn VideoEncoder>> = Ok(Box::new(TestEncoder(None)));
        let description = describe(&settings, &unknown, Vec::new());
        assert_eq!(description.kind, "unknown");
        assert!(description.backend.is_none());
        assert!(description.fallback_reason.is_none());
    }

    #[test]
    fn initialization_errors_do_not_copy_arbitrary_private_paths() {
        let settings = ExportSettings::default();
        let result = Err(crate::ExportError::Io("/private/customer/project.fcproj".into()));
        let description = describe(&settings, &result, vec!["NVENC: no NVIDIA encoder driver".into()]);
        let message = description.fallback_reason.unwrap();
        assert_eq!(description.kind, "unavailable");
        assert!(message.contains("Encoder initialization failed"));
        assert!(message.contains("NVENC: no NVIDIA encoder driver"));
        assert!(!message.contains("/private"));
    }

    #[test]
    fn snapshot_does_not_wait_for_a_writer() {
        let cell = Mutex::new(None);
        let guard = cell.lock().unwrap();
        assert!(snapshot(&cell).is_none());
        drop(guard);
        assert!(snapshot(&cell).unwrap().is_none());
    }

    #[test]
    fn reasons_are_scoped_bounded_and_thread_local() {
        note_encoder_declined("ignored", "outside a selection");
        let _capture = Capture(REASONS.with(|cell| cell.replace(Some(Vec::new()))));
        std::thread::spawn(|| note_encoder_declined("other thread", "unrelated")).join().unwrap();
        for _ in 0..20 {
            note_encoder_declined("NVENC", &"x".repeat(1024));
        }
        REASONS.with(|cell| {
            let reasons = cell.borrow();
            let reasons = reasons.as_ref().unwrap();
            assert_eq!(reasons.len(), 8);
            assert!(reasons.iter().all(|r| r.len() == 7 + 512));
        });
    }

    #[test]
    fn selected_software_is_not_inferred_from_hardware_counters() {
        let mut settings = ExportSettings { hardware_encoding: HardwareEncoding::Auto, ..Default::default() };
        let result = crate::mjpeg_factory(Format::Mjpeg, 16, 16, filmcraft_time::FrameRate::FPS_24, &settings).unwrap();
        let selection = describe(&settings, &result, vec!["NVENC: no NVIDIA encoder driver".into()]);
        assert_eq!(selection.kind, "software");
        assert_eq!(selection.backend, Some("FilmCraft"));
        assert_eq!(selection.fallback_reason.as_deref(), Some("NVENC: no NVIDIA encoder driver"));
        settings.hardware_encoding = HardwareEncoding::Off;
        assert_eq!(describe(&settings, &result, Vec::new()).fallback_reason.as_deref(), Some("Hardware encoding is off for this export."));
        let failure = Err(crate::ExportError::Unsupported("HEVC unavailable".into()));
        assert_eq!(describe(&settings, &failure, Vec::new()).kind, "unavailable");
    }
}
