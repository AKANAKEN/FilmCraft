//! Program-monitor Motion handles for video, stills, subclips and nested sequences.
//! Geometry uses the renderer's exact Motion/PAR mapping. Project changes are atomic
//! `motion.set` commands, merged for the duration of one pointer gesture.

use egui::{Color32, Pos2, Rect, Response, Stroke, StrokeKind, pos2, vec2};
use filmcraft_engine::preview::Element;
use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::{ClipId, ItemKind};
use filmcraft_time::Tick;
use serde_json::json;

use super::graphics::LayerView;
use crate::{FilmcraftApp, state::Tool};

#[derive(Clone)]
struct View {
    clip: ClipId,
    to_screen: Affine,
    screen_to_frame: Affine,
    size: (u32, u32),
    position: Vec2,
    scale: f64,
    scale_width: f64,
    quad: [Pos2; 4],
}

impl View {
    fn hit(&self, p: Pos2) -> bool {
        self.to_screen.inverse().is_some_and(|m| {
            let p = m.apply(Vec2::new(p.x as f64, p.y as f64));
            p.x >= 0.0 && p.y >= 0.0 && p.x <= self.size.0 as f64 && p.y <= self.size.1 as f64
        })
    }
    fn handle(&self, p: Pos2) -> Option<usize> {
        self.quad.iter().position(|q| (*q - p).length() <= 9.0)
    }
    fn delta(&self, d: egui::Vec2) -> Vec2 {
        Affine { e: 0.0, f: 0.0, ..self.screen_to_frame }.apply(Vec2::new(d.x as f64, d.y as f64))
    }
}

fn views(app: &FilmcraftApp, pic: Rect, frame: (u32, u32)) -> Vec<View> {
    let Some(seq) = app.session.active_sequence() else { return Vec::new() };
    if pic.width() <= 0.0 || pic.height() <= 0.0 || frame.0 == 0 || frame.1 == 0 {
        return Vec::new();
    }
    let screen = Affine::translate(pic.min.x as f64, pic.min.y as f64)
        .then_apply(&Affine::scale(pic.width() as f64 / frame.0 as f64, pic.height() as f64 / frame.1 as f64));
    let Some(screen_to_frame) = screen.inverse() else { return Vec::new() };
    let time = app.session.playhead();
    seq.video_tracks
        .iter()
        .rev()
        .filter(|tr| tr.enabled && !tr.locked)
        .filter_map(|tr| {
            let it = tr.item_at(time).filter(|it| it.enabled)?;
            // Re-enabling a disabled Motion could revive an old rotation/anchor. Let the
            // user enable it in the panel first; never reset those values behind their back.
            it.effect("motion").filter(|effect| effect.enabled)?;
            let item = app.session.project.item(it.item)?;
            if matches!(item.kind, ItemKind::Graphic { .. } | ItemKind::AdjustmentLayer { .. }) {
                return None;
            }
            let size = filmcraft_render::source_size(&app.session.project, it.item).filter(|(w, h)| *w > 0 && *h > 0)?;
            let mt = it.source_time_at(time);
            let motion = filmcraft_render::motion_matrix(seq, it, size, filmcraft_render::source_par(&app.session.project, it.item), mt);
            let to_screen = screen.then_apply(&motion);
            to_screen.inverse()?;
            let quad = [(0.0, 0.0), (size.0 as f64, 0.0), (size.0 as f64, size.1 as f64), (0.0, size.1 as f64)].map(|(x, y)| {
                let p = to_screen.apply(Vec2::new(x, y));
                pos2(p.x as f32, p.y as f32)
            });
            if quad.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
                return None;
            }
            let anchor = it
                .effect("motion")
                .filter(|e| e.enabled)
                .map(|e| e.vec2_at("anchor", mt))
                .filter(|p| p.x.is_finite() && p.y.is_finite())
                .unwrap_or(Vec2::new(size.0 as f64 / 2.0, size.1 as f64 / 2.0));
            let position = motion.apply(anchor);
            let effect = it.effect("motion").filter(|e| e.enabled);
            let scale = effect.map_or(100.0, |e| e.f64_at("scale", mt));
            let uniform = effect.and_then(|e| e.param("uniform_scale")).and_then(|p| p.value.as_bool()).unwrap_or(true);
            let scale_width = if uniform { scale } else { effect.map_or(100.0, |e| e.f64_at("scale_width", mt)) };
            Some(View { clip: it.id, to_screen, screen_to_frame, size, position, scale, scale_width, quad })
        })
        .collect()
}

