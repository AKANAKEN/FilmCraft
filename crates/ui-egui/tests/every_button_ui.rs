//! Every button, toggle, field and menu the app draws can be driven by id (AGENTS.md / CLAUDE.md:
//! "every interactive widget registers an automation id"). The real app runs headless under
//! `egui_kittest`, driven through the control channel as agents drive it over MCP. While driven, it
//! reads egui's AccessKit tree, so `ui.elements {discovered: true}` lists the widgets no panel
//! registered. The audit walks every workspace, panel, mode, menu dialog, panel menu and context
//! menu and fails on any of them, and on any id that names two widgets at once (a click by id would
//! reach only the first), naming each and where it was seen.
//!
//! The other tests cover what agents rely on: the app's `command.list` includes the frontend
//! commands, `edit.batch` runs UI commands too, `ui.click` opens the panel of an element that isn't
//! on screen, warns about a disabled element and reports the commands a click ran.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use filmcraft_ui_egui::dock::PanelKind;
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
        let v = self.call(method, params.clone());
        assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
        v["result"].clone()
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn elements(&mut self, params: Value) -> Vec<Value> {
        self.ok("ui.elements", params).as_array().cloned().unwrap_or_default()
    }
}

/// Widgets found while visiting: id → (what and where on screen, unused, where in the app).
type Missing = BTreeMap<String, (String, String, BTreeSet<String>)>;

fn rect(e: &Value) -> [f32; 4] {
    let r = |i: usize| e["rect"][i].as_f64().unwrap_or(0.0) as f32;
    [r(0), r(1), r(2), r(3)]
}

/// Where a widget is, for the report: its rect, the registered element it overlaps most and the
/// smallest registered one around it (usually enough to find the code that draws it).
fn whereabouts(e: &Value, registered: &[Value]) -> String {
    let [x, y, w, h] = rect(e);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let mut overlap: Option<(f32, &str)> = None;
    let mut inside: Option<(f32, &str)> = None;
    for r in registered {
        let [rx, ry, rw, rh] = rect(r);
        let id = r["id"].as_str().unwrap_or_default();
        let iw = (x + w).min(rx + rw) - x.max(rx);
        let ih = (y + h).min(ry + rh) - y.max(ry);
        if iw > 0.0 && ih > 0.0 {
            let iou = iw * ih / (w * h + rw * rh - iw * ih);
            if overlap.is_none_or(|(b, _)| iou > b) {
                overlap = Some((iou, id));
            }
        }
        if (rx..=rx + rw).contains(&cx) && (ry..=ry + rh).contains(&cy) && inside.is_none_or(|(a, _)| rw * rh < a) {
            inside = Some((rw * rh, id));
        }
    }
    let mut out = format!("at [{x:.0},{y:.0} {w:.0}x{h:.0}]");
    if let Some((iou, id)) = overlap {
        out += &format!(", overlaps {id} ({iou:.2})");
    }
    if let Some((_, id)) = inside {
        out += &format!(", inside {id}");
    }
    out
}

fn collect(d: &mut Driver, missing: &mut Missing, place: &str) {
    d.frames(3);
    let registered = d.elements(json!({"discovered": false}));
    // one id, two widgets: a click by id reaches only the first
    let mut rects: BTreeMap<&str, Vec<[f32; 4]>> = BTreeMap::new();
    for e in &registered {
        let r = rect(e);
        let seen = rects.entry(e["id"].as_str().unwrap_or_default()).or_default();
        if !seen.iter().any(|s| s.iter().zip(&r).all(|(a, b)| (a - b).abs() < 1.0)) {
            seen.push(r);
        }
    }
    for (id, rs) in rects.into_iter().filter(|(_, rs)| rs.len() > 1) {
        let where_ = rs.iter().map(|[x, y, w, h]| format!("[{x:.0},{y:.0} {w:.0}x{h:.0}]")).collect::<Vec<_>>().join(" ");
        let entry = missing.entry(format!("DUPLICATE {id}")).or_insert_with(|| (format!("{} widgets at {where_}", rs.len()), String::new(), BTreeSet::new()));
        entry.2.insert(place.to_string());
    }
    for e in d.elements(json!({"discovered": true})) {
        let id = e["id"].as_str().unwrap_or_default().to_string();
        let what = format!("{} \"{}\" {}", e["role"].as_str().unwrap_or_default(), e["label"].as_str().unwrap_or_default(), whereabouts(&e, &registered));
        let entry = missing.entry(id).or_insert_with(|| (what, String::new(), BTreeSet::new()));
        entry.2.insert(place.to_string());
    }
}

