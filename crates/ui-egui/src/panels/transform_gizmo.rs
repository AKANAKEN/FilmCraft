//! Program monitor transform gizmo (bounding box, scale handles, rotation, anchor point, move).
//!
//! Provides on-screen direct manipulation for video clips targeting the intrinsic Motion effect
//! (position, scale, rotation, anchor point). Addresses Premiere Pro parity issue #639.

use egui::{Color32, CursorIcon, Id, Pos2, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::{ClipId, ItemKind, TrackItem, TrackKind};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::state::Tool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoDragKind {
    Move,
    Scale(usize),
    Rotate(usize),
    Anchor,
}

#[derive(Clone, Copy, Debug)]
pub struct GizmoDragState {
    pub kind: GizmoDragKind,
    pub clip: ClipId,
    pub start: Pos2,
    pub initial_pos: Vec2,
    pub initial_scale: Vec2,
    pub initial_rot: f64,
    pub initial_anchor: Vec2,
    pub initial_size: (u32, u32),
}

#[derive(Clone, Debug)]
pub struct ClipMotion {
    pub pos: Vec2,
    pub scale: Vec2,
    pub uniform_scale: bool,
    pub rotation: f64,
    pub anchor: Vec2,
    pub size: (u32, u32),
}

/// Find the primary selected video clip active at the playhead.
pub fn selected_video_clip(app: &FilmcraftApp) -> Option<(ClipId, TrackItem)> {
    let seq = app.session.active_sequence()?;
    let ph = app.session.playhead();
    for &cid in &app.session.state.selection {
        if let Some((tid, it)) = seq.find_item(cid)
            && seq.track(tid).is_some_and(|t| t.kind == TrackKind::Video)
            && it.enabled
            && ph >= it.start
            && ph < it.end()
        {
            return Some((cid, it.clone()));
        }
    }
    None
}

/// Read intrinsic Motion properties and source size for a track item at `mt`.
pub fn clip_motion(app: &FilmcraftApp, it: &TrackItem, frame: (u32, u32), mt: Tick) -> ClipMotion {
    let seq = app.session.active_sequence();
    let (sw, sh) = seq.map(|s| (s.settings.width as f64, s.settings.height as f64)).unwrap_or((frame.0 as f64, frame.1 as f64));
    let size = filmcraft_render::source_size(&app.session.project, it.item).unwrap_or(frame);
    let mut pos = Vec2::new(sw / 2.0, sh / 2.0);
    let mut anchor = Vec2::new(size.0 as f64 / 2.0, size.1 as f64 / 2.0);
    let mut scale = 100.0;
    let mut scale_width = 100.0;
    let mut uniform_scale = true;
    let mut rotation = 0.0;

    if let Some(m) = it.effect("motion") {
        let p = m.vec2_at("position", mt);
        if !p.x.is_nan() && m.param("position").is_some() {
            pos = p;
        }
        let a = m.vec2_at("anchor", mt);
        if !a.x.is_nan() && m.param("anchor").is_some() {
            anchor = a;
        }
        scale = m.f64_at("scale", mt);
        scale_width = m.f64_at("scale_width", mt);
        uniform_scale = m.param("uniform_scale").and_then(|p| p.value.as_bool()).unwrap_or(true);
        rotation = m.f64_at("rotation", mt);
    }

    ClipMotion { pos, scale: Vec2::new(if uniform_scale { scale } else { scale_width }, scale), uniform_scale, rotation, anchor, size }
}

/// Compute Motion matrix directly from explicit parameter values and sequence properties.
fn motion_matrix_from_values(
    seq: &filmcraft_project::Sequence,
    src: (u32, u32),
    src_par: Option<(u32, u32)>,
    scale_to_frame: bool,
    pos: Vec2,
    scale: Vec2,
    rot: f64,
    anchor: Vec2,
) -> Affine {
    let (sw, sh) = (seq.settings.width as f64, seq.settings.height as f64);
    let seq_par = filmcraft_project::sane_par(seq.settings.par);
    let src_par = src_par.map_or(seq_par, filmcraft_project::sane_par);
    let (p, q) = (filmcraft_project::par_ratio(src_par), filmcraft_project::par_ratio(seq_par));
    let mut s = Vec2::new(scale.x / 100.0, scale.y / 100.0);
    if scale_to_frame {
        let fit = (sw * q / (src.0.max(1) as f64 * p)).min(sh / (src.1.max(1) as f64));
        s = s * fit;
    }
    if rot == 0.0 {
        let r = if src_par == seq_par { 1.0 } else { p / q };
        return Affine::motion(pos, Vec2::new(s.x * r, s.y), rot, anchor);
    }
    Affine::translate(pos.x, pos.y).then_apply(&Affine::scale(1.0 / q, 1.0)).then_apply(&Affine::motion(Vec2::ZERO, Vec2::new(s.x * p, s.y), rot, anchor))
}

/// Handle location in clip local space `(0..w, 0..h)`.
fn clip_handle_point(n: usize, w: f64, h: f64) -> Vec2 {
    match n {
        0 => Vec2::new(0.0, 0.0),
        1 => Vec2::new(w, 0.0),
        2 => Vec2::new(w, h),
        3 => Vec2::new(0.0, h),
        4 => Vec2::new(w / 2.0, 0.0),
        5 => Vec2::new(w, h / 2.0),
        6 => Vec2::new(w / 2.0, h),
        7 => Vec2::new(0.0, h / 2.0),
        _ => Vec2::new(w / 2.0, h / 2.0),
    }
}

/// Opposite pin location in clip local space.
fn clip_pin_point(n: usize, w: f64, h: f64, alt: bool, anchor: Vec2) -> Vec2 {
    if alt {
        return anchor;
    }
    match n {
        0 => Vec2::new(w, h),
        1 => Vec2::new(0.0, h),
        2 => Vec2::new(0.0, 0.0),
        3 => Vec2::new(w, 0.0),
        4 => Vec2::new(w / 2.0, h),
        5 => Vec2::new(0.0, h / 2.0),
        6 => Vec2::new(w / 2.0, 0.0),
        7 => Vec2::new(w, h / 2.0),
        _ => anchor,
    }
}

/// Sequence pixels -> screen points for the monitor picture `pic`.
fn sequence_to_screen(pic: Rect, frame: (u32, u32)) -> Affine {
    let kx = pic.width() as f64 / (frame.0.max(1) as f64);
    let ky = pic.height() as f64 / (frame.1.max(1) as f64);
    Affine::translate(pic.min.x as f64, pic.min.y as f64).then_apply(&Affine::scale(kx, ky))
}

fn sp(m: &Affine, v: Vec2) -> Pos2 {
    let q = m.apply(v);
    pos2(q.x as f32, q.y as f32)
}

/// The 8 handle positions of a quad (TL, TR, BR, BL, Top, Right, Bottom, Left).
fn handle_points(quad: &[Pos2; 4]) -> [Pos2; 8] {
    let mid = |a: usize, b: usize| quad[a] + (quad[b] - quad[a]) * 0.5;
    [quad[0], quad[1], quad[2], quad[3], mid(0, 1), mid(1, 2), mid(2, 3), mid(3, 0)]
}

/// Hit test a point against a polygon quad.
fn hit_quad(quad: &[Pos2; 4], p: Pos2) -> bool {
    let n = 4;
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let pi = quad[i];
        let pj = quad[j];
        if (pi.y > p.y) != (pj.y > p.y) && p.x < (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y + 1e-9) + pi.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Compute cursor icon based on rotation-adjusted handle direction.
fn scale_cursor(rot: f64, n: usize) -> CursorIcon {
    let base_angle = match n {
        0 | 2 => 45.0,  // NW-SE diagonal
        1 | 3 => 135.0, // NE-SW diagonal
        4 | 6 => 90.0,  // N-S vertical
        5 | 7 => 0.0,   // E-W horizontal
        _ => 0.0,
    };
    let a = (base_angle + rot).rem_euclid(180.0);
    if (22.5..67.5).contains(&a) {
        CursorIcon::ResizeNwSe
    } else if (67.5..112.5).contains(&a) {
        CursorIcon::ResizeVertical
    } else if (112.5..157.5).contains(&a) {
        CursorIcon::ResizeNeSw
    } else {
        CursorIcon::ResizeHorizontal
    }
}

/// Render the transform gizmo overlay and handle direct manipulation interactions.
pub fn monitor_overlay(app: &mut FilmcraftApp, ui: &mut Ui, pic: Rect, frame: (u32, u32)) {
    // Yield if an effect mask is selected or graphic text is currently being edited
    if app.session.state.selected_mask.is_some() || app.ui.gfx_edit.is_some() {
        return;
    }

    let Some(seq) = app.session.active_sequence().cloned() else { return };
    let ph = app.session.playhead();
    let view = sequence_to_screen(pic, frame);

    let sel_clip = selected_video_clip(app);
    let t = app.tokens;
    let accent = t.accent;
    let painter = ui.painter().with_clip_rect(pic.expand(16.0));

    let gizmo_id = Id::new("transform-gizmo");
    let drag_id = gizmo_id.with("drag");
    let mut drag: Option<GizmoDragState> = ui.data(|d| d.get_temp(drag_id));

    let mut actions: Vec<(String, Value)> = Vec::new();

    // 1. If a clip is selected, compute its projected quad, handles, and anchor
    if let Some((clip_id, ref it)) = sel_clip {
        let mt = it.source_time_at(ph);
        let mot = clip_motion(app, it, frame, mt);
        let src_par = filmcraft_render::source_par(&app.session.project, it.item);
        let motion_aff = filmcraft_render::motion_matrix(&seq, it, mot.size, src_par, mt);
        let to_screen = view.then_apply(&motion_aff);

        let (w, h) = (mot.size.0 as f64, mot.size.1 as f64);
        let quad = [sp(&to_screen, Vec2::new(0.0, 0.0)), sp(&to_screen, Vec2::new(w, 0.0)), sp(&to_screen, Vec2::new(w, h)), sp(&to_screen, Vec2::new(0.0, h))];
        let handles = handle_points(&quad);
        let anchor_screen = sp(&to_screen, mot.anchor);

        // Automation registration
        let box_rect = Rect::from_points(&quad);
        app.auto.add(&format!("program.clip.{}.box", clip_id.0), box_rect, "clip bounding box");
        for (n, pt) in handles.iter().enumerate() {
            let hr = Rect::from_center_size(*pt, vec2(10.0, 10.0));
            app.auto.add(&format!("program.clip.{}.handle.{n}", clip_id.0), hr, "transform handle");
        }
        app.auto.add(&format!("program.clip.{}.anchor", clip_id.0), Rect::from_center_size(anchor_screen, vec2(14.0, 14.0)), "anchor point");

        // Interaction processing only when Selection tool is active
        if app.ui.tool == Tool::Selection {
            let hover_pos = ui.input(|i| i.pointer.hover_pos());

            // Handle press start
            if ui.input(|i| i.pointer.any_pressed())
                && drag.is_none()
                && let Some(pos) = hover_pos
            {
                // Check anchor point first (10px radius)
                if (pos - anchor_screen).length() <= 8.0 {
                    drag = Some(GizmoDragState {
                        kind: GizmoDragKind::Anchor,
                        clip: clip_id,
                        start: pos,
                        initial_pos: mot.pos,
                        initial_scale: mot.scale,
                        initial_rot: mot.rotation,
                        initial_anchor: mot.anchor,
                        initial_size: mot.size,
                    });
                }
                // Check 8 resize handles
                else if let Some(n) = handles.iter().position(|h| (*h - pos).length() <= 7.0) {
                    drag = Some(GizmoDragState {
                        kind: GizmoDragKind::Scale(n),
                        clip: clip_id,
                        start: pos,
                        initial_pos: mot.pos,
                        initial_scale: mot.scale,
                        initial_rot: mot.rotation,
                        initial_anchor: mot.anchor,
                        initial_size: mot.size,
                    });
                }
                // Check rotation zone outside the 4 corners (8px to 22px distance)
                else if let Some(c) = quad.iter().position(|p| {
                    let d = (*p - pos).length();
                    d > 7.0 && d <= 22.0
                }) {
                    drag = Some(GizmoDragState {
                        kind: GizmoDragKind::Rotate(c),
                        clip: clip_id,
                        start: pos,
                        initial_pos: mot.pos,
                        initial_scale: mot.scale,
                        initial_rot: mot.rotation,
                        initial_anchor: mot.anchor,
                        initial_size: mot.size,
                    });
                }
                // Check translation (inside the bounding box)
                else if hit_quad(&quad, pos) {
                    drag = Some(GizmoDragState {
                        kind: GizmoDragKind::Move,
                        clip: clip_id,
                        start: pos,
                        initial_pos: mot.pos,
                        initial_scale: mot.scale,
                        initial_rot: mot.rotation,
                        initial_anchor: mot.anchor,
                        initial_size: mot.size,
                    });
                }
            }

            // Set appropriate cursor icons on hover
            if drag.is_none()
                && let Some(pos) = hover_pos
            {
                if (pos - anchor_screen).length() <= 8.0 {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                } else if let Some(n) = handles.iter().position(|h| (*h - pos).length() <= 7.0) {
                    ui.ctx().set_cursor_icon(scale_cursor(mot.rotation, n));
                } else if quad.iter().any(|p| {
                    let d = (*p - pos).length();
                    d > 7.0 && d <= 22.0
                }) {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                } else if hit_quad(&quad, pos) {
                    ui.ctx().set_cursor_icon(CursorIcon::Move);
                }
            }
        }

        // Active drag processing
        if let Some(d) = drag {
            let cur = ui.input(|i| i.pointer.latest_pos()).unwrap_or(d.start);
            let shift = ui.input(|i| i.modifiers.shift);
            let alt = ui.input(|i| i.modifiers.alt);
            let merge_key = format!(
                "gizmo:{}:{}",
                d.clip.0,
                match d.kind {
                    GizmoDragKind::Move => "move",
                    GizmoDragKind::Scale(_) => "scale",
                    GizmoDragKind::Rotate(_) => "rotate",
                    GizmoDragKind::Anchor => "anchor",
                }
            );

            // Compute the initial transformation matrix once for deterministic, flicker-free dragging
            let initial_motion =
                motion_matrix_from_values(&seq, d.initial_size, src_par, it.scale_to_frame, d.initial_pos, d.initial_scale, d.initial_rot, d.initial_anchor);
            let initial_to_screen = view.then_apply(&initial_motion);
            let (iw, ih) = (d.initial_size.0 as f64, d.initial_size.1 as f64);

            match d.kind {
                GizmoDragKind::Move => {
                    let delta_screen = cur - d.start;
                    let kx = pic.width() as f64 / (frame.0.max(1) as f64);
                    let ky = pic.height() as f64 / (frame.1.max(1) as f64);
                    let mut dx = (delta_screen.x as f64) / kx.max(1e-4);
                    let mut dy = (delta_screen.y as f64) / ky.max(1e-4);
                    if shift {
                        if dx.abs() > dy.abs() {
                            dy = 0.0;
                        } else {
                            dx = 0.0;
                        }
                    }
                    let new_pos = Vec2::new(d.initial_pos.x + dx, d.initial_pos.y + dy);
                    actions.push((
                        "effects.setParam".into(),
                        json!({
                            "clip": d.clip.0,
                            "effect": "motion",
                            "param": "position",
                            "value": [new_pos.x, new_pos.y],
                            "merge": true,
                            "merge_key": merge_key,
                        }),
                    ));
                }
                GizmoDragKind::Scale(n) => {
                    let p_tl = sp(&initial_to_screen, Vec2::new(0.0, 0.0));
                    let p_tr = sp(&initial_to_screen, Vec2::new(iw, 0.0));
                    let p_bl = sp(&initial_to_screen, Vec2::new(0.0, ih));
                    let ux = (p_tr - p_tl).normalized();
                    let uy = (p_bl - p_tl).normalized();

                    let pin_clip = clip_pin_point(n, iw, ih, alt, d.initial_anchor);
                    let handle_clip = clip_handle_point(n, iw, ih);

                    let initial_pin_screen = sp(&initial_to_screen, pin_clip);
                    let initial_handle_screen = sp(&initial_to_screen, handle_clip);

                    let d_x0 = (initial_handle_screen - initial_pin_screen).dot(ux) as f64;
                    let d_y0 = (initial_handle_screen - initial_pin_screen).dot(uy) as f64;
                    let d_x = (cur - initial_pin_screen).dot(ux) as f64;
                    let d_y = (cur - initial_pin_screen).dot(uy) as f64;

                    let mut fx = if d_x0.abs() > 1.0 { (d_x / d_x0).max(0.01) } else { 1.0 };
                    let mut fy = if d_y0.abs() > 1.0 { (d_y / d_y0).max(0.01) } else { 1.0 };

                    let force_uniform = mot.uniform_scale || shift;
                    if n < 4 {
                        // Corner handle
                        if force_uniform {
                            let f = if (fx - 1.0).abs() >= (fy - 1.0).abs() { fx } else { fy };
                            fx = f;
                            fy = f;
                        }
                    } else if matches!(n, 5 | 7) {
                        // Right/Left handles scale horizontal
                        fy = if force_uniform { fx } else { 1.0 };
                    } else {
                        // Top/Bottom handles scale vertical
                        fx = if force_uniform { fy } else { 1.0 };
                    }

                    let new_scale_x = (d.initial_scale.x * fx).clamp(0.1, 10000.0);
                    let new_scale_y = (d.initial_scale.y * fy).clamp(0.1, 10000.0);

                    // Compute position adjustment relative to the initial state to hold the pin fixed
                    let delta_s_c = Vec2::new(
                        (d.initial_scale.x - new_scale_x) / 100.0 * (pin_clip.x - d.initial_anchor.x),
                        (d.initial_scale.y - new_scale_y) / 100.0 * (pin_clip.y - d.initial_anchor.y),
                    );
                    let rot_aff = Affine::rotate_deg(d.initial_rot);
                    let delta_pos = rot_aff.apply(delta_s_c);
                    let new_pos = Vec2::new(d.initial_pos.x + delta_pos.x, d.initial_pos.y + delta_pos.y);

                    if mot.uniform_scale || shift {
                        actions.push((
                            "effects.setParam".into(),
                            json!({
                                "clip": d.clip.0,
                                "effect": "motion",
                                "param": "scale",
                                "value": new_scale_y,
                                "merge": true,
                                "merge_key": merge_key,
                            }),
                        ));
                    } else {
                        actions.push((
                            "effects.setParam".into(),
                            json!({
                                "clip": d.clip.0,
                                "effect": "motion",
                                "param": "scale_width",
                                "value": new_scale_x,
                                "merge": true,
                                "merge_key": merge_key,
                            }),
                        ));
                        actions.push((
                            "effects.setParam".into(),
                            json!({
                                "clip": d.clip.0,
                                "effect": "motion",
                                "param": "scale",
                                "value": new_scale_y,
                                "merge": true,
                                "merge_key": merge_key,
                            }),
                        ));
                    }

                    if !alt {
                        actions.push((
                            "effects.setParam".into(),
                            json!({
                                "clip": d.clip.0,
                                "effect": "motion",
                                "param": "position",
                                "value": [new_pos.x, new_pos.y],
                                "merge": true,
                                "merge_key": merge_key,
                            }),
                        ));
                    }
                }
                GizmoDragKind::Rotate(_) => {
                    let initial_anchor_screen = sp(&initial_to_screen, d.initial_anchor);
                    let v0 = d.start - initial_anchor_screen;
                    let v1 = cur - initial_anchor_screen;
                    let th0 = (v0.y as f64).atan2(v0.x as f64);
                    let th1 = (v1.y as f64).atan2(v1.x as f64);
                    let mut delta_rot = (th1 - th0).to_degrees();
                    if shift {
                        delta_rot = (delta_rot / 15.0).round() * 15.0;
                    }
                    let new_rot = d.initial_rot + delta_rot;
                    actions.push((
                        "effects.setParam".into(),
                        json!({
                            "clip": d.clip.0,
                            "effect": "motion",
                            "param": "rotation",
                            "value": new_rot,
                            "merge": true,
                            "merge_key": merge_key,
                        }),
                    ));
                }
                GizmoDragKind::Anchor => {
                    if let Some(initial_to_clip) = initial_to_screen.inverse() {
                        let initial_anchor_screen = sp(&initial_to_screen, d.initial_anchor);
                        let a_screen_cur = initial_anchor_screen + (cur - d.start);
                        let new_anchor_clip = initial_to_clip.apply(Vec2::new(a_screen_cur.x as f64, a_screen_cur.y as f64));

                        let delta_anchor_clip = new_anchor_clip - d.initial_anchor;
                        let s_factor = Vec2::new(d.initial_scale.x / 100.0, d.initial_scale.y / 100.0);
                        let rot_aff = Affine::rotate_deg(d.initial_rot);
                        let delta_seq = rot_aff.apply(Vec2::new(delta_anchor_clip.x * s_factor.x, delta_anchor_clip.y * s_factor.y));
                        let new_pos = d.initial_pos + delta_seq;

                        actions.push((
                            "effects.setParam".into(),
                            json!({
                                "clip": d.clip.0,
                                "effect": "motion",
                                "param": "anchor",
                                "value": [new_anchor_clip.x, new_anchor_clip.y],
                                "merge": true,
                                "merge_key": merge_key,
                            }),
                        ));
                        actions.push((
                            "effects.setParam".into(),
                            json!({
                                "clip": d.clip.0,
                                "effect": "motion",
                                "param": "position",
                                "value": [new_pos.x, new_pos.y],
                                "merge": true,
                                "merge_key": merge_key,
                            }),
                        ));
                    }
                }
            }

            if ui.input(|i| i.pointer.any_released()) {
                drag = None;
            }
        }

        // Draw gizmo wireframe, handles, and anchor point
        painter.add(egui::Shape::closed_line(quad.to_vec(), Stroke::new(1.5, accent)));

        for pt in &handles {
            let hr = Rect::from_center_size(*pt, vec2(7.0, 7.0));
            painter.rect_filled(hr, 0.0, Color32::WHITE);
            painter.rect_stroke(hr, 0.0, Stroke::new(1.0, accent), StrokeKind::Middle);
        }

        painter.circle_stroke(anchor_screen, 5.0, Stroke::new(1.5, accent));
        painter.circle_filled(anchor_screen, 1.5, accent);
        painter.line_segment([anchor_screen - vec2(8.0, 0.0), anchor_screen + vec2(8.0, 0.0)], Stroke::new(1.0, accent));
        painter.line_segment([anchor_screen - vec2(0.0, 8.0), anchor_screen + vec2(0.0, 8.0)], Stroke::new(1.0, accent));
    } else if app.ui.tool == Tool::Selection && ui.input(|i| i.pointer.primary_clicked()) {
        // 2. No clip selected: hit test visible video clips to select one on click
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
            && pic.contains(pos)
        {
            let mut clicked_clip = None;
            for tr in seq.video_tracks.iter().rev() {
                if !tr.enabled {
                    continue;
                }
                let Some(it) = tr.item_at(ph) else { continue };
                if !it.enabled {
                    continue;
                }
                // Exclude graphic clips if they have their own layer hit-testing
                let is_graphic = app.session.project.item(it.item).is_some_and(|p| matches!(p.kind, ItemKind::Graphic { .. }));
                if is_graphic {
                    continue;
                }
                let mt = it.source_time_at(ph);
                let mot = clip_motion(app, it, frame, mt);
                let src_par = filmcraft_render::source_par(&app.session.project, it.item);
                let motion_aff = filmcraft_render::motion_matrix(&seq, it, mot.size, src_par, mt);
                let to_screen = view.then_apply(&motion_aff);
                let (w, h) = (mot.size.0 as f64, mot.size.1 as f64);
                let q = [
                    sp(&to_screen, Vec2::new(0.0, 0.0)),
                    sp(&to_screen, Vec2::new(w, 0.0)),
                    sp(&to_screen, Vec2::new(w, h)),
                    sp(&to_screen, Vec2::new(0.0, h)),
                ];
                if hit_quad(&q, pos) {
                    clicked_clip = Some(it.id);
                    break;
                }
            }

            if let Some(cid) = clicked_clip {
                actions.push(("timeline.select".into(), json!({"clips": [cid.0]})));
            }
        }
    }

    // Persist drag state in egui memory
    ui.data_mut(|d| {
        if let Some(st) = drag {
            d.insert_temp(drag_id, st);
        } else {
            d.remove::<GizmoDragState>(drag_id);
        }
    });

    // Execute queued command actions
    for (cmd, p) in actions {
        if let Err(e) = app.session.execute(&cmd, p) {
            app.ui.status = e.to_string();
        }
    }
}