/// Respect video-track stacking: graphic glyphs/shapes above a media clip own their hits.
fn hit<'a>(app: &FilmcraftApp, views: &'a [View], graphics: &[LayerView], p: Pos2) -> Option<&'a View> {
    let seq = app.session.active_sequence()?;
    for tr in seq.video_tracks.iter().rev().filter(|tr| tr.enabled) {
        let Some(it) = tr.item_at(app.session.playhead()).filter(|it| it.enabled) else { continue };
        if graphics.iter().any(|v| v.clip == it.id && v.hit(p)) {
            return None;
        }
        if let Some(v) = views.iter().find(|v| v.clip == it.id && v.hit(p)) {
            return Some(v);
        }
        // A locked media track is visible but cannot be selected through another layer.
        if (tr.locked || it.effect("motion").is_some_and(|effect| !effect.enabled))
            && let Some(size) = app.session.project.source_size(it.item)
        {
            let m = filmcraft_render::motion_matrix(seq, it, size, app.session.project.source_par(it.item), it.source_time_at(app.session.playhead()));
            if let Some(inverse) = m.inverse() {
                let frame = views.first()?.screen_to_frame.apply(Vec2::new(p.x as f64, p.y as f64));
                let local = inverse.apply(frame);
                if local.x >= 0.0 && local.y >= 0.0 && local.x <= size.0 as f64 && local.y <= size.1 as f64 {
                    return None;
                }
            }
        }
    }
    None
}

fn graphic_handle(app: &FilmcraftApp, graphics: &[LayerView], p: Pos2) -> bool {
    if filmcraft_engine::preview::selected(&app.session).len() > 1 {
        return false;
    }
    graphics.iter().filter(|v| app.session.state.selection.contains(&v.clip) && app.session.state.graphic_layers.contains(&v.layer)).any(|v| {
        let q = v.quad();
        let corners = q.iter().any(|c| (*c - p).length() <= 9.0);
        let edges = (0..4).any(|n| (q[n] + (q[(n + 1) % 4] - q[n]) * 0.5 - p).length() <= 9.0);
        let a = v.to_screen.apply(v.spec.transform.anchor);
        corners || edges || (pos2(a.x as f32, a.y as f32) - p).length() <= 7.0
    })
}

pub(crate) fn selection_geometry(app: &FilmcraftApp, pic: Rect, frame: (u32, u32), clip: ClipId) -> Option<(Vec2, Affine, [Pos2; 4])> {
    views(app, pic, frame).into_iter().find(|v| v.clip == clip).map(|v| (v.position, v.screen_to_frame, v.quad))
}

pub(crate) fn body_hit(app: &FilmcraftApp, pic: Rect, frame: (u32, u32), clip: ClipId, p: Pos2) -> bool {
    views(app, pic, frame).iter().any(|v| v.clip == clip && v.hit(p))
}

pub(crate) fn front_element(app: &FilmcraftApp, pic: Rect, frame: (u32, u32), graphics: &[LayerView], p: Pos2) -> Option<Element> {
    let seq = app.session.active_sequence()?;
    let media = views(app, pic, frame);
    for tr in seq.video_tracks.iter().rev().filter(|t| t.enabled) {
        let Some(it) = tr.item_at(app.session.playhead()).filter(|it| it.enabled) else { continue };
        if let Some(v) = graphics.iter().find(|v| v.clip == it.id && v.hit(p)) {
            return (!tr.locked).then_some(Element { clip: v.clip, layer: Some(v.layer) });
        }
        if let Some(v) = media.iter().find(|v| v.clip == it.id && v.hit(p)) {
            return Some(Element { clip: v.clip, layer: None });
        }
        if (tr.locked || it.effect("motion").is_some_and(|e| !e.enabled))
            && pic.width() > 0.0
            && pic.height() > 0.0
            && let Some(size) = app.session.project.source_size(it.item)
        {
            let motion = filmcraft_render::motion_matrix(seq, it, size, app.session.project.source_par(it.item), it.source_time_at(app.session.playhead()));
            let point =
                Vec2::new((p.x - pic.min.x) as f64 / pic.width() as f64 * frame.0 as f64, (p.y - pic.min.y) as f64 / pic.height() as f64 * frame.1 as f64);
            if motion.inverse().is_some_and(|m| {
                let v = m.apply(point);
                v.x >= 0.0 && v.y >= 0.0 && v.x <= size.0 as f64 && v.y <= size.1 as f64
            }) {
                return None;
            }
        }
    }
    None
}