/// The first movie in a `project.inspect` tree.
fn first_movie(v: &Value) -> Option<u64> {
    if v["type"] == "Movie" {
        return v["item"].as_u64();
    }
    v["children"].as_array()?.iter().find_map(first_movie)
}

#[test]
fn every_widget_has_an_automation_id() {
    let mut d = Driver::demo();
    // give the clip panels something to show: a selected clip, a clip in the Source monitor
    let seq = d.exec("sequence.inspect", json!({}));
    if let Some(clip) = seq["video"][0]["items"][0]["clip"].as_u64() {
        d.exec("timeline.select", json!({"clips": [clip]}));
    }
    let mut missing = Missing::new();
    collect(&mut d, &mut missing, "start");
    let menu = d.ok("ui.menu.list", json!({}));
    let workspaces: Vec<String> = menu
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["id"].as_str())
        .filter(|id| id.starts_with("window.workspace.") && !id.ends_with(".reset") && !id.ends_with(".saveChanges"))
        .filter(|id| !id.ends_with(".saveAs") && !id.ends_with(".edit"))
        .map(str::to_string)
        .collect();
    assert!(workspaces.len() >= 6, "{workspaces:?}");
    for w in &workspaces {
        d.exec(w, json!({}));
        collect(&mut d, &mut missing, w);
    }
    for p in PanelKind::ALL {
        d.exec(&format!("window.panel.{}", p.id()), json!({}));
        collect(&mut d, &mut missing, &format!("panel {}", p.id()));
    }
    for m in ["mode.import", "mode.export", "mode.edit"] {
        d.exec(m, json!({}));
        collect(&mut d, &mut missing, m);
    }
    // a clip in the Source monitor, and trim mode in the Program monitor
    let project = d.exec("project.inspect", json!({}));
    if let Some(item) = first_movie(&project["root"]) {
        d.exec("source.open", json!({"item": item}));
        collect(&mut d, &mut missing, "source.open");
    }
    if d.call("engine.execute", json!({"command": "trim.selectNearest", "params": {}}))["ok"] == true {
        collect(&mut d, &mut missing, "trim mode");
        d.ok("ui.key", json!({"key": "Escape"}));
    }
    // context menus and panel menus (≡), opened by clicks on what is on screen now
    d.exec("window.workspace.allpanels", json!({}));
    d.frames(3);
    let mut menus: Vec<(String, &str)> = Vec::new();
    for e in d.elements(json!({"prefix": "panel.menu."})) {
        if let Some(id) = e["id"].as_str().filter(|id| id.matches('.').count() == 2) {
            menus.push((id.to_string(), "left"));
        }
    }
    for prefix in ["timeline.clip.", "timeline.transition.", "timeline.marker.", "project.item.", "effectControls.effect."] {
        if let Some(id) = d.elements(json!({"prefix": prefix})).iter().filter_map(|e| e["id"].as_str()).find(|id| id.matches('.').count() == 2) {
            menus.push((id.to_string(), "right"));
        }
    }
    for (id, button) in menus {
        if d.call("ui.click", json!({"id": id, "button": button}))["ok"] == true {
            collect(&mut d, &mut missing, &format!("{button} click on {id}"));
        }
        d.ok("ui.key", json!({"key": "Escape"}));
        d.frames(2);
    }
    // every dialog a menu item opens ("…" items run with no parameters, as from the menu)
    let dialogs: Vec<String> = menu
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["label"].as_str().is_some_and(|l| l.ends_with('…')) && m["enabled"] == json!(true))
        .filter_map(|m| m["id"].as_str().map(str::to_string))
        .filter(|id| !id.starts_with("help."))
        .collect();
    // each from a fresh app: dialogs keep their state in different places, and one left open would
    // be reported again under every later dialog
    for id in &dialogs {
        let mut d = Driver::demo();
        if let Some(clip) = seq["video"][0]["items"][0]["clip"].as_u64() {
            d.exec("timeline.select", json!({"clips": [clip]}));
        }
        let r = d.call("engine.execute", json!({"command": id, "params": {}}));
        if r["ok"] == json!(true) {
            collect(&mut d, &mut missing, id);
        }
    }
    let report: Vec<String> = missing
        .iter()
        .map(|(id, (what, _, at))| {
            let at: Vec<String> = at.iter().take(4).cloned().collect();
            format!("{id}: {what}; seen in {}", at.join(", "))
        })
        .collect();
    assert!(report.is_empty(), "{} widgets have no automation id (or share one):\n{}", report.len(), report.join("\n"));
}

