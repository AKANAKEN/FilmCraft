//! Held timeline feedback under resolved System Light and Dark appearances.
//!
//! These tests inspect geometry and colors emitted by the real app painters. Optional
//! FILMCRAFT_UI_SNAPSHOT_DIR snapshots render the same held frame through wgpu.
//! Injected egui appearance/file input proves the input-to-painter path, not native OS delivery.

use std::path::PathBuf;

use egui::{Color32, Event, Modifiers, PointerButton, Pos2, Rect, Shape, pos2, vec2};
use egui_kittest::Harness;
use filmcraft_engine::{Session, clip_ops::source_view};
use filmcraft_project::{BinEntry, ItemId, TrackKind};
use filmcraft_time::Tick;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::panels::timeline::{Drag, Layout, Row};
use filmcraft_ui_egui::theme::ThemeKind;
use serde_json::{Value, json};

const MEDIA: ItemId = ItemId(5); // The generated demo's video/audio item, also used by source_drag_ui.

struct Driver {
    h: Harness<'static, FilmcraftApp>,
    system: egui::Theme,
    shots: Option<PathBuf>,
}

impl Driver {
    fn new(system: egui::Theme) -> Self {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        s.execute("file.newSequence", json!({"name":"Feedback test","fps":24,"video":1,"audio":1})).unwrap();
        s.execute("source.open", json!({"item":MEDIA.0})).unwrap();
        s.execute("project.view.set", json!({"view":"list"})).unwrap();
        s.execute("prefs.set", json!({"key":"appearance.darkTheme","value":"darkest"})).unwrap();
        s.execute("prefs.set", json!({"key":"appearance.appearanceMode","value":"auto"})).unwrap();
        s.execute("sequence.snap", json!({"on":false})).unwrap();
        let mut app = FilmcraftApp::new(s);
        app.ui.expanded_bins = app
            .session
            .project
            .root
            .children
            .iter()
            .filter_map(|entry| match entry {
                BinEntry::Bin(bin) => Some(bin.id.0),
                _ => None,
            })
            .collect();
        let shots = std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR").map(PathBuf::from);
        let mut builder = Harness::builder().with_size(vec2(1600.0, 980.0)).with_step_dt(1.0 / 60.0).with_max_steps(10_000);
        if shots.is_some() {
            builder = builder.wgpu().with_pixels_per_point(1.0);
        }
        let h = builder.build_eframe(move |_cc| app);
        let mut d = Self { h, system, shots };
        d.frames(4);
        d.fix_zoom();
        assert_eq!(d.h.state().session.prefs.appearance.appearance_mode, "auto");
        assert_eq!(d.h.state().tokens.kind, if system == egui::Theme::Light { ThemeKind::Light } else { ThemeKind::Dark });
        d
    }

    fn step(&mut self) {
        let ctx = self.h.ctx.clone();
        let mut raw = std::mem::take(self.h.input_mut());
        eframe::App::raw_input_hook(self.h.state_mut(), &ctx, &mut raw);
        raw.system_theme = Some(self.system);
        *self.h.input_mut() = raw;
        ctx.request_repaint();
        self.h.step();
        assert!(self.h.state().ui_error.is_none(), "{:?}", self.h.state().ui_error);
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }

    fn fix_zoom(&mut self) {
        let view = &mut self.h.state_mut().ui.timeline;
        view.pps = 30.0;
        view.target_pps = 30.0;
        view.scroll = 0.0;
        view.target_scroll = 0.0;
        view.fit_pending = false;
        view.fit_empty = None;
        self.step();
    }

