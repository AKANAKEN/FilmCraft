//! Read-only acceleration diagnostics. Opening this dialog never probes a driver or starts work.

use serde_json::{Value, json};

use crate::FilmcraftApp;

/// Host-independent diagnostics, extended with this window's preview compositor.
pub fn snapshot(app: &FilmcraftApp) -> Value {
    let mut report = filmcraft_engine::perf::capabilities_json();
    report["preview"] = match &app.gpu {
        _ if crate::panels::menu_dialogs::software_renderer(app) => {
            json!({"state": "cpu", "adapter": null, "reason": "Project Settings → Renderer is Software Only."})
        }
        _ if app.ui.program.display.is_channel() => {
            json!({"state": "cpu", "adapter": null, "reason": "The Program monitor's channel display uses CPU compositing."})
        }
        Some(g) if !g.broken.load(std::sync::atomic::Ordering::Relaxed) => {
            let info = g.render_state.adapter.get_info();
            json!({"state": "gpu", "adapter": {"name": info.name, "backend": format!("{:?}", info.backend)}, "reason": null})
        }
        Some(_) => json!({"state": "cpu", "adapter": null, "reason": "The GPU device reported an error; preview is falling back to the CPU."}),
        None => json!({"state": "cpu", "adapter": null, "reason": app.gpu_fallback_reason.as_deref().unwrap_or(
            "No GPU compositor is attached to this window. The host may use software rendering or may not have supplied a usable adapter."
        )}),
    };
    report
}

fn string<'a>(value: &'a Value, fallback: &'a str) -> &'a str {
    value.as_str().unwrap_or(fallback)
}

/// Stable row identifiers also make diagnostics accessible to automation without parsing prose.
pub fn rows(report: &Value) -> Vec<(&'static str, &'static str, String)> {
    let preview = &report["preview"];
    let decoder = &report["decoder"];
    let observed = &decoder["observed"];
    let latest = &report["encoder"]["latest"];
    let count = |v: &Value| v.as_u64().unwrap_or(0);
    let preview_text = if preview["state"] == "gpu" {
        format!(
            "GPU compositor available: {} ({})",
            string(&preview["adapter"]["name"], "unknown adapter"),
            string(&preview["adapter"]["backend"], "unknown backend")
        )
    } else {
        format!("CPU compositor. {}", string(&preview["reason"], "No GPU compositor attached."))
    };
    let decode_text = if decoder["enabled"] == false {
        "Hardware decoding is Off. Already-open media retain their decoder until reopened.".to_owned()
    } else if decoder["snapshotAvailable"] == false {
        "Decoder registration snapshot is busy; try again.".to_owned()
    } else if let Some(backend) = decoder["backend"].as_str() {
        format!("Auto; {backend} registered. Individual streams may use software decoding.")
    } else {
        "Auto; no native hardware decoder backend registered. Browser decoding, if supplied by the host, is separate.".to_owned()
    };
    let encoder_text = if report["encoder"]["snapshotAvailable"] == false {
        "Diagnostics busy; try again.".to_owned()
    } else if latest.is_null() {
        "No export encoder selection recorded yet.".to_owned()
    } else {
        let mut text = format!(
            "{} — {} ({})",
            string(&latest["format"], "unknown format"),
            string(&latest["backend"], "unknown backend"),
            string(&latest["kind"], "unknown")
        );
        if let Some(why) = latest["fallbackReason"].as_str() {
            text.push_str(". ");
            text.push_str(why);
        }
        text
    };
    vec![
        ("preview", "Preview compositor", preview_text),
        ("decoder", "Video decoding", decode_text),
        (
            "decodeUsage",
            "Decoded frames (process lifetime)",
            format!(
                "Hardware: {}; software: {}; declined sessions: {}; mid-stream fallbacks: {}",
                count(&observed["hardwareFrames"]),
                count(&observed["softwareFrames"]),
                count(&observed["declined"]),
                count(&observed["fallbacks"])
            ),
        ),
        ("encoder", "Latest export encoder selection", encoder_text),
        (
            "exportRendering",
            "Export render counters",
            format!(
                "GPU: {}; CPU fallback: {}. {}",
                count(&report["rendering"]["gpuFrames"]),
                count(&report["rendering"]["cpuFallbackFrames"]),
                string(&report["rendering"]["scope"], "Process counters")
            ),
        ),
    ]
}

