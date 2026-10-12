//! Read-only capabilities dialog, driven through the same commands and widget ids as the desktop.
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::{Dialog, FilmcraftApp, control::ControlRequest};
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new() -> Self {
        let (tx, rx) = channel();
        let mut app = FilmcraftApp::new(Session::default()).with_control(rx);
        app.gpu_fallback_reason = Some("Synthetic device loss; preview uses CPU".into());
        let mut builder = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).with_max_steps(10_000);
        if std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR").is_some() {
            builder = builder.wgpu();
        }
        let harness = builder.build_eframe(move |_| app);
        Self { harness, tx }
    }

    fn step(&mut self) {
        let ctx = self.harness.ctx.clone();
        let mut input = std::mem::take(self.harness.input_mut());
        eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut input);
        *self.harness.input_mut() = input;
        self.harness.step();
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params);
        self.tx.send(req).unwrap();
        for _ in 0..100 {
            self.step();
            if let Ok(value) = reply.try_recv() {
                assert_eq!(value["ok"], true, "{value}");
                return value["result"].clone();
            }
        }
        panic!("no reply to {method}");
    }
}

#[test]
fn capabilities_menu_copy_close_and_live_reason_are_read_only() {
    let mut d = Driver::new();
    for _ in 0..4 {
        d.step();
    }
    let revision = d.harness.state().session.revision;
    let undo = d.harness.state().session.history.undo.len();
    let menu = d.call("ui.menu.list", json!({}));
    assert!(menu.as_array().unwrap().iter().any(|m| m["id"] == "help.renderingCapabilities" && m["path"] == json!(["Help"])));
    let report = d.call("ui.menu.invoke", json!({"id": "help.renderingCapabilities"}));
    assert_eq!(report["preview"]["state"], "cpu");
    assert!(report["preview"]["reason"].as_str().unwrap().contains("Synthetic device loss"));
    for _ in 0..3 {
        d.step();
    }
    assert_eq!(d.harness.state().dialog, Some(Dialog::RenderingCapabilities));
    let elements = d.call("ui.elements", json!({"prefix": "capabilities."}));
    for id in ["preview", "decoder", "decodeUsage", "encoder", "exportRendering", "copy", "close"] {
        assert!(elements.as_array().unwrap().iter().any(|e| e["id"] == format!("capabilities.{id}")), "missing {id}: {elements}");
    }
    d.call("ui.click", json!({"id": "capabilities.copy"}));
    let report = d.call("perf.stats", json!({}));
    assert!(report["capabilities"]["preview"]["reason"].as_str().unwrap().contains("Synthetic device loss"));
    d.harness.state_mut().gpu_fallback_reason = Some("Updated diagnostic".into());
    for _ in 0..3 {
        d.step();
    }
    let elements = d.call("ui.elements", json!({"prefix": "capabilities.preview"}));
    assert!(elements.to_string().contains("Updated diagnostic"), "{elements}");
    if let Some(dir) = std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        d.harness
            .render()
            .expect("offscreen GPU rendering must succeed when requested")
            .save(std::path::Path::new(&dir).join("rendering-capabilities.png"))
            .unwrap();
    }
    d.call("ui.click", json!({"id": "capabilities.close"}));
    for _ in 0..2 {
        d.step();
    }
    assert_eq!(d.harness.state().dialog, None);
    assert_eq!(d.harness.state().session.revision, revision);
    assert_eq!(d.harness.state().session.history.undo.len(), undo);
}