#[derive(Clone)]
struct Drag {
    view: View,
    start: Pos2,
    corner: Option<usize>,
    time: Tick,
    begun: bool,
    last: Option<(Vec2, f64, f64)>,
    duplicate: bool,
    gesture: ClipId,
}

impl Drag {
    fn values(&self, current: Pos2, movement: egui::Vec2) -> (Vec2, f64, f64) {
        let v = &self.view;
        let Some(n) = self.corner else { return (v.position + v.delta(movement), v.scale, v.scale_width) };
        let pin = v.quad[(n + 2) % 4];
        let diagonal = v.quad[n] - pin;
        let denominator = diagonal.length_sq();
        let f = if denominator > 1e-6 { ((diagonal + (current - self.start)).dot(diagonal) / denominator) as f64 } else { 1.0 };
        let max = (10000.0 / v.scale.abs().max(v.scale_width.abs()).max(1.0)).max(0.0001);
        let f = f.clamp(0.0001, max);
        let anchor = v.screen_to_frame.inverse().map(|m| m.apply(v.position)).unwrap_or_default();
        let anchor = pos2(anchor.x as f32, anchor.y as f32);
        let shifted = pin + (anchor - pin) * f as f32;
        (v.position + v.delta(shifted - anchor), v.scale * f, v.scale_width * f)
    }
}

