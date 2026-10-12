//! Headless UI test of the Timeline marker menu (#1143): right-clicking a marker on the ruler
//! offers a Label submenu that sets the marker's colour through `markers.edit`. The clip menu's
//! `edit.label` silently did nothing to markers; before the fix the marker menu had no Label entry
//! at all.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new() -> Self {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(s).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
        d
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({ "command": command, "params": params }))
    }

    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({ "id": id }));
        self.frames(3);
    }

    fn right_click(&mut self, id: &str) {
        self.ok("ui.click", json!({ "id": id, "button": "right" }));
        self.frames(3);
    }

    fn has(&mut self, id: &str) -> bool {
        let v = self.ok("ui.elements", json!({ "prefix": id }));
        v.as_array().unwrap().iter().any(|e| e["id"] == json!(id))
    }

    fn marker_color(&mut self, marker: u64) -> String {
        let q = self.harness.state_mut().session.active_sequence().unwrap();
        q.markers.iter().find(|m| m.id.0 == marker).expect("marker exists").color.name().to_string()
    }
}

#[test]
fn marker_menu_label_sets_the_marker_colour() {
    let mut d = Driver::new();
    // A range marker at frame 24 (the demo sequence is open).
    let r = d.exec("markers.addRange", json!({ "frame": 24, "durationFrames": 12, "name": "triage" }));
    let id = r["marker"].as_u64().expect("marker id");
    d.frames(3);

    // Right-click the marker pentagon on the ruler: the marker menu opens with a Label submenu.
    d.right_click(&format!("timeline.marker.{id}"));
    assert!(d.has("timeline.markerMenu.markers.clearCurrent"), "marker menu is open");
    // The Label submenu renders its colours only once opened.
    assert!(d.has("timeline.markerMenu.Label"), "the marker menu offers a Label submenu");
    d.click("timeline.markerMenu.Label");
    assert!(d.has("timeline.markerMenu.label.Green"), "Label submenu lists colours");

    // Choosing a colour edits THIS marker (markers.edit), not the empty clip selection (edit.label).
    let before = d.marker_color(id);
    let pick = ["Green", "Yellow", "Blue"].iter().map(|l| l.to_string()).find(|l| l != &before).expect("a colour differing from the default");
    d.click(&format!("timeline.markerMenu.label.{pick}"));
    assert_eq!(d.marker_color(id), pick, "the marker's colour follows the menu choice");
}
