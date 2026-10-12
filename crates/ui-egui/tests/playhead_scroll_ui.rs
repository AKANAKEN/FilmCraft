//! Headless UI test of #1188: while paused, a playhead that jumps outside the visible part of the
//! Timeline (Home / End, a go-to-edit command, a script) moves the view to it, as in Premiere Pro.
//! A playhead that did not move never moves the view, so a scroll the user set stays put.

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
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
        d
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            // the control channel's clicks and keys enter through the input hook
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

    fn app(&mut self) -> &mut FilmcraftApp {
        self.harness.state_mut()
    }

    fn seconds(&mut self) -> f64 {
        self.app().session.active_sequence().unwrap().duration().seconds()
    }

    fn playhead_seconds(&mut self) -> f64 {
        self.app().session.playhead().seconds()
    }

    /// Points wide: the Timeline's content area (the ruler / tracks, right of the track header).
    fn width(&mut self) -> f64 {
        self.app().last_timeline_width as f64
    }

    /// Zoom in so the sequence is several times wider than the view, and park the view at its head.
    fn zoomed_in(&mut self) {
        let dur = self.seconds();
        let width = self.width();
        assert!(width > 50.0, "the Timeline is drawn: {width} pt wide");
        // 4x the zoom that would fit the whole sequence in the view
        let pps = (4.0 * width / dur).clamp(0.05, 24_000.0);
        self.ok("ui.set", json!({ "timeline": { "pps": pps, "scroll": 0.0 } }));
        self.settle();
        let (scroll, pps) = self.settle();
        assert_eq!(scroll, 0.0, "the view starts at the head of the sequence");
        assert!(dur * pps > 2.0 * width, "the sequence is wider than the view at {pps} px/s");
    }

    /// (scroll, pps) once the zoom / scroll animation has settled.
    fn settle(&mut self) -> (f64, f64) {
        for _ in 0..400 {
            let v = &self.app().ui.timeline;
            if v.pps == v.target_pps && v.scroll == v.target_scroll {
                break;
            }
            self.frames(1);
        }
        let v = &self.app().ui.timeline;
        (v.scroll, v.pps)
    }

    /// The playhead's x relative to the left edge of the Timeline content, in points.
    fn playhead_px(&mut self) -> f64 {
        let (scroll, pps) = {
            let v = &self.app().ui.timeline;
            (v.scroll, v.pps)
        };
        (self.playhead_seconds() - scroll) * pps
    }
}

/// End (playhead.end) jumps the playhead past the right edge of the view; the view follows so the
/// playhead is at the right edge. Home (playhead.start) takes both back to the head.
#[test]
fn end_and_home_move_the_paused_view_to_the_playhead() {
    let mut d = Driver::demo();
    assert!(!d.app().playback.playing, "paused");
    d.zoomed_in();
    let dur = d.seconds();
    let width = d.width();

    // End: the playhead goes to the end of the sequence and the view moves so it is visible
    d.exec("playhead.end", json!({}));
    let (scroll, pps) = d.settle();
    assert!((d.playhead_seconds() - dur).abs() < 0.1, "the playhead is at the end");
    assert!(scroll > 0.0, "the view moved: scroll {scroll}");
    // The view cannot scroll past the Timeline's extent (the sequence plus ten minutes), so at the
    // end of the sequence either the playhead lands on the right edge or the view has run out of
    // scroll and the playhead sits inside it. Either way it is on screen.
    let px = d.playhead_px();
    let max_scroll = (dur + 600.0) - width / pps;
    assert!(px > 0.0 && px <= width + 1.0, "the playhead is inside the view: {px} px of {width}");
    if (scroll - max_scroll).abs() < 0.01 {
        assert!(scroll > 0.0, "the view scrolled to its limit: {scroll}");
    } else {
        assert!((px - width).abs() < 2.0, "the playhead sits at the right edge: {px} of {width}");
    }

    // Home: the playhead goes back to zero and the view goes with it
    d.exec("playhead.start", json!({}));
    let (scroll, _) = d.settle();
    assert_eq!(d.playhead_seconds(), 0.0);
    assert_eq!(scroll, 0.0, "the view is back at the head of the sequence");
}

/// Stepping frames past the right edge keeps following the playhead while paused: the rule is the
/// same for a step as for a jump — only a playhead that moved moves the view.
#[test]
fn stepping_past_the_right_edge_keeps_the_playhead_visible() {
    let mut d = Driver::demo();
    d.zoomed_in();
    d.exec("playhead.start", json!({}));
    let (_, pps) = d.settle();
    let width = d.width();
    // step until the playhead's time is past what the view can show from its head
    let view_s = width / pps;
    let mut steps = 0;
    while d.playhead_seconds() < view_s * 1.5 && steps < 5_000 {
        d.exec("playhead.stepForward", json!({}));
        d.frames(2);
        steps += 1;
    }
    assert!(steps > 1, "it took several frames to cross the view: {steps}");
    assert!(d.playhead_seconds() > view_s, "the playhead went past the right edge of the view");
    let px = d.playhead_px();
    assert!(px > 0.0 && px <= width + 1.0, "the playhead stayed in view: {px} px of {width}");
    assert!(d.app().ui.timeline.scroll > 0.0, "the view scrolled: {}", d.app().ui.timeline.scroll);
}

/// A scroll the user set while the playhead sits still is not moved by the next frame: the fix
/// reacts to playhead changes, not to playhead position.
#[test]
fn a_still_playhead_leaves_the_users_scroll_alone() {
    let mut d = Driver::demo();
    d.zoomed_in();
    d.exec("playhead.end", json!({}));
    d.settle();
    let moved = d.app().ui.timeline.scroll;
    assert!(moved > 0.0, "the view followed the playhead: {moved}");
    // the user scrolls back to the head of the sequence; the playhead does not move
    d.ok("ui.set", json!({ "timeline": { "scroll": 0.0 } }));
    let (scroll, _) = d.settle();
    assert_eq!(scroll, 0.0, "the view stays where the user put it");
    d.frames(20);
    assert_eq!(d.app().ui.timeline.scroll, 0.0, "and stays there while the playhead is still");
}