/// Share the graphics overlay's response, avoiding overlapping full-picture input widgets.
/// Returns true only while media owns a press/drag/click; graphics still handles empty space.
pub(crate) fn monitor_overlay(
    app: &mut FilmcraftApp,
    ui: &mut egui::Ui,
    pic: Rect,
    frame: (u32, u32),
    response: &Response,
    graphics: &[LayerView],
    allow_start: bool,
) -> bool {
    let id = egui::Id::new("media-motion-drag");
    if app.ui.tool != Tool::Selection || app.session.state.selected_mask.is_some() || app.ui.mask_pen.is_some() || app.playback.playing {
        ui.data_mut(|d| d.remove::<Drag>(id));
        return false;
    }
    let additive_click = response.clicked() && super::preview_selection::shift_click(ui);
    if super::transform_handles::owns_media(app, pic, frame) && !ui.input(|i| i.modifiers.alt) && !additive_click {
        ui.data_mut(|d| d.remove::<Drag>(id));
        return false;
    }
    let views = views(app, pic, frame);
    let selected = views.iter().find(|v| app.session.state.selection.contains(&v.clip));
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    for v in &views {
        let visible = Rect::from_points(&v.quad).intersect(pic);
        if visible.is_positive() {
            app.auto.add(&format!("program.media.{}", v.clip.0), visible, "Move clip");
        }
    }
    if let Some(v) = selected
        && filmcraft_engine::preview::selected(&app.session).len() < 2
    {
        painter.add(egui::Shape::closed_line(v.quad.to_vec(), Stroke::new(1.0, app.tokens.accent)));
        for (n, p) in v.quad.iter().enumerate() {
            let r = Rect::from_center_size(*p, vec2(8.0, 8.0));
            painter.rect_filled(r, 0.0, Color32::WHITE);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, app.tokens.accent), StrokeKind::Middle);
            app.auto.add(&format!("program.media.{}.handle.{n}", v.clip.0), r.expand(4.0), "Resize clip");
        }
    }
    if let Some(p) = response.hover_pos().filter(|p| !graphic_handle(app, graphics, *p)) {
        if let Some(n) = selected.and_then(|v| v.handle(p)) {
            ui.ctx().set_cursor_icon(if n % 2 == 0 { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw });
        } else if hit(app, &views, graphics, p).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }
    }
    let mut drag: Option<Drag> = ui.data(|d| d.get_temp(id));
    if response.drag_started() && allow_start {
        let start = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos());
        if let Some(start) = start.filter(|p| !graphic_handle(app, graphics, *p)) {
            let handle = selected.and_then(|v| v.handle(start).map(|n| (v, Some(n))));
            let target = handle.or_else(|| hit(app, &views, graphics, start).map(|v| (v, None)));
            if let Some((v, corner)) = target {
                select(app, ui, v.clip, false);
                drag = Some(Drag {
                    view: v.clone(),
                    start,
                    corner,
                    time: app.session.playhead(),
                    begun: false,
                    last: None,
                    duplicate: corner.is_none() && ui.input(|i| i.modifiers.alt),
                    gesture: v.clip,
                });
            }
        }
    }
    let handled = drag.is_some();
    if let Some(d) = &mut drag {
        // A deleted/locked/hidden clip, changed playhead or source ends the gesture safely.
        if d.time != app.session.playhead() || !views.iter().any(|v| v.clip == d.view.clip) {
            drag = None;
        } else if let Some(current) = response.interact_pointer_pos() {
            let (movement, lines) = if d.corner.is_none() && !ui.input(|i| i.modifiers.command) {
                super::monitor_view::snap_move(app, pic, frame, Rect::from_points(&d.view.quad), current - d.start)
            } else {
                (current - d.start, Vec::new())
            };
            super::monitor_view::draw_snap_lines(&painter, pic, &lines);
            if d.begun || (current - d.start).length() > 2.0 {
                let (position, scale, scale_width) = d.values(current, movement);
                if d.last != Some((position, scale, scale_width)) {
                    let mut params = json!({"clip": d.view.clip.0, "position": [position.x, position.y],
                        "time": d.time.0, "merge": true, "begin": !d.begun});
                    if d.corner.is_some() {
                        params["scale"] = json!(scale);
                        params["scaleWidth"] = json!(scale_width);
                    }
                    let command = if d.corner.is_none() {
                        params["duplicate"] = json!(d.duplicate && !d.begun);
                        params["gesture"] = json!(d.gesture.0);
                        "preview.move"
                    } else {
                        "motion.set"
                    };
                    match app.session.execute(command, params) {
                        Err(e) => {
                            app.ui.status = e.to_string();
                            drag = None;
                        }
                        Ok(result) => {
                            if d.duplicate
                                && !d.begun
                                && let Some(id) = result["clip"].as_u64()
                            {
                                d.view.clip = ClipId(id);
                            }
                            d.begun = true;
                            d.last = Some((position, scale, scale_width));
                        }
                    }
                    ui.ctx().request_repaint();
                }
            }
        }
    }
    if response.drag_stopped() {
        drag = None;
    }
    ui.data_mut(|data| {
        if let Some(d) = drag {
            data.insert_temp(id, d);
        } else {
            data.remove::<Drag>(id);
        }
    });
    if !handled
        && response.clicked()
        && allow_start
        && let Some(p) = response.interact_pointer_pos()
        && !graphic_handle(app, graphics, p)
        && let Some(v) = hit(app, &views, graphics, p)
    {
        select(app, ui, v.clip, super::preview_selection::shift_click(ui));
        return true;
    }
    handled
}

fn select(app: &mut FilmcraftApp, ui: &egui::Ui, clip: ClipId, add: bool) {
    if add || !filmcraft_engine::preview::selected(&app.session).is_empty() {
        super::preview_selection::choose(app, Element { clip, layer: None }, add);
        return;
    }
    super::graphics::deselect_on_monitor(app, ui);
    if let Err(e) = app.session.execute("timeline.select", json!({"clips": [clip.0]})) {
        app.ui.status = e.to_string();
    }
}