/// The app's `command.list` is the engine's plus the frontend's own commands, each with its scope.
#[test]
fn the_app_lists_its_own_commands_too() {
    let mut d = Driver::demo();
    let list = d.exec("command.list", json!({}));
    let find = |id: &str| list.as_array().unwrap().iter().find(|c| c["id"] == id).cloned();
    for (id, scope) in
        [("timeline.razor", "engine"), ("tool.razor", "ui"), ("playback.toggle", "ui"), ("window.panel.Timeline", "ui"), ("window.workspace.color", "ui")]
    {
        let c = find(id).unwrap_or_else(|| panic!("{id} not listed"));
        assert_eq!(c["scope"], scope, "{c}");
    }
    let ids: Vec<&str> = list.as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
    let unique: BTreeSet<&str> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "every id once");
    assert_eq!(find("playback.toggle").unwrap()["params"], r#"{"monitor":"source"|"program"?}"#);
}

/// `edit.batch` in the app runs every step as `engine.execute` would, UI commands included, and is
/// one undo step; a failing step rolls the project back.
#[test]
fn a_batch_in_the_app_runs_ui_commands_and_is_one_undo_step() {
    let mut d = Driver::demo();
    let before = d.exec("sequence.inspect", json!({}));
    let steps = json!([
        {"id": "tool.razor"},
        {"id": "sequence.addEditAllTracks", "params": {"seconds": 1.0}},
        {"id": "sequence.addEditAllTracks", "params": {"seconds": 2.0}},
    ]);
    let r = d.exec("edit.batch", json!({"steps": steps, "label": "Agent cuts"}));
    assert_eq!((r["completed"].as_u64(), r["undo"].as_str()), (Some(3), Some("Agent cuts")), "{r}");
    assert_eq!(d.ok("ui.inspect", json!({}))["ui"]["tool"], "Razor");
    d.exec("edit.undo", json!({}));
    assert_eq!(d.exec("sequence.inspect", json!({}))["video"], before["video"]);
    let r = d.call(
        "engine.execute",
        json!({"command": "edit.batch", "params": {"steps": [{"id": "sequence.addEditAllTracks", "params": {"seconds": 1.0}}, {"id": "tool.nope"}]}}),
    );
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().unwrap().contains("step 2 (`tool.nope`)"), "{r}");
    assert_eq!(d.exec("sequence.inspect", json!({}))["video"], before["video"]);
}

/// A click reports the commands it ran, opens the panel of an element that isn't on screen, and
/// warns about a disabled element. Registered elements carry what AccessKit knows about them.
#[test]
fn clicks_report_what_they_ran_and_reach_hidden_panels() {
    let mut d = Driver::demo();
    let r = d.ok("ui.click", json!({"id": "timeline.track.V1.locked"}));
    let ran = r["ran"].as_array().cloned().unwrap_or_default();
    assert_eq!(ran.len(), 1, "{r}");
    assert_eq!(ran[0]["id"], "timeline.setTrack", "{r}");
    assert_eq!(ran[0]["params"]["locked"], true, "{r}");
    let r = d.ok("ui.click", json!({"id": "tools.Razor"}));
    assert_eq!(r["ran"][0]["id"], "tool.razor", "{r}");
    // the Audio Track Mixer is not in the Editing workspace's front tabs: a click on its element opens it
    d.exec("window.workspace.editing", json!({}));
    d.frames(3);
    let mixer: Vec<Value> = d.elements(json!({"prefix": "mixer.A1.mute"}));
    if mixer.is_empty() {
        let r = d.ok("ui.click", json!({"id": "mixer.A1.mute"}));
        assert!(r["ran"].as_array().is_some(), "{r}");
        assert!(!d.elements(json!({"prefix": "mixer.A1."})).is_empty(), "the mixer is on screen now");
    }
    // registered elements carry a role; a query finds them by label words
    let found = d.elements(json!({"query": "toggle track lock", "role": "clickable"}));
    assert!(found.iter().any(|e| e["id"] == "timeline.track.V1.locked"), "{found:?}");
    // a click on a disabled element runs nothing, and says so
    let disabled = d.elements(json!({})).into_iter().find(|e| e["enabled"] == false && !e["id"].as_str().unwrap_or("").starts_with("auto."));
    if let Some(e) = disabled {
        let r = d.ok("ui.click", json!({"id": e["id"]}));
        if r["ran"].as_array().is_some_and(Vec::is_empty) {
            assert!(r["warning"].as_str().unwrap_or("").contains("disabled"), "{r}");
        }
    }
}
