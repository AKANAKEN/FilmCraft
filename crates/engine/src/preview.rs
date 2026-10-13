//! Atomic Program-monitor duplication and synchronized text positioning.

use filmcraft_geom::Vec2;
use filmcraft_project::graphic::{LayerContent, eval_layer, layer_indices};
use filmcraft_project::{ClipId, ItemKind, Param, ParamValue, Track, TrackId, TrackKind};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session, commands};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Element {
    pub clip: ClipId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<usize>,
}

/// Explicit preview selection is invalidated when another selection command changes clips.
pub fn selected(s: &Session) -> Vec<Element> {
    let all = &s.state.preview_elements;
    if all.is_empty() || s.state.selection.iter().any(|c| !all.iter().any(|e| e.clip == *c)) || all.iter().any(|e| !s.state.selection.contains(&e.clip)) {
        return Vec::new();
    }
    all.iter().copied().filter(|e| element_effect(s, *e, s.playhead()).is_ok()).collect()
}

fn element_effect(s: &Session, element: Element, time: Tick) -> Result<Option<usize>> {
    let bad = |why| commands::bad("preview.select", why);
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (track, it) = q.find_item(element.clip).ok_or_else(|| bad("no such clip"))?;
    let tr = q.track(track).ok_or_else(|| bad("no such track"))?;
    if tr.kind != TrackKind::Video || tr.locked || !tr.enabled || !it.enabled || !it.range().contains(time) {
        return Err(bad("element must be visible on an unlocked video track"));
    }
    if !it.effect("motion").is_some_and(|e| e.enabled) {
        return Err(bad("Motion is disabled"));
    }
    match s.project.item(it.item).map(|i| &i.kind) {
        Some(ItemKind::Graphic { width, height, .. }) => {
            let ei = element.layer.and_then(|n| layer_indices(&it.effects).get(n).copied()).ok_or_else(|| bad("invalid graphic layer"))?;
            if !it.effects.get(ei).and_then(|e| eval_layer(e, it.source_time_at(time), (*width, *height))).is_some_and(|sp| sp.enabled) {
                return Err(bad("layer is hidden"));
            }
            Ok(Some(ei))
        }
        Some(ItemKind::AdjustmentLayer { .. }) | None => Err(bad("unsupported preview element")),
        _ if element.layer.is_none() && s.project.source_size(it.item).is_some() => Ok(None),
        _ => Err(bad("invalid media element")),
    }
}

pub fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let element: Element = serde_json::from_value(p.clone()).map_err(|_| commands::bad("preview.select", "need clip and optional layer index"))?;
    element_effect(s, element, s.playhead())?;
    let add = p.get("add").and_then(Value::as_bool).unwrap_or(false);
    let mut elements = selected(s);
    if add && elements.is_empty() {
        // Seed Shift-click from the existing single/timeline selection, only visible elements.
        for c in &s.state.selection {
            let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
            let graphic = q.find_item(*c).and_then(|(_, it)| s.project.item(it.item)).is_some_and(|i| matches!(i.kind, ItemKind::Graphic { .. }));
            if graphic && s.state.graphic_layers.len() > 1024 {
                return Err(commands::bad("preview.select", "at most 1024 elements"));
            }
            let layers = if graphic { s.state.graphic_layers.iter().copied().map(Some).collect::<Vec<_>>() } else { vec![None] };
            for layer in layers {
                let e = Element { clip: *c, layer };
                if element_effect(s, e, s.playhead()).is_ok() && !elements.contains(&e) {
                    if elements.len() >= 1024 {
                        return Err(commands::bad("preview.select", "at most 1024 elements"));
                    }
                    elements.push(e);
                }
            }
        }
    }
    if !add && !(p.get("retain").and_then(Value::as_bool).unwrap_or(false) && elements.contains(&element)) {
        elements.clear();
    }
    if !elements.contains(&element) {
        if elements.len() >= 1024 {
            return Err(commands::bad("preview.select", "at most 1024 elements"));
        }
        elements.push(element);
    }
    s.state.selection.clear();
    for e in &elements {
        if !s.state.selection.contains(&e.clip) {
            s.state.selection.push(e.clip);
        }
    }
    let graphic_clip = elements.iter().find(|e| e.layer.is_some()).map(|e| e.clip);
    s.state.graphic_layers = elements.iter().filter(|e| Some(e.clip) == graphic_clip).filter_map(|e| e.layer).collect();
    s.state.preview_elements = elements;
    s.state.selected_mask = None;
    s.state.edit_points.clear();
    s.state.transition_selection.clear();
    s.state.gap_selection = None;
    s.state.caption_selection.clear();
    Ok(json!({"elements":s.state.preview_elements}))
}