pub(crate) fn show(app: &mut FilmcraftApp, ctx: &egui::Context) -> bool {
    let report = snapshot(app);
    let mut open = true;
    let mut close = false;
    let mut elements = Vec::new();
    egui::Window::new(crate::i18n::t("Rendering Capabilities"))
        .id(egui::Id::new("rendering-capabilities"))
        .open(&mut open)
        .collapsible(false)
        .default_width(580.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 140.0).max(100.0)).show(ui, |ui| {
                for (id, label, value) in rows(&report) {
                    ui.label(egui::RichText::new(crate::i18n::t(label)).strong());
                    let r = ui.add(egui::Label::new(&value).wrap());
                    elements.push((format!("capabilities.{id}"), r.rect, value));
                    ui.add_space(8.0);
                }
                ui.separator();
                ui.label("GPU compositing, video decoding and video encoding are independent. GPU availability does not mean every effect runs on the GPU.");
                ui.label("The encoder entry records the latest initialization, not a currently running job. Counters can include multiple projects and exports.");
                ui.label("H.264 supports software encoding. HEVC and AV1 require a compatible registered encoder in this build; missing hardware does not imply a software encoder exists.");
                ui.label("To force software output: choose H.264, GPU Rendering Off, and Hardware Encoding Off. For software decoding too, set Playback → Hardware decoding to Off and reopen the media.");
                ui.small("Review copied diagnostics before sharing: driver messages may include system library paths.");
            });
            ui.separator();
            ui.horizontal(|ui| {
                let copy = ui.button(crate::i18n::t("Copy diagnostics"));
                elements.push(("capabilities.copy".into(), copy.rect, "Copy diagnostics".into()));
                if copy.clicked() {
                    // No project fields or media metadata. Driver messages may include library paths.
                    if let Ok(text) = serde_json::to_string_pretty(&report) {
                        ctx.copy_text(text);
                    }
                }
                let done = ui.button(tl!("Close"));
                elements.push(("capabilities.close".into(), done.rect, "Close".into()));
                close = done.clicked();
            });
        });
    for (id, rect, label) in elements {
        app.auto.add(&id, rect, &label);
    }
    ctx.request_repaint_after(std::time::Duration::from_secs(1));
    open && !close && !ctx.input(|i| i.key_pressed(egui::Key::Escape))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_activity_is_not_claimed_as_hardware_use() {
        let app = FilmcraftApp::new(filmcraft_engine::Session::default());
        let report = snapshot(&app);
        assert_eq!(report["preview"]["state"], "cpu");
        assert!(report["preview"]["reason"].as_str().is_some());
        let r = rows(&json!({"encoder": {"latest": null, "snapshotAvailable": true}}));
        assert!(r.iter().find(|r| r.0 == "encoder").unwrap().2.contains("No export"));
    }

    #[test]
    fn diagnostics_distinguish_policy_registration_and_observation() {
        let r = rows(&json!({
            "preview": {"state": "cpu", "reason": "device lost"},
            "decoder": {"enabled": false, "backend": "NVDEC", "observed": {"hardwareFrames": 50}},
            "encoder": {"snapshotAvailable": true, "latest": {"format": "h264", "backend": "FilmCraft", "kind": "software", "fallbackReason": "NVENC driver unavailable"}}
        }));
        assert!(r[0].2.contains("device lost"));
        assert!(r[1].2.contains("Off"));
        assert!(r[2].2.contains("Hardware: 50"));
        assert!(r[3].2.contains("NVENC driver unavailable"));
    }

    #[test]
    fn busy_snapshots_are_not_reported_as_absent_hardware() {
        let r = rows(&json!({"decoder": {"enabled": true, "snapshotAvailable": false}, "encoder": {"snapshotAvailable": false}}));
        assert!(r.iter().find(|r| r.0 == "decoder").unwrap().2.contains("busy"));
        assert!(r.iter().find(|r| r.0 == "encoder").unwrap().2.contains("busy"));
    }

    #[test]
    fn software_project_setting_explains_cpu_preview() {
        let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
        std::sync::Arc::make_mut(&mut app.session.project).settings.renderer = filmcraft_engine::project_tools::RENDERER_SOFTWARE.into();
        let report = snapshot(&app);
        assert_eq!(report["preview"]["state"], "cpu");
        assert!(report["preview"]["reason"].as_str().unwrap().contains("Software Only"));
    }
}
