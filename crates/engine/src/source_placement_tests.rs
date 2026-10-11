use super::*;
use filmcraft_project::ItemId;
use filmcraft_time::{Tick, TimeRange};
use serde_json::json;

fn setup() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("file.newSequence", json!({"name":"Source test","width":1280,"height":720,"fps":24})).unwrap();
    s.execute("source.open", json!({"item":5})).unwrap();
    s
}
fn counts(s: &Session) -> (usize, usize) {
    let q = s.active_sequence().unwrap();
    (q.video_tracks.iter().map(|t| t.items.len()).sum(), q.audio_tracks.iter().map(|t| t.items.len()).sum())
}

#[test]
fn subframe_placement_keeps_exact_spans_and_undo_redo() {
    for insert in [false, true] {
        for (item, track, count) in [(11, "A2", 1), (5, "V1", 2)] {
            let mut s = setup();
            let before = s.project.clone();
            s.execute(
                "timeline.place",
                json!({"item":item,"track":track,"time":638_568_000_000_i64,
                       "sourceIn":254_016_000_000_i64,"duration":631_512_000_000_i64,"insert":insert}),
            )
            .unwrap();
            let q = s.active_sequence().unwrap();
            let clips: Vec<_> = q.all_tracks().flat_map(|t| t.items.iter()).collect();
            assert_eq!(clips.len(), count);
            for clip in &clips {
                assert_eq!(clip.start, Tick(638_568_000_000));
                assert_eq!(clip.duration, Tick(631_512_000_000));
                assert_eq!(clip.end(), Tick(1_270_080_000_000));
                assert_eq!(clip.source_in, Tick(254_016_000_000));
                assert_eq!(clip.source_out(), Tick(885_528_000_000));
            }
            if count == 2 {
                assert!(clips[0].link.is_some());
                assert_eq!(clips[0].link, clips[1].link);
            }
            q.check().unwrap();
            let after = s.project.clone();
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*s.project, *before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(*s.project, *after);
            assert_eq!(s.execute("sequence.inspect", json!({})).unwrap()["durationFrames"], 120);
        }
    }
}

#[test]
fn source_edit_keeps_frame_rounded_duration_and_playhead() {
    for command in ["source.insert", "source.overwrite"] {
        let mut s = setup();
        // Demo media is 24000/1001 fps: inclusive Out adds 10594584000 ticks.
        s.execute("project.setMarks", json!({"item":5,"in":0,"out":620_928_000_000_i64})).unwrap();
        s.set_playhead(Tick(638_568_000_000));
        s.execute(command, json!({})).unwrap();
        // Source edits keep their existing frame-rounded span and cursor.
        assert_eq!(s.playhead(), Tick(1_270_080_000_000));
        assert_eq!(s.active_sequence().unwrap().duration(), Tick(1_270_080_000_000));
    }
}

#[test]
fn explicit_placement_keeps_frame_minimum_and_implicit_rounding() {
    for (duration, expected) in [(None, 635_040_000_000_i64), (Some(1_i64), 10_584_000_000), (Some(635_040_000_000), 635_040_000_000)] {
        let mut s = setup();
        s.execute("project.setMarks", json!({"item":5,"in":0,"out":620_928_000_000_i64})).unwrap();
        let mut params = json!({"item":5,"track":"V1","time":0});
        if let Some(duration) = duration {
            params["duration"] = json!(duration);
        }
        s.execute("timeline.place", params).unwrap();
        let q = s.active_sequence().unwrap();
        for clip in q.all_tracks().flat_map(|t| t.items.iter()) {
            assert_eq!(clip.duration, Tick(expected), "requested duration {duration:?}");
        }
        q.check().unwrap();
    }
}

#[test]
fn unmarked_source_defaults_to_full_span_without_writing_marks() {
    let s = setup();
    let view = clip_ops::source_view(&s, ItemId(5)).unwrap();
    assert_eq!(view.selected_range(), TimeRange::from_bounds(view.start, view.end));
    assert!(view.mark_in.is_none() && view.mark_out.is_none());
}

#[test]
fn marked_range_places_video_audio_or_linked_both_as_one_undo_step() {
    for (video, audio, expected) in [(true, false, (1, 0)), (false, true, (0, 1)), (true, true, (1, 1))] {
        let mut s = setup();
        let start = Tick::from_seconds_f64(1.0);
        let out = Tick::from_seconds_f64(3.0);
        s.execute("project.setMarks", json!({"item":5,"in":start.0,"out":out.0})).unwrap();
        let view = clip_ops::source_view(&s, ItemId(5)).unwrap();
        let range = view.selected_range();
        assert_eq!(range.end(), out + view.rate.frame_duration());
        let before = s.project.to_json();
        s.execute(
            "timeline.place",
            json!({"item":5,"track":"V1","audioTrack":"A1","time":0,"sourceIn":range.start.0,"duration":range.duration.0,"video":video,"audio":audio}),
        )
        .unwrap();
        assert_eq!(counts(&s), expected);
        let q = s.active_sequence().unwrap();
        for clip in q.all_tracks().flat_map(|t| t.items.iter()) {
            assert_eq!(clip.source_in, start);
            // Two seconds plus the inclusive 24000/1001 fps source frame, without
            // rounding that source span to the 24 fps sequence grid.
            assert_eq!(clip.duration, Tick(518_626_584_000));
        }
        if video && audio {
            assert!(q.video_tracks[0].items[0].link.is_some());
            assert_eq!(q.video_tracks[0].items[0].link, q.audio_tracks[0].items[0].link);
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.project.to_json(), before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(counts(&s), expected);
    }
}

#[test]
fn invalid_stream_choices_and_locked_destinations_do_not_mutate_project() {
    let mut s = setup();
    let before = s.project.to_json();
    for params in [json!({"video":false,"audio":false}), json!({"video":"yes"}), json!({"audio":2})] {
        let mut p = json!({"item":5,"track":"V1","audioTrack":"A1","time":0});
        for (k, v) in params.as_object().unwrap() {
            p[k] = v.clone();
        }
        assert!(s.execute("timeline.place", p).is_err());
        assert_eq!(s.project.to_json(), before);
    }
    let id = s.state.active_sequence.unwrap();
    std::sync::Arc::make_mut(&mut s.project).sequence_mut(id).unwrap().audio_tracks[0].locked = true;
    let locked = s.project.to_json();
    assert!(s.execute("timeline.place", json!({"item":5,"track":"V1","audioTrack":"A1","time":0,"video":true,"audio":true})).is_err());
    assert_eq!(s.project.to_json(), locked);
}

#[test]
fn audio_only_source_drops_only_sound() {
    let mut s = setup();
    s.execute("timeline.place", json!({"item":11,"track":"A2","time":0,"video":false,"audio":true})).unwrap();
    assert_eq!(counts(&s), (0, 1));
    assert_eq!(s.active_sequence().unwrap().audio_tracks[1].items.len(), 1);
}

#[test]
fn malformed_source_spans_are_bounded() {
    let s = setup();
    let mut view = clip_ops::source_view(&s, ItemId(5)).unwrap();
    view.mark_in = Some(Tick::MAX);
    view.mark_out = Some(Tick::ZERO);
    let range = view.selected_range();
    assert_eq!(range.start, view.end);
    assert_eq!(range.duration, Tick::ZERO);
    view.end = Tick::ZERO;
    view.start = Tick::from_seconds_f64(1.0);
    assert_eq!(view.selected_range().duration, Tick::ZERO);
}
