//! Shift-click selection and relative group translation in Program.

use egui::{Pos2, Rect, Response, Stroke};
use filmcraft_engine::preview::{Element, selected};
use filmcraft_geom::{Affine, Vec2};
use filmcraft_time::Tick;
use serde_json::json;

use super::graphics::LayerView;
use crate::{FilmcraftApp, state::Tool};

pub(crate) fn shift_click(ui: &egui::Ui) -> bool {
    // Read the click's modifiers: synthetic clicks release modifier keys in the same
    // frame as the mouse, so the frame's final keyboard state is already unshifted.
    ui.input(|i| {
        i.events
            .iter()
            .rev()
            .find_map(|e| match e {
                egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, modifiers, .. } => Some(modifiers.shift),
                _ => None,
            })
            .unwrap_or(i.modifiers.shift)
    })
}

pub(crate) fn choose(app: &mut FilmcraftApp, element: Element, add: bool) {
    app.ui.gfx_edit = None;
    if let Err(e) = app.session.execute("preview.select", json!({"clip":element.clip.0,"layer":element.layer,"add":add,"retain":!add})) {
        app.ui.status = e.to_string();
    }
}

#[derive(Clone)]
struct Target {
    element: Element,
    position: Vec2,
    screen_to_canvas: Affine,
    quad: [Pos2; 4],
}

#[derive(Clone)]
struct Drag {
    start: Pos2,
    targets: Vec<Target>,
    time: Tick,
    begun: bool,
    last: Option<egui::Vec2>,
    bounds: Rect,
}

pub(crate) fn monitor_group(app: &mut FilmcraftApp, ui: &mut egui::Ui, pic: Rect, frame: (u32, u32), response: &Response, graphics: &[LayerView]) -> bool {
    let id = egui::Id::new("preview-group-drag");
    if app.ui.tool != Tool::Selection || app.playback.playing || app.session.state.selected_mask.is_some() || app.ui.mask_pen.is_some() {
        ui.data_mut(|d| d.remove::<Drag>(id));
        return false;
    }
    let elements = selected(&app.session);
    let targets: Vec<Target> = elements
        .iter()
        .filter_map(|e| {
            let (position, matrix, quad) = if let Some(layer) = e.layer {
                let v = graphics.iter().find(|v| v.clip == e.clip && v.layer == layer)?;
                (v.spec.transform.position, v.canvas_to_screen.inverse()?, v.quad())
            } else {
                super::media_transform::selection_geometry(app, pic, frame, e.clip)?
            };
            Some(Target { element: *e, position, screen_to_canvas: Affine { e: 0.0, f: 0.0, ..matrix }, quad })
        })
        .collect();
    if targets.len() < 2 {
        ui.data_mut(|d| d.remove::<Drag>(id));
        return false;
    }
    let painter = ui.painter().with_clip_rect(pic);
    let mut bounds = Rect::NOTHING;
    for target in &targets {
        painter.add(egui::Shape::closed_line(target.quad.to_vec(), Stroke::new(1.0, app.tokens.accent)));
        bounds = bounds.union(Rect::from_points(&target.quad));
    }
    let mut drag: Option<Drag> = ui.data(|d| d.get_temp(id));
    if response.drag_started() && !ui.input(|i| i.modifiers.alt) {
        let start = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos());
        if let Some(start) = start {
            let hit = targets.iter().any(|target| {
                if let Some(layer) = target.element.layer {
                    graphics.iter().any(|v| v.clip == target.element.clip && v.layer == layer && v.hit(start))
                } else {
                    super::media_transform::body_hit(app, pic, frame, target.element.clip, start)
                }
            });
            // Only a selected frontmost element owns a group gesture. An unselected element
            // in front is allowed to replace the selection through the ordinary overlays.
            let front = super::media_transform::front_element(app, pic, frame, graphics, start);
            if hit && front.is_some_and(|e| elements.contains(&e)) {
                drag = Some(Drag { start, targets, time: app.session.playhead(), begun: false, last: None, bounds });
            }
        }
    }
    let handled = drag.is_some();
    if let Some(d) = &mut drag {
        if d.time != app.session.playhead() || d.targets.iter().any(|t| !elements.contains(&t.element)) {
            drag = None;
        } else if let Some(current) = response.interact_pointer_pos() {
            let (off, lines) = if ui.input(|i| i.modifiers.command) {
                (current - d.start, Vec::new())
            } else {
                super::monitor_view::snap_move(app, pic, frame, d.bounds, current - d.start)
            };
            super::monitor_view::draw_snap_lines(&painter, pic, &lines);
            if (d.begun || off.length() > 2.0) && d.last != Some(off) {
                let moves: Vec<_> = d
                    .targets
                    .iter()
                    .map(|t| {
                        let pos = t.position + t.screen_to_canvas.apply(Vec2::new(off.x as f64, off.y as f64));
                        let mut value = json!({"clip":t.element.clip.0,"position":[pos.x,pos.y]});
                        if let Some(layer) = t.element.layer {
                            value["layer"] = json!(layer);
                        }
                        value
                    })
                    .collect();
                match app.session.execute("preview.translate", json!({"moves":moves,"time":d.time.0,"merge":true,"begin":!d.begun})) {
                    Ok(_) => {
                        d.begun = true;
                        d.last = Some(off);
                    }
                    Err(e) => {
                        app.ui.status = e.to_string();
                        drag = None;
                    }
                }
                ui.ctx().request_repaint();
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
    handled
}
