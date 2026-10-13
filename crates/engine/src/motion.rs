//! Atomic Motion edits: moving and resizing a clip is one undoable gesture.

use filmcraft_geom::Vec2;
use filmcraft_project::{Param, ParamValue, TrackKind};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::{Result, Session, commands};

pub fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "motion.set";
    let bad = |why| commands::bad(CMD, why);
    let clip = commands::clip_p(p, "clip").ok_or_else(|| bad("need `clip`"))?;
    let mut values = Vec::new();
    if let Some(v) = p.get("position") {
        let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad("position must be [x,y]"))?;
        let coordinate = |i| a.get(i).and_then(Value::as_f64).filter(|v| v.is_finite() && v.abs() <= 1e9);
        let (Some(x), Some(y)) = (coordinate(0), coordinate(1)) else { return Err(bad("position must contain finite coordinates within ±1e9")) };
        values.push(("position", ParamValue::Vec2(Vec2::new(x, y))));
    }
    for (key, id) in [("scale", "scale"), ("scaleWidth", "scale_width")] {
        if let Some(v) = p.get(key) {
            let v = v.as_f64().filter(|v| v.is_finite() && (0.0..=10000.0).contains(v)).ok_or_else(|| bad("scale must be finite and within 0..10000"))?;
            values.push((id, ParamValue::Float(v)));
        }
    }
    if values.is_empty() {
        return Err(bad("need position, scale or scaleWidth"));
    }
    let seq = s.active_sequence().ok_or_else(|| bad("no sequence is open"))?;
    let (track, it) = seq.find_item(clip).ok_or_else(|| bad("no such clip"))?;
    let track = seq.track(track).ok_or_else(|| bad("no such track"))?;
    if track.locked || track.kind != TrackKind::Video {
        return Err(bad("clip must be on an unlocked video track"));
    }
    if s.project.source_size(it.item).is_none() {
        return Err(bad("clip has no video source"));
    }
    if it.effect("motion").is_none() {
        return Err(bad("clip has no Motion effect"));
    }
    let requested_time = commands::time_p(s, p, "");
    if requested_time.is_none() && ["time", "frame", "seconds", "timecode"].iter().any(|k| p.get(*k).is_some()) {
        return Err(bad("invalid timeline time"));
    }
    let time = requested_time.unwrap_or(s.playhead());
    let merge = p.get("merge").and_then(Value::as_bool).unwrap_or(false).then(|| format!("motion:{}", clip.0));
    if p.get("begin").and_then(Value::as_bool).unwrap_or(false) {
        s.history.merge_key = None;
    }
    s.edit_sequence_as("Transform Clip", merge.as_deref(), |q, _, _| {
        let (_, it) = q.find_item_mut(clip).ok_or_else(|| bad("no such clip"))?;
        let mt = it.source_time_at(time.clamp(it.start, (it.end() - Tick(1)).max(it.start)));
        let effect = it.effects.iter_mut().find(|e| e.effect == "motion").ok_or_else(|| bad("clip has no Motion effect"))?;
        effect.enabled = true;
        for (id, value) in &values {
            effect.params.entry((*id).into()).or_insert_with(|| Param::new(value.clone())).set_at(mt, value.clone());
        }
        Ok(())
    })?;
    Ok(json!({"clip": clip.0}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> (Session, u64) {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id.0;
        (s, clip)
    }

    #[test]
    fn combined_transform_is_one_undo_and_gestures_do_not_merge() {
        let (mut s, clip) = demo();
        let before = s.project.clone();
        let n = s.history.undo.len();
        for (i, scale) in [90, 80, 70].into_iter().enumerate() {
            s.execute("motion.set", json!({"clip":clip,"position":[700,400],"scale":scale,"scaleWidth":scale,"merge":true,"begin":i==0})).unwrap();
        }
        assert_eq!(s.history.undo.len(), n + 1);
        let after = s.project.clone();
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.project, after);
        s.execute("motion.set", json!({"clip":clip,"position":[600,300],"merge":true,"begin":true})).unwrap();
        assert_eq!(s.history.undo.len(), n + 2);
    }

    #[test]
    fn hostile_parameters_and_locked_or_audio_tracks_leave_project_untouched() {
        let (mut s, clip) = demo();
        for p in [
            json!({"clip":clip}),
            json!({"clip":clip,"position":[1]}),
            json!({"clip":clip,"position":[1e30,0]}),
            json!({"clip":clip,"scale":-1}),
            json!({"clip":clip,"scale":"bad"}),
            json!({"clip":clip,"scaleWidth":10001}),
            json!({"clip":u64::MAX,"scale":80}),
        ] {
            let before = s.project.clone();
            let n = s.history.undo.len();
            assert!(s.execute("motion.set", p).is_err());
            assert_eq!(s.project, before);
            assert_eq!(s.history.undo.len(), n);
        }
        s.execute("timeline.setTrack", json!({"track":"V1","locked":true})).unwrap();
        let before = s.project.clone();
        assert!(s.execute("motion.set", json!({"clip":clip,"scale":80})).is_err());
        assert_eq!(s.project, before);
        let mut empty = Session::default();
        assert!(empty.execute("motion.set", json!({"clip":clip,"scale":80})).is_err());
    }

    #[test]
    fn animated_motion_edits_only_the_requested_media_time() {
        let (mut s, clip) = demo();
        s.execute("effects.setParam", json!({"clip":clip,"param":"scale","value":100,"keyframe":true,"seconds":0})).unwrap();
        s.execute("effects.setParam", json!({"clip":clip,"param":"scale","value":50,"keyframe":true,"seconds":4})).unwrap();
        s.execute("motion.set", json!({"clip":clip,"scale":75,"seconds":2})).unwrap();
        let (_, it) = s.active_sequence().unwrap().find_item(filmcraft_project::ClipId(clip)).unwrap();
        let effect = it.effect("motion").unwrap();
        assert_eq!(effect.f64_at("scale", it.source_time_at(Tick::from_seconds_f64(0.0))), 100.0);
        assert_eq!(effect.f64_at("scale", it.source_time_at(Tick::from_seconds_f64(2.0))), 75.0);
        assert_eq!(effect.f64_at("scale", it.source_time_at(Tick::from_seconds_f64(4.0))), 50.0);
    }
}