    fn rect(&self, id: &str) -> Rect {
        let e = self.h.state().auto.elements.iter().find(|e| e.id == id).unwrap_or_else(|| panic!("missing automation element {id}"));
        let [x, y, w, h] = e.rect;
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    fn layout(&self) -> &Layout {
        self.h.state().tl.layout.as_ref().unwrap()
    }

    fn video_row(&self) -> Row {
        self.layout().rows.iter().find(|r| r.kind == TrackKind::Video && r.index == 0).unwrap().clone()
    }

    fn target(&self) -> (Pos2, Tick) {
        let tick = self.h.state().session.sequence_rate().tick_of(120); // Five seconds, well away from the playhead.
        let row = self.video_row();
        let pos = pos2(self.layout().x_of(tick), row.rect.center().y);
        assert!(self.layout().content.contains(pos));
        (pos, tick)
    }

    fn pointer(&mut self, pos: Pos2, pressed: Option<bool>, modifiers: Modifiers) {
        self.h.input_mut().events.push(Event::ModifiersChanged(modifiers));
        self.h.input_mut().events.push(Event::PointerMoved(pos));
        if let Some(pressed) = pressed {
            self.h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers });
        }
        self.step();
    }

    fn hold(&mut self, from: Pos2, to: Pos2, modifiers: Modifiers) {
        self.pointer(from, None, modifiers); // Establish hover before the press.
        self.pointer(from, Some(true), modifiers);
        self.pointer(from + vec2(0.0, 10.0), None, modifiers); // Cross the real drag threshold.
        self.pointer(to, None, modifiers);
        self.step(); // Snap paint consumes the previous interaction frame's snap_x.
        assert!(self.h.ctx.input(|i| i.pointer.primary_down()), "the observed preview must be held");
    }

    fn release(&mut self, at: Pos2, modifiers: Modifiers) {
        self.pointer(at, Some(false), modifiers);
        self.frames(2);
    }

    fn project(&self) -> Value {
        serde_json::to_value(self.h.state().session.project.as_ref()).unwrap()
    }

    fn assert_unchanged(&self, project: &Value, prefs: &Value) {
        assert_eq!(&self.project(), project, "held feedback cannot edit the project");
        assert_eq!(&self.h.state().session.prefs.to_value(), prefs, "feedback cannot replace System or other preferences");
    }

    fn shapes(&self) -> Vec<&Shape> {
        fn append<'a>(shape: &'a Shape, out: &mut Vec<&'a Shape>) {
            if let Shape::Vec(children) = shape {
                for child in children {
                    append(child, out);
                }
            } else {
                out.push(shape);
            }
        }
        let mut shapes = Vec::new();
        for clipped in &self.h.output().shapes {
            append(&clipped.shape, &mut shapes);
        }
        shapes
    }

    fn row_background(&self) -> Color32 {
        let row = self.video_row().rect;
        self.shapes()
            .into_iter()
            .find_map(|shape| match shape {
                Shape::Rect(r) if same_rect(r.rect, row) && r.fill.a() == 255 && r.stroke.width == 0.0 => Some(r.fill),
                _ => None,
            })
            .expect("the neutral row itself must be painted")
    }

    fn assert_ink(&self, ink: Color32, label: &str) {
        assert_eq!(ink, self.h.state().tokens.icon_active, "{label} must use the resolved neutral feedback ink");
        assert!(contrast(ink, self.row_background()) >= 3.0, "{label}: ink {ink:?} has insufficient contrast against the painted row");
    }

    fn assert_rect_feedback(&self, expected: Rect, width: f32, fill_alpha: Option<u8>, stroke_alpha: u8) {
        let shapes = self.shapes();
        let border = shapes
            .iter()
            .find_map(|shape| match shape {
                Shape::Rect(r) if same_rect(r.rect, expected) && (r.stroke.width - width).abs() < 0.01 => Some(r.stroke.color),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing held preview border at {expected:?}"));
        let ink = self.h.state().tokens.icon_active;
        assert_eq!(border, ink.gamma_multiply(f32::from(stroke_alpha) / 255.0), "held border at {expected:?}");
        if stroke_alpha == 255 {
            self.assert_ink(border, "drop outline");
        } else {
            assert!(contrast(over(border, self.row_background()), self.row_background()) >= 1.5, "marquee border must remain visible");
        }
        if let Some(alpha) = fill_alpha {
            let fill = shapes
                .iter()
                .find_map(|shape| match shape {
                    Shape::Rect(r) if same_rect(r.rect, expected) && r.fill.a() > 0 => Some(r.fill),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing held preview fill at {expected:?}"));
            assert_eq!(fill, ink.gamma_multiply(f32::from(alpha) / 255.0), "held fill at {expected:?}");
        }
    }

    fn assert_label(&self, expected: &str, anchor: Pos2, above: bool) {
        let color = self
            .shapes()
            .into_iter()
            .find_map(|shape| match shape {
                Shape::Text(text)
                    if text.galley.job.text == expected
                        && (text.pos.x - anchor.x).abs() < 1.0
                        && if above { text.pos.y < anchor.y && text.pos.y > anchor.y - 20.0 } else { (text.pos.y - anchor.y).abs() < 1.0 } =>
                {
                    let section = text.galley.job.sections.first().expect("label layout has a section");
                    Some(text.override_text_color.unwrap_or(if section.format.color == Color32::PLACEHOLDER {
                        text.fallback_color
                    } else {
                        section.format.color
                    }))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing feedback label {expected:?} at {anchor:?}"));
        self.assert_ink(color, expected);
    }

    fn drop_rect(&self, start: Tick, duration: Tick) -> Rect {
        let row = self.video_row();
        Rect::from_min_max(pos2(self.layout().x_of(start), row.rect.min.y + 1.0), pos2(self.layout().x_of(start + duration), row.rect.max.y - 1.0))
    }

    fn assert_snap(&self, target: Tick) {
        let layout = self.layout();
        let x = layout.x_of(target);
        let a = pos2(x, layout.ruler.max.y);
        let b = pos2(x, layout.content.max.y);
        let shapes = self.shapes();
        // The snapped playhead and snap indicator share geometry. Paint order puts the
        // preserved blue playhead first and the neutral snap line over it.
        let lines: Vec<_> = shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::LineSegment { points, stroke } if near(points[0], a) && near(points[1], b) && (stroke.width - 1.0).abs() < 0.01 => Some(stroke.color),
                _ => None,
            })
            .collect();
        assert_eq!(lines.len(), 2, "both the playhead and actual snap line must be emitted at the target");
        assert_eq!(lines[0], self.h.state().tokens.playhead, "the semantic playhead color stays unchanged");
        self.assert_ink(lines[1], "snap line");
        for (y, direction) in [(a.y, 1.0), (b.y, -1.0)] {
            let points = [pos2(x - 4.0, y), pos2(x + 4.0, y), pos2(x, y + 5.0 * direction)];
            let fill = shapes
                .iter()
                .find_map(|shape| match shape {
                    Shape::Path(path) if path.points.len() == 3 && path.points.iter().zip(points).all(|(actual, expected)| near(*actual, expected)) => {
                        Some(path.fill)
                    }
                    _ => None,
                })
                .expect("actual snap arrow at the end of the snap line");
            self.assert_ink(fill, "snap arrow");
        }
    }

    fn shot(&mut self, name: &str) {
        let Some(dir) = self.shots.clone() else { return };
        std::fs::create_dir_all(&dir).unwrap();
        // Render this exact held output. Advancing extra frames could hide or release feedback.
        let suffix = if self.system == egui::Theme::Light { "light" } else { "dark" };
        self.h.render().unwrap().save(dir.join(format!("timeline-feedback-{name}-{suffix}.png"))).unwrap();
    }
}

fn near(a: Pos2, b: Pos2) -> bool {
    a.distance(b) < 1.0
}

fn same_rect(a: Rect, b: Rect) -> bool {
    near(a.min, b.min) && near(a.max, b.max)
}

fn luminance(color: Color32) -> f64 {
    let linear = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

fn contrast(a: Color32, b: Color32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn over(foreground: Color32, background: Color32) -> Color32 {
    let alpha = f64::from(foreground.a()) / 255.0;
    let channel = |fg: u8, bg: u8| (f64::from(fg) + f64::from(bg) * (1.0 - alpha)).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(channel(foreground.r(), background.r()), channel(foreground.g(), background.g()), channel(foreground.b(), background.b()))
}

fn insert_modifier() -> Modifiers {
    Modifiers { command: true, ctrl: !cfg!(target_os = "macos"), mac_cmd: cfg!(target_os = "macos"), ..Default::default() }
}

#[test]
fn system_light_and_dark_paint_actual_held_snap_line_and_arrows() {
    for system in [egui::Theme::Light, egui::Theme::Dark] {
        let mut d = Driver::new(system);
        let (_, marker) = d.target();
        d.h.state_mut().session.execute("markers.add", json!({"time":marker.0,"name":"Snap target","target":"program"})).unwrap();
        d.h.state_mut().session.execute("sequence.snap", json!({"on":true})).unwrap();
        d.step();
        let before = d.project();
        let prefs = d.h.state().session.prefs.to_value();
        let ruler = d.rect("timeline.ruler");
        let x = d.layout().x_of(marker);
        let y = ruler.center().y;
        let to = pos2(x + 3.0, y);
        d.hold(pos2(x + 50.0, y), to, Modifiers::NONE);
        assert_eq!(d.h.state().session.playhead(), marker, "the live gesture must snap to the marker");
        d.assert_snap(marker);
        d.assert_unchanged(&before, &prefs);
        d.shot("snap-held");
        d.release(to, Modifiers::NONE);
        assert_eq!(d.h.state().session.playhead(), marker);
        d.assert_unchanged(&before, &prefs);
    }
}

#[test]
fn system_light_and_dark_paint_held_project_and_source_insert_previews() {
    for system in [egui::Theme::Light, egui::Theme::Dark] {
        for source in [false, true] {
            let mut d = Driver::new(system);
            let (duration, source_in) = if source {
                let rate = d.h.state().session.sequence_rate();
                let start = rate.tick_of(24);
                d.h.state_mut().session.execute("project.setMarks", json!({"item":MEDIA.0,"in":start.0,"out":rate.tick_of(71).0})).unwrap();
                d.step();
                let range = source_view(&d.h.state().session, MEDIA).unwrap().selected_range();
                (range.duration, range.start)
            } else {
                (d.h.state().session.project.item(MEDIA).unwrap().duration(), Tick::ZERO)
            };
            // Preview uses the raw source span; placement conforms it to the sequence frame grid.
            let rate = d.h.state().session.sequence_rate();
            let placed_duration = rate.snap_nearest(duration).max(rate.frame_duration());
            let from = d.rect(if source { "source.drag.video" } else { "project.item.5" }).center();
            let (to, start) = d.target();
            let before = d.project();
            let prefs = d.h.state().session.prefs.to_value();
            let modifiers = insert_modifier();
            d.hold(from, to, modifiers);
            let preview = d.drop_rect(start, duration);
            assert!(d.layout().content.contains(preview.right_center()), "preview must stay on a neutral visible row");
            d.assert_rect_feedback(preview, 1.5, Some(40), 255);
            d.assert_label("Insert", preview.left_top() + vec2(4.0, -2.0), true);
            d.assert_unchanged(&before, &prefs);
            d.shot(if source { "source-insert-held" } else { "project-insert-held" });
            d.release(to, modifiers);
            let q = d.h.state().session.active_sequence().unwrap();
            assert_eq!(q.video_tracks[0].items.len(), 1, "release places the preview once: {}", d.h.state().ui.status);
            let placed = &q.video_tracks[0].items[0];
            assert_eq!((placed.item, placed.start, placed.duration, placed.source_in), (MEDIA, start, placed_duration, source_in));
            if source {
                assert!(q.audio_tracks.iter().all(|track| track.items.is_empty()), "video-only source drag cannot place audio");
            }
            d.h.state_mut().session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(d.project(), before, "release remains one undoable placement");
        }
    }
}

#[test]
fn system_light_and_dark_paint_held_template_outline_before_placement() {
    for system in [egui::Theme::Light, egui::Theme::Dark] {
        let mut d = Driver::new(system);
        d.h.state_mut().set_workspace("Captions and Graphics");
        d.h.state_mut().show_panel(filmcraft_ui_egui::dock::PanelKind::EssentialGraphics);
        d.h.state_mut().ui.gfx_templates.tab = "browse".into();
        d.h.state_mut().ui.gfx_templates.query = "tag".into();
        d.frames(4);
        d.fix_zoom();
        let from = d.rect("gfxTemplates.item.builtin:callout-tag").center();
        let (to, start) = d.target();
        let before = d.project();
        let prefs = d.h.state().session.prefs.to_value();
        d.hold(from, to, Modifiers::NONE);
        let row = d.video_row();
        let x = d.layout().x_of(start);
        let preview = Rect::from_min_max(pos2(x, row.rect.min.y + 1.0), pos2(x + 60.0, row.rect.max.y - 1.0));
        d.assert_rect_feedback(preview, 1.5, None, 255);
        d.assert_unchanged(&before, &prefs);
        d.shot("template-held");
        d.release(to, Modifiers::NONE);
        let q = d.h.state().session.active_sequence().unwrap();
        assert_eq!(q.video_tracks[0].items.len(), 1, "template placed once: {}", d.h.state().ui.status);
        assert_eq!(q.video_tracks[0].items[0].start, start);
        assert!(q.video_tracks[0].items[0].name.contains("Callout"));
        d.h.state_mut().session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(d.project(), before);
    }
}

#[test]
fn system_light_and_dark_paint_actual_os_file_hover_counts_and_clear_them() {
    for system in [egui::Theme::Light, egui::Theme::Dark] {
        let mut d = Driver::new(system);
        let (to, start) = d.target();
        let before = d.project();
        let prefs = d.h.state().session.prefs.to_value();
        for count in [1, 2] {
            d.h.input_mut().hovered_files = vec![egui::HoveredFile { path: None, mime: "image/png".into() }; count];
            d.pointer(to, None, Modifiers::NONE);
            let row = d.video_row();
            let x = d.layout().x_of(start);
            let preview = Rect::from_min_max(pos2(x, row.rect.min.y + 1.0), pos2((x + 120.0).min(d.layout().content.max.x), row.rect.max.y - 1.0));
            d.assert_rect_feedback(preview, 1.5, Some(40), 255);
            d.assert_label(if count == 1 { "1 file" } else { "2 files" }, preview.left_top() + vec2(4.0, 2.0), false);
            d.assert_unchanged(&before, &prefs);
            d.shot(if count == 1 { "os-one-file-held" } else { "os-two-files-held" });
            d.h.input_mut().hovered_files.clear();
            d.frames(2);
            assert!(
                !d.shapes().iter().any(|shape| matches!(shape, Shape::Rect(r) if same_rect(r.rect, preview) && (r.stroke.width - 1.5).abs() < 0.01)),
                "leaving the window must remove the file-drop preview"
            );
            d.assert_unchanged(&before, &prefs);
        }
    }
}

#[test]
fn system_light_and_dark_paint_actual_empty_row_marquee() {
    for system in [egui::Theme::Light, egui::Theme::Dark] {
        let mut d = Driver::new(system);
        let row = d.video_row();
        let from = pos2(row.rect.min.x + 350.0, row.rect.min.y + 6.0);
        let to = pos2(from.x + 110.0, row.rect.max.y - 6.0);
        assert!(row.rect.contains(from) && row.rect.contains(to));
        let before = d.project();
        let prefs = d.h.state().session.prefs.to_value();
        d.hold(from, to, Modifiers::NONE);
        assert!(matches!(d.h.state().tl.drag, Some(Drag::Marquee { .. })), "the pointer must exercise the real empty-row marquee branch");
        d.assert_rect_feedback(Rect::from_two_pos(from, to), 1.0, Some(18), 140);
        d.assert_unchanged(&before, &prefs);
        d.shot("marquee-held");
        d.release(to, Modifiers::NONE);
        assert!(d.h.state().tl.drag.is_none());
        d.assert_unchanged(&before, &prefs);
    }
}

#[test]
fn system_light_and_dark_keep_actual_selected_gap_visible() {
    for system in [egui::Theme::Light, egui::Theme::Dark] {
        let mut d = Driver::new(system);
        let rate = d.h.state().session.sequence_rate();
        for frame in [0, 96] {
            d.h.state_mut().session.execute("timeline.place", json!({"item":MEDIA.0,"frame":frame,"duration":rate.tick_of(48).0})).unwrap();
        }
        d.h.state_mut().session.execute("edit.deselectAll", json!({})).unwrap();
        d.frames(3);
        d.fix_zoom();
        let before = d.project();
        let prefs = d.h.state().session.prefs.to_value();
        let row = d.video_row();
        let gap =
            Rect::from_min_max(pos2(d.layout().x_of(rate.tick_of(48)), row.rect.min.y + 1.0), pos2(d.layout().x_of(rate.tick_of(96)), row.rect.max.y - 1.0));
        d.pointer(gap.center(), Some(true), Modifiers::NONE);
        d.pointer(gap.center(), Some(false), Modifiers::NONE);
        d.step();
        let selected = d.h.state().session.state.gap_selection.expect("the real click selects the gap");
        assert_eq!((selected.track, selected.range.start, selected.range.end()), (row.track, rate.tick_of(48), rate.tick_of(96)));
        assert!(same_rect(d.rect("timeline.gap"), gap));
        d.assert_rect_feedback(gap, 1.0, None, 255);
        assert!(
            d.shapes().into_iter().any(|shape| matches!(shape, Shape::Rect(r) if same_rect(r.rect, gap) && r.fill == Color32::from_gray(224))),
            "the existing selected-gap block is retained"
        );
        d.assert_unchanged(&before, &prefs);
        d.shot("selected-gap");
        let after = gap.right_bottom() + vec2(100.0, 0.0);
        d.pointer(after, Some(true), Modifiers::NONE);
        d.pointer(after, Some(false), Modifiers::NONE);
        assert!(d.h.state().session.state.gap_selection.is_none(), "clicking after the last clip clears selection");
        d.assert_unchanged(&before, &prefs);
    }
}