/// All absolute positions are supplied from a gesture's initial geometry, avoiding drift.
pub fn translate(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |why| commands::bad("preview.translate", why);
    let moves = p.get("moves").and_then(Value::as_array).filter(|a| !a.is_empty() && a.len() <= 1024).ok_or_else(|| bad("need 1..1024 moves"))?;
    let time = commands::time_p(s, p, "");
    if time.is_none() && ["time", "frame", "seconds", "timecode"].iter().any(|k| p.get(*k).is_some()) {
        return Err(bad("invalid time"));
    }
    let time = time.unwrap_or(s.playhead());
    let mut validated = Vec::new();
    for m in moves {
        let element: Element = serde_json::from_value(m.clone()).map_err(|_| bad("invalid element"))?;
        if validated.iter().any(|(e, _, _)| *e == element) {
            return Err(bad("duplicate move target"));
        }
        let a = m.get("position").and_then(Value::as_array).filter(|a| a.len() == 2).ok_or_else(|| bad("need position [x,y]"))?;
        let coord = |n| a.get(n).and_then(Value::as_f64).filter(|v| v.is_finite() && v.abs() <= 1e9);
        let (Some(x), Some(y)) = (coord(0), coord(1)) else { return Err(bad("invalid position")) };
        let ei = element_effect(s, element, time)?;
        validated.push((element, ei, Vec2::new(x, y)));
    }
    if p.get("begin").and_then(Value::as_bool).unwrap_or(false) {
        s.history.merge_key = None;
    }
    let merge = p.get("merge").and_then(Value::as_bool).unwrap_or(false).then_some("preview.translate");
    s.edit_sequence_as("Move Preview Group", merge, |q, _, _| {
        for (e, ei, position) in &validated {
            set_position(q, e.clip, *ei, time, *position)?;
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

pub fn move_clip(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |why| commands::bad("preview.move", why);
    let clip = commands::clip_p(p, "clip").ok_or_else(|| bad("need clip"))?;
    let pos = p.get("position").and_then(Value::as_array).filter(|a| a.len() == 2).ok_or_else(|| bad("need position [x,y]"))?;
    let coordinate = |n| pos.get(n).and_then(Value::as_f64).filter(|v| v.is_finite() && v.abs() <= 1e9);
    let (Some(x), Some(y)) = (coordinate(0), coordinate(1)) else { return Err(bad("invalid position")) };
    let position = Vec2::new(x, y);
    let time = commands::time_p(s, p, "");
    if time.is_none() && ["time", "frame", "seconds", "timecode"].iter().any(|k| p.get(*k).is_some()) {
        return Err(bad("invalid time"));
    }
    let time = time.unwrap_or(s.playhead());
    let duplicate = p.get("duplicate").and_then(Value::as_bool).unwrap_or(false);
    let layer = match p.get("layer") {
        Some(v) => Some(v.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("invalid layer"))?),
        None => None,
    };
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (track, original) = q.find_item(clip).ok_or_else(|| bad("no such clip"))?;
    let index = q.video_tracks.iter().position(|t| t.id == track && !t.locked && t.enabled).ok_or_else(|| bad("need an unlocked video track"))?;
    let size = s.project.source_size(original.item).ok_or_else(|| bad("no video source"))?;
    if !original.enabled || !original.effect("motion").is_some_and(|e| e.enabled) {
        return Err(bad("clip or Motion is disabled"));
    }
    let graphic = matches!(s.project.item(original.item).map(|i| &i.kind), Some(ItemKind::Graphic { .. }));
    let indices = layer_indices(&original.effects);
    let ei = if graphic {
        Some(*layer.and_then(|n| indices.get(n)).ok_or_else(|| bad("need a valid graphic layer"))?)
    } else {
        if layer.is_some() || matches!(s.project.item(original.item).map(|i| &i.kind), Some(ItemKind::AdjustmentLayer { .. })) {
            return Err(bad("unsupported preview element"));
        }
        None
    };
    let mut targets = Vec::new();
    if !duplicate && ei.is_some() {
        let mt = original.source_time_at(time.clamp(original.start, (original.end() - Tick(1)).max(original.start)));
        let frame_position = filmcraft_render::motion_matrix(q, original, size, None, mt).apply(position);
        if let Some(v) = p.get("texts") {
            let ids = v
                .as_array()
                .filter(|a| a.len() <= crate::graphics::MAX_TIMELINE_TEXT_CLIPS)
                .ok_or_else(|| commands::bad("preview.move", format!("texts must be at most {} clip ids", crate::graphics::MAX_TIMELINE_TEXT_CLIPS)))?;
            for id in ids {
                let id = ClipId(id.as_u64().ok_or_else(|| bad("invalid text clip id"))?);
                if id == clip || targets.iter().any(|(c, _, _)| *c == id) {
                    continue;
                }
                let (tr, it) = q.find_item(id).ok_or_else(|| bad("no such selected clip"))?;
                if q.track(tr).is_none_or(|t| t.locked || !t.enabled) {
                    continue;
                }
                let Some(ItemKind::Graphic { width, height, .. }) = s.project.item(it.item).map(|i| &i.kind) else { continue };
                let mt = it.source_time_at(time.clamp(it.start, (it.end() - Tick(1)).max(it.start)));
                let matrix = filmcraft_render::motion_matrix(q, it, (*width, *height), None, mt);
                let local = matrix.inverse().ok_or_else(|| bad("selected graphic has a singular transform"))?.apply(frame_position);
                if !local.x.is_finite() || !local.y.is_finite() || local.x.abs() > 1e9 || local.y.abs() > 1e9 {
                    return Err(bad("selected graphic position is out of range"));
                }
                let layers: Vec<usize> = layer_indices(&it.effects)
                    .into_iter()
                    .filter(|&n| {
                        it.effects.get(n).and_then(|e| eval_layer(e, mt, (*width, *height))).is_some_and(|sp| matches!(sp.content, LayerContent::Text(_)))
                    })
                    .collect();
                targets.push((id, layers, local));
            }
        }
    }
    if duplicate && (q.video_tracks.len() >= crate::sequence_tools::MAX_TRACKS || s.project.next_id.checked_add(2).is_none()) {
        return Err(bad("cannot allocate another track"));
    }
    let mut original = original.clone();
    let gesture = p.get("gesture").map(|v| v.as_u64().ok_or_else(|| bad("invalid gesture"))).transpose()?.unwrap_or(clip.0);
    let merge = p.get("merge").and_then(Value::as_bool).unwrap_or(false).then(|| format!("preview.move:{gesture}"));
    if p.get("begin").and_then(Value::as_bool).unwrap_or(false) {
        s.history.merge_key = None;
    }
    // The label must stay identical across the initial duplication and subsequent moves,
    // because history coalescing checks both the gesture key and the label.
    let label = "Move or Duplicate Preview Element";
    let result = s.edit_sequence_as(label, merge.as_deref(), |q, ctx, state| {
        let (id, effect_index) = if duplicate {
            original.id = ClipId(ctx.alloc());
            original.link = None;
            original.group = None;
            // A graphic clip can contain several layers: duplicate only the grabbed one,
            // while retaining intrinsic Motion/Opacity and the clip's other effects.
            if let Some(ei) = ei {
                original.effects = original.effects.into_iter().enumerate().filter(|(n, _)| !indices.contains(n) || *n == ei).map(|(_, e)| e).collect();
            }
            let effect_index = ei.and_then(|_| layer_indices(&original.effects).first().copied());
            let id = original.id;
            let mut tr = Track::new(TrackId(ctx.alloc()), TrackKind::Video, format!("{} copy", original.name));
            tr.items.push(original);
            q.video_tracks.insert(index + 1, tr);
            state.selection = vec![id];
            state.preview_elements.clear();
            state.graphic_layers = if graphic { vec![0] } else { Vec::new() };
            (id, effect_index)
        } else {
            (clip, ei)
        };
        set_position(q, id, effect_index, time, position)?;
        for (id, layers, pos) in &targets {
            for ei in layers {
                set_position(q, *id, Some(*ei), time, *pos)?;
            }
        }
        Ok(id)
    })?;
    Ok(json!({"clip": result.0, "layer": if duplicate { 0 } else { layer.unwrap_or(0) }}))
}

fn set_position(q: &mut filmcraft_project::Sequence, clip: ClipId, layer: Option<usize>, time: Tick, position: Vec2) -> Result<()> {
    let (_, it) = q.find_item_mut(clip).ok_or(EngineError::NoSequence)?;
    let mt = it.source_time_at(time.clamp(it.start, (it.end() - Tick(1)).max(it.start)));
    let effect = match layer {
        Some(n) => it.effects.get_mut(n),
        None => it.effects.iter_mut().find(|e| e.effect == "motion"),
    }
    .ok_or_else(|| commands::bad("preview.move", "no position effect"))?;
    effect.params.entry("position".into()).or_insert_with(|| Param::new(ParamValue::Vec2(position))).set_at(mt, ParamValue::Vec2(position));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Session {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        s
    }

    #[test]
    fn duplicate_retains_timing_style_and_original_and_merges_the_entire_gesture() {
        let mut s = demo();
        let text =
            s.execute("graphics.newText", json!({"text":"Copy me","position":[300,250],"size":87,"seconds":3,"time":0})).unwrap()["clip"].as_u64().unwrap();
        s.execute("graphics.newText", json!({"clip":text,"text":"Other internal layer"})).unwrap();
        let before = s.project.clone();
        let q = s.active_sequence().unwrap();
        let (track, original) = q.find_item(ClipId(text)).unwrap();
        let original = original.clone();
        let index = q.video_tracks.iter().position(|t| t.id == track).unwrap();
        let n = s.history.undo.len();
        let copy = s
            .execute("preview.move", json!({"clip":text,"layer":0,"position":[600,350],"duplicate":true,"merge":true,"begin":true,"gesture":text}))
            .unwrap()["clip"]
            .as_u64()
            .unwrap();
        s.execute("preview.move", json!({"clip":copy,"layer":0,"position":[650,400],"merge":true,"gesture":text})).unwrap();
        let q = s.active_sequence().unwrap();
        assert_eq!(q.find_item(ClipId(text)).unwrap().1, &original);
        let (track, it) = q.find_item(ClipId(copy)).unwrap();
        assert_eq!(q.video_tracks[index + 1].id, track);
        assert_eq!((it.start, it.duration, it.source_in, it.speed), (original.start, original.duration, original.source_in, original.speed));
        let layers = layer_indices(&it.effects);
        assert_eq!(layers.len(), 1);
        assert_eq!(it.effects[layers[0]].param("size"), original.effects[layer_indices(&original.effects)[0]].param("size"));
        assert_eq!(s.history.undo.len(), n + 1);
        let after = s.project.clone();
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.project, after);

        let movie = s.active_sequence().unwrap().video_tracks[0].items[0].id;
        let original = s.active_sequence().unwrap().find_item(movie).unwrap().1.clone();
        let copy = s.execute("preview.move", json!({"clip":movie.0,"position":[850,700],"duplicate":true})).unwrap()["clip"].as_u64().unwrap();
        let copied = s.active_sequence().unwrap().find_item(ClipId(copy)).unwrap().1;
        assert_eq!((copied.item, copied.start, copied.duration, copied.source_in), (original.item, original.start, original.duration, original.source_in));
        assert_eq!(copied.effect("motion").unwrap().param("scale"), original.effect("motion").unwrap().param("scale"));
        assert_eq!(copied.link, None);
        assert_eq!(s.active_sequence().unwrap().find_item(movie).unwrap().1, &original);
    }

    #[test]
    fn nonconcurrent_selected_texts_share_position_and_bad_input_is_atomic() {
        let mut s = demo();
        let a =
            s.execute("graphics.newText", json!({"newClip":true,"text":"First","position":[300,300],"time":0,"seconds":2})).unwrap()["clip"].as_u64().unwrap();
        let b = s
            .execute("graphics.newText", json!({"newClip":true,"text":"Later","position":[700,700],"time":Tick::from_seconds_f64(5.0).0,"seconds":2}))
            .unwrap()["clip"]
            .as_u64()
            .unwrap();
        s.execute("preview.move", json!({"clip":a,"layer":0,"position":[900,400],"texts":[a,b],"time":0})).unwrap();
        for c in [a, b] {
            let layers = s.execute("graphics.list", json!({"clip":c})).unwrap();
            assert_eq!(layers["layers"][0]["position"], json!([900.0, 400.0]));
        }
        for p in [
            json!({"clip":a,"layer":0,"position":[1e30,0]}),
            json!({"clip":a,"layer":0,"position":[1,2],"texts":[u64::MAX]}),
            json!({"clip":a,"layer":u64::MAX,"position":[1,2],"duplicate":true}),
            json!({"clip":a,"layer":0,"position":[1,2],"time":"invalid"}),
        ] {
            let before = s.project.clone();
            let history = s.history.undo.len();
            assert!(s.execute("preview.move", p).is_err());
            assert_eq!(s.project, before);
            assert_eq!(s.history.undo.len(), history);
        }
        let track = s.active_sequence().unwrap().find_item(ClipId(a)).unwrap().0;
        s.execute("timeline.setTrack", json!({"track":track.0,"locked":true})).unwrap();
        let before = s.project.clone();
        assert!(s.execute("preview.move", json!({"clip":a,"layer":0,"position":[1,2],"duplicate":true})).is_err());
        assert_eq!(s.project, before);
    }

    #[test]
    fn seeding_a_preview_group_caps_visible_timeline_elements_without_changing_selection() {
        let mut s = demo();
        let template = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
        s.set_playhead(template.start);
        let seq = s.state.active_sequence.unwrap();
        let project = std::sync::Arc::make_mut(&mut s.project);
        let mut ids = vec![template.id];
        for _ in 0..1024 {
            let mut item = template.clone();
            item.id = ClipId(project.alloc_id());
            ids.push(item.id);
            project.sequence_mut(seq).unwrap().video_tracks[0].items.push(item);
        }
        s.state.selection = ids.clone();
        s.state.preview_elements.clear();
        let before = s.project.clone();
        let history = s.history.undo.len();
        assert!(s.execute("preview.select", json!({"clip":template.id.0,"add":true})).is_err());
        assert_eq!(s.state.selection, ids);
        assert!(s.state.preview_elements.is_empty());
        assert_eq!(s.project, before);
        assert_eq!(s.history.undo.len(), history);
    }

    #[test]
    fn explicit_group_selection_and_atomic_translation_validate_all_targets() {
        let mut s = demo();
        s.execute("playhead.set", json!({"seconds":2})).unwrap();
        let a = s.execute("graphics.newText", json!({"text":"A","position":[300,400],"time":0})).unwrap()["clip"].as_u64().unwrap();
        s.execute("graphics.newText", json!({"clip":a,"text":"B","position":[300,600]})).unwrap();
        s.execute("preview.select", json!({"clip":a,"layer":0})).unwrap();
        s.execute("preview.select", json!({"clip":a,"layer":1,"add":true})).unwrap();
        assert_eq!(selected(&s).len(), 2);
        assert_eq!(s.state.selection, vec![ClipId(a)]);
        let before = s.project.clone();
        let n = s.history.undo.len();
        for i in 1..=3 {
            s.execute("preview.translate",json!({"moves":[{"clip":a,"layer":0,"position":[300+i*10,400+i*10]},{"clip":a,"layer":1,"position":[300+i*10,600+i*10]}],"merge":true,"begin":i==1})).unwrap();
        }
        let layers = s.execute("graphics.list", json!({"clip":a})).unwrap();
        assert_eq!(layers["layers"][0]["position"], json!([330.0, 430.0]));
        assert_eq!(layers["layers"][1]["position"], json!([330.0, 630.0]));
        assert_eq!(s.history.undo.len(), n + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        for p in [
            json!({"moves":[]}),
            json!({"moves":[{"clip":a,"layer":0,"position":[350,450]},{"clip":u64::MAX,"position":[1,2]}]}),
            json!({"moves":[{"clip":a,"layer":0,"position":[1e30,0]}]}),
            json!({"moves":[{"clip":a,"layer":0,"position":[350,450]}],"time":"invalid"}),
        ] {
            assert!(s.execute("preview.translate", p).is_err());
            assert_eq!(s.project, before);
        }
        for p in [json!({"clip":a,"layer":u64::MAX,"add":true}), json!({"clip":u64::MAX}), json!({"clip":a,"layer":"invalid"})] {
            assert!(s.execute("preview.select", p).is_err());
            assert_eq!(selected(&s).len(), 2);
        }
        s.execute("timeline.select", json!({"clips":[a]})).unwrap();
        assert!(selected(&s).is_empty());
        let tr = s.active_sequence().unwrap().find_item(ClipId(a)).unwrap().0;
        s.execute("timeline.setTrack", json!({"track":tr.0,"locked":true})).unwrap();
        assert!(s.execute("preview.translate", json!({"moves":[{"clip":a,"layer":0,"position":[1,2]}]})).is_err());
    }
}
