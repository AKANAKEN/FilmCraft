//! Properties must remain usable after resizing, scrolling, and entering large coordinates.
//! Optional GPU captures: FILMCRAFT_PROPERTIES_SNAPSHOT_DIR=<directory>.

use egui::{Event, Id, Key, Modifiers, PointerButton, Rect, pos2, vec2};
use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::ClipId;
use filmcraft_ui_egui::{
    FilmcraftApp,
    dock::{self, DockNode, PanelKind, SeqTabs, SplitSize},
    menus, panels,
    theme::{self, ThemeKind},
};
use serde_json::json;

fn session() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id;
    s.execute("timeline.select", json!({"clips": [clip.0]})).unwrap();
    s.execute("effects.setParam", json!({"clip": clip.0, "effect": "motion", "param": "position", "value": [-100000, 100000]})).unwrap();
    s.execute("effects.toggleAnimation", json!({"clip": clip.0, "effect": "motion", "param": "position"})).unwrap();
    let seq = s.state.active_sequence.unwrap();
    std::sync::Arc::make_mut(&mut s.project).sequence_mut(seq).unwrap().find_item_mut(clip).unwrap().1.name =
        "A long clip name — ทะเลทราย — dunes — a filename that must never cover its neighbours.mp4".into();
    s
}

fn selected(h: &Harness<'_, FilmcraftApp>) -> ClipId {
    *h.state().session.state.selection.first().unwrap()
}

fn frames(h: &mut Harness<'_, FilmcraftApp>, n: usize) {
    for _ in 0..n {
        let ctx = h.ctx.clone();
        let mut input = std::mem::take(h.input_mut());
        eframe::App::raw_input_hook(h.state_mut(), &ctx, &mut input);
        *h.input_mut() = input;
        h.step();
        assert!(h.state().ui_error.is_none(), "UI caught a panic: {:?}", h.state().ui_error);
    }
}

fn rect(h: &Harness<'_, FilmcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.elements.iter().find(|e| e.id == id).unwrap_or_else(|| panic!("missing {id}"));
    let [x, y, w, h] = e.rect;
    Rect::from_min_size(pos2(x, y), vec2(w, h))
}

fn click(h: &mut Harness<'_, FilmcraftApp>, id: &str) {
    let at = rect(h, id).center();
    h.event(Event::PointerMoved(at));
    frames(h, 1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        frames(h, 1);
    }
    frames(h, 4);
}

fn wheel(h: &mut Harness<'_, FilmcraftApp>, delta: egui::Vec2) {
    let at = rect(h, "panel.Properties").center();
    h.event(Event::PointerMoved(at));
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move });
    frames(h, 12);
}

fn inspector(width: f32, height: f32, kind: ThemeKind, scale: f32) -> Harness<'static, FilmcraftApp> {
    let mut builder = Harness::builder().with_size(vec2(width + 16.0, height + 16.0)).with_pixels_per_point(scale);
    if std::env::var_os("FILMCRAFT_PROPERTIES_SNAPSHOT_DIR").is_some() {
        builder = builder.wgpu();
    }
    let mut initialized = false;
    let mut h = builder.build_ui_state(
        move |ui, app: &mut FilmcraftApp| {
            app.set_theme(ui.ctx(), kind);
            if !initialized {
                theme::install(ui.ctx(), &app.tokens);
                initialized = true;
                return;
            }
            app.auto.begin_frame();
            let mut node = DockNode::Tabs { panels: vec![PanelKind::Properties, PanelKind::Text], active: 0 };
            let mut groups = Vec::new();
            dock::layout(ui, &mut node, ui.max_rect(), &app.tokens, "properties", &mut groups, &mut app.auto);
            for group in groups {
                dock::draw_group_chrome(ui, &group, PanelKind::Properties, &SeqTabs::default(), &app.tokens, &mut app.auto);
                app.auto.add("panel.Properties", group.content, "Properties");
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(group.content));
                child.set_clip_rect(group.content.intersect(ui.clip_rect()));
                panels::effect_controls::properties_panel(app, &mut child, group.content);
            }
        },
        FilmcraftApp::new(session()),
    );
    theme::install(&h.ctx, &h.state().tokens);
    frames(&mut h, 5);
    h
}

