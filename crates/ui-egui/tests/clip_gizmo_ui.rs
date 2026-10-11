//! Headless UI tests for the Program monitor clip transform gizmo:
//! bounding box, scale handles, move translation, and undo history integration (#639).

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
    fn demo() -> Self {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let b = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000);
        let harness = b.build_eframe(move |_cc| app);
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

    fn call(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                return v;
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params.clone());
        assert_eq!(r["ok"], true, "{method} {params}: {r}");
        r["result"].clone()
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn rect(&mut self, id: &str) -> [f64; 4] {
        let v = self.ok("ui.elements", json!({"prefix": id}));
        let e = v.as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("no element {id}: {v}")).clone();
        let r: Vec<f64> = e["rect"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        [r[0], r[1], r[2], r[3]]
    }

    fn center(&mut self, id: &str) -> (f64, f64) {
        let r = self.rect(id);
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    }

    fn drag(&mut self, id: &str, dx: f64, dy: f64) {
        let (x, y) = self.center(id);
        self.ok("ui.drag", json!({"from": {"x": x, "y": y}, "to": {"x": x + dx, "y": y + dy}, "steps": 6}));
        self.frames(2);
    }

    fn clip_motion_param(&mut self, clip_id: u64, param: &str) -> Value {
        let seq = self.exec("sequence.inspect", json!({}));
        for track in seq["video"].as_array().unwrap() {
            for it in track["items"].as_array().unwrap() {
                if it["clip"].as_u64() == Some(clip_id) {
                    if let Some(effects) = it.get("effects").and_then(Value::as_array) {
                        for eff in effects {
                            if eff["effect"] == "motion" {
                                return eff["params"][param]["value"].clone();
                            }
                        }
                    }
                }
            }
        }
        Value::Null
    }
}

#[test]
fn clip_gizmo_elements_and_move_drag() {
    let mut d = Driver::demo();
    d.ok("ui.set", json!({"workspace": "Editing"}));
    let seq = d.exec("sequence.inspect", json!({}));
    let clip = &seq["video"][0]["items"][0];
    let (id, start, dur) = (clip["clip"].as_u64().unwrap(), clip["start"].as_i64().unwrap(), clip["duration"].as_i64().unwrap());
    d.exec("playhead.set", json!({"time": start + dur / 2}));
    d.exec("timeline.select", json!({"clips": [id]}));
    d.frames(4);

    // Verify gizmo elements exist
    let box_id = format!("program.clip.{id}.box");
    let anchor_id = format!("program.clip.{id}.anchor");
    let handle2_id = format!("program.clip.{id}.handle.2"); // Bottom-right corner

    d.rect(&box_id);
    d.rect(&anchor_id);
    d.rect(&handle2_id);

    // Initial position
    let pos_before = d.clip_motion_param(id, "position");
    let undo0 = d.exec("history.list", json!({}))["undo"].as_array().unwrap().len();

    // Drag the bounding box (translation)
    d.drag(&box_id, 30.0, 20.0);
    d.frames(2);

    let pos_after = d.clip_motion_param(id, "position");
    assert_ne!(pos_before, pos_after, "clip position changed after drag");
    let undo1 = d.exec("history.list", json!({}))["undo"].as_array().unwrap().len();
    assert_eq!(undo1, undo0 + 1, "drag merged into a single undo step");

    // Scale drag (bottom-right handle)
    let scale_before = d.clip_motion_param(id, "scale");
    d.drag(&handle2_id, 40.0, 40.0);
    d.frames(2);

    let scale_after = d.clip_motion_param(id, "scale");
    assert_ne!(scale_before, scale_after, "clip scale changed after handle drag");
    let undo2 = d.exec("history.list", json!({}))["undo"].as_array().unwrap().len();
    assert_eq!(undo2, undo1 + 1, "scale drag merged into a single undo step");

    // Anchor point drag
    let anchor_before = d.clip_motion_param(id, "anchor");
    d.drag(&anchor_id, 25.0, 25.0);
    d.frames(2);

    let anchor_after = d.clip_motion_param(id, "anchor");
    assert_ne!(anchor_before, anchor_after, "clip anchor changed after drag");
    let undo3 = d.exec("history.list", json!({}))["undo"].as_array().unwrap().len();
    assert_eq!(undo3, undo2 + 1, "anchor drag merged into a single undo step");
}