fn snapshot(h: &mut Harness<'_, FilmcraftApp>, name: &str) {
    let Some(directory) = std::env::var_os("FILMCRAFT_PROPERTIES_SNAPSHOT_DIR") else { return };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    h.render().unwrap().save(directory.join(format!("{name}.png"))).unwrap();
}

fn assert_controls_fit(h: &Harness<'_, FilmcraftApp>) {
    let viewport = rect(h, "panel.Properties");
    let controls: Vec<_> = h
        .state()
        .auto
        .elements
        .iter()
        .filter(|e| e.id.starts_with("properties.") && !e.id.starts_with("properties.section."))
        .map(|e| (e.id.as_str(), rect(h, &e.id)))
        .collect();
    assert!(controls.len() >= 20, "expected visible values and keyframe controls: {controls:?}");
    for (i, (id, r)) in controls.iter().enumerate() {
        assert!(r.min.is_finite() && r.max.is_finite() && r.is_positive() && viewport.contains_rect(*r), "{id}: {r:?}");
        for (other_id, other) in controls.iter().skip(i + 1) {
            assert!(!r.intersect(*other).is_positive(), "{id} overlaps {other_id}: {r:?} / {other:?}");
        }
    }
    let x = rect(h, "properties.motion.position.x");
    let y = rect(h, "properties.motion.position.y");
    let scale = rect(h, "properties.motion.scale.value");
    assert!(x.right() < y.left());
    assert!((x.left() - scale.left()).abs() < 0.5, "value columns must align");
}

#[test]
fn values_and_keyframes_fit_at_minimum_width_in_all_themes_and_scales() {
    for kind in [ThemeKind::Dark, ThemeKind::Medium, ThemeKind::Light] {
        for scale in [1.0, 2.0] {
            for width in [360.0, 720.0] {
                let mut h = inspector(width, 760.0, kind, scale);
                assert_controls_fit(&h);
                snapshot(&mut h, &format!("properties-{kind:?}-{width}-{scale}x"));
                click(&mut h, "properties.motion.position.x");
                assert_controls_fit(&h);
                if kind == ThemeKind::Dark && scale == 2.0 && width == 360.0 {
                    snapshot(&mut h, "properties-minimum-editing-coordinate");
                }
            }
        }
    }
}

fn float_value(h: &Harness<'_, FilmcraftApp>, effect: &str, param: &str) -> f64 {
    h.state().session.active_sequence().unwrap().find_item(selected(h)).unwrap().1.effect(effect).unwrap().param(param).unwrap().value.as_f64().unwrap()
}

#[test]
fn typing_and_reset_keep_undo_and_do_not_collapse_the_section() {
    let mut h = inspector(360.0, 760.0, ThemeKind::Dark, 1.0);
    let before = float_value(&h, "motion", "scale");
    click(&mut h, "properties.motion.scale.value");
    h.event(Event::Text("125".into()));
    frames(&mut h, 1);
    for pressed in [true, false] {
        h.event(Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
        frames(&mut h, 1);
    }
    frames(&mut h, 3);
    assert_eq!(float_value(&h, "motion", "scale"), 125.0);
    h.state_mut().session.undo();
    frames(&mut h, 3);
    assert_eq!(float_value(&h, "motion", "scale"), before);
    let clip = selected(&h);
    h.state_mut().session.execute("effects.setParam", json!({"clip":clip.0,"effect":"motion","param":"scale","value":150})).unwrap();
    frames(&mut h, 3);
    click(&mut h, "properties.section.Transform.reset");
    assert_eq!(float_value(&h, "motion", "scale"), before);
    assert!(!h.state().ui.collapsed_fx.contains(&"props:Transform".into()));
    assert!(h.state().auto.elements.iter().any(|e| e.id == "properties.motion.scale.value"));
}

#[test]
fn short_inspector_scrolls_to_audio_and_keyframes_use_the_scrolled_hit_target() {
    let mut h = inspector(360.0, 220.0, ThemeKind::Dark, 1.0);
    let project = h.state().session.project.clone();
    let selection = h.state().session.state.selection.clone();
    let playhead = h.state().session.playhead();
    assert!(!h.state().auto.elements.iter().any(|e| e.id == "properties.volume.level.value"));
    wheel(&mut h, vec2(0.0, -1600.0));
    let level = rect(&h, "properties.volume.level.value");
    assert!(rect(&h, "panel.Properties").contains_rect(level));
    assert!(std::sync::Arc::ptr_eq(&project, &h.state().session.project));
    assert_eq!(h.state().session.state.selection, selection);
    assert_eq!(h.state().session.playhead(), playhead);
    snapshot(&mut h, "properties-short-scrolled-audio");
    let target = rect(&h, "properties.volume.level.addKeyframe");
    click(&mut h, "properties.volume.level.addKeyframe");
    let seq = h.state().session.active_sequence().unwrap();
    assert!(
        seq.audio_tracks[0].items[0].effect("volume").unwrap().param("level").unwrap().is_animated(),
        "scrolled target={:?}, level={level:?}, status={}",
        target,
        h.state().ui.status
    );
}

#[test]
fn tiny_inspector_scrolls_horizontally_instead_of_overlapping_its_neighbour() {
    let mut h = inspector(90.0, 760.0, ThemeKind::Dark, 1.0);
    let menu = rect(&h, "panel.menu.Properties");
    let more = rect(&h, "panel.tabs.more.properties");
    assert!(!menu.intersect(more).is_positive(), "narrow header keeps its menu and tab overflow separate");
    assert!(menu.right() <= rect(&h, "panel.Properties").right());
    wheel(&mut h, vec2(-1600.0, 0.0));
    assert!(rect(&h, "panel.Properties").contains_rect(rect(&h, "properties.motion.position.addKeyframe")));
    snapshot(&mut h, "properties-tiny-scrolled-keyframes");
    click(&mut h, "properties.motion.position.addKeyframe");
}

#[test]
fn undersized_host_contains_video_and_graphic_properties_in_their_panel() {
    for graphic in [false, true] {
        let mut s = session();
        if graphic {
            s.execute("graphics.newText", json!({"text": "Original test title"})).unwrap();
        }
        let mut h = Harness::builder().with_size(vec2(300.0, 900.0)).build_eframe(move |_| FilmcraftApp::new(s));
        frames(&mut h, 4);
        h.state_mut().ui.dock = split(tabs(&[PanelKind::Properties]), tabs(&[PanelKind::Timeline]), SplitSize::Ratio(0.5), false);
        frames(&mut h, 5);
        let viewport = rect(&h, "panel.Properties");
        assert!(viewport.width() < 360.0, "test must exercise an impossible combined minimum");
        let project = h.state().session.project.clone();
        let mut found_label = false;
        for clipped in &h.output().shapes {
            if let egui::Shape::Text(text) = &clipped.shape {
                let label = text.galley.text();
                if (graphic && label == "Layers") || (!graphic && label == "Transform") {
                    found_label = true;
                    assert!(clipped.clip_rect.right() <= viewport.right(), "{label} can paint over a neighbouring panel: {:?}", clipped.clip_rect);
                }
            }
        }
        assert!(found_label, "the production inspector branch must be drawn");
        wheel(&mut h, vec2(-1000.0, 0.0));
        assert!(std::sync::Arc::ptr_eq(&project, &h.state().session.project));
        if !graphic {
            let navigator = rect(&h, "properties.motion.position.addKeyframe");
            assert!(viewport.contains_rect(navigator));
            click(&mut h, "properties.motion.position.addKeyframe");
        }
    }
}

fn tabs(panels: &[PanelKind]) -> DockNode {
    DockNode::Tabs { panels: panels.to_vec(), active: 0 }
}

fn split(a: DockNode, b: DockNode, size: SplitSize, vertical: bool) -> DockNode {
    DockNode::Split { a: Box::new(a), b: Box::new(b), size, vertical }
}

#[test]
fn saved_narrow_and_nested_groups_keep_properties_minimum_without_rewriting_layout() {
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_eframe(move |_| FilmcraftApp::new(session()));
    frames(&mut h, 4);
    for properties_first in [true, false] {
        for mode in [SplitSize::FixedA(40.0), SplitSize::FixedB(40.0), SplitSize::Ratio(0.02), SplitSize::Ratio(0.98)] {
            let properties = tabs(&[PanelKind::Text, PanelKind::Properties]);
            // The inactive Properties tab still imposes its minimum on this vertical subtree.
            let subtree = split(properties, tabs(&[PanelKind::Info]), SplitSize::Ratio(0.5), true);
            let layout = if properties_first {
                split(subtree, tabs(&[PanelKind::Timeline]), mode, false)
            } else {
                split(tabs(&[PanelKind::Timeline]), subtree, mode, false)
            };
            h.state_mut().ui.dock = layout.clone();
            frames(&mut h, 3);
            assert!(rect(&h, "panel.Text").width() >= 360.0);
            assert_eq!(h.state().ui.dock, layout, "rendering must not rewrite a saved workspace");
        }
    }
    h.state_mut().ui.dock = split(
        split(tabs(&[PanelKind::Info]), tabs(&[PanelKind::Properties]), SplitSize::Ratio(0.9), false),
        tabs(&[PanelKind::Timeline]),
        SplitSize::FixedA(40.0),
        false,
    );
    frames(&mut h, 3);
    assert!(rect(&h, "panel.Properties").width() >= 360.0);
}

#[test]
fn pointer_and_keyboard_resize_stop_at_properties_minimum_and_preserve_sizing_mode() {
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_eframe(move |_| FilmcraftApp::new(session()));
    frames(&mut h, 4);
    for first in [true, false] {
        for mode in [SplitSize::FixedA(600.0), SplitSize::FixedB(600.0), SplitSize::Ratio(0.5)] {
            h.state_mut().ui.dock = if first {
                split(tabs(&[PanelKind::Properties]), tabs(&[PanelKind::Timeline]), mode, false)
            } else {
                split(tabs(&[PanelKind::Timeline]), tabs(&[PanelKind::Properties]), mode, false)
            };
            frames(&mut h, 3);
            let project = h.state().session.project.clone();
            let playhead = h.state().session.playhead();
            let from = rect(&h, "dock.gutter.").center();
            let to = from + vec2(if first { -800.0 } else { 800.0 }, 0.0);
            h.event(Event::PointerMoved(from));
            frames(&mut h, 1);
            h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            frames(&mut h, 1);
            for fraction in [0.1, 0.3, 0.6, 1.0] {
                h.event(Event::PointerMoved(from.lerp(to, fraction)));
                frames(&mut h, 1);
            }
            h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            frames(&mut h, 3);
            assert_eq!(rect(&h, "panel.Properties").width(), 360.0);
            h.ctx.memory_mut(|m| m.request_focus(Id::new(("dock-gutter", String::new()))));
            let key = if first { Key::ArrowLeft } else { Key::ArrowRight };
            for pressed in [true, false] {
                h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
                frames(&mut h, 1);
            }
            assert_eq!(rect(&h, "panel.Properties").width(), 360.0);
            let DockNode::Split { size, .. } = h.state().ui.dock else { panic!("split was lost") };
            assert_eq!(std::mem::discriminant(&size), std::mem::discriminant(&mode));
            assert!(std::sync::Arc::ptr_eq(&project, &h.state().session.project));
            assert_eq!(h.state().session.playhead(), playhead);
        }
    }
}

#[test]
fn floating_properties_uses_the_same_minimum_for_legacy_sizes_and_resize() {
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_eframe(move |_| FilmcraftApp::new(session()));
    frames(&mut h, 4);
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "window.floatPanel", json!({"panel":"Properties","rect":[100,100,80,400]})).unwrap();
    frames(&mut h, 4);
    assert_eq!(rect(&h, "panel.Properties").width(), 360.0);
    assert_eq!(h.state().ui.floating[0].rect[2], 80.0, "rendering leaves legacy saved sizes intact");
    let at = rect(&h, "panel.float.0.resize").center();
    h.event(Event::PointerMoved(at));
    frames(&mut h, 1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    frames(&mut h, 1);
    for offset in [-10.0, -100.0, -250.0] {
        h.event(Event::PointerMoved(at + vec2(offset, 0.0)));
        frames(&mut h, 1);
    }
    h.event(Event::PointerButton { pos: at + vec2(-250.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    frames(&mut h, 3);
    assert_eq!(rect(&h, "panel.Properties").width(), 360.0);
}
