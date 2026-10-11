//! `edit.batch`: all steps or none, and one undo step for the whole batch.

use serde_json::{Value, json};

use crate::Session;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn sequence(s: &mut Session) -> Value {
    s.execute("sequence.inspect", json!({})).unwrap()
}

fn clips(s: &mut Session) -> usize {
    let q = sequence(s);
    ["video", "audio"].iter().flat_map(|k| q[k].as_array().unwrap().iter()).map(|t| t["items"].as_array().map_or(0, Vec::len)).sum()
}

fn undo_labels(s: &mut Session) -> Vec<String> {
    let h = s.execute("history.list", json!({})).unwrap();
    h["undo"].as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_string()).collect()
}

fn cuts(times: &[f64]) -> Value {
    Value::Array(times.iter().map(|t| json!({"id": "sequence.addEditAllTracks", "params": {"seconds": t}})).collect())
}

#[test]
fn a_batch_is_one_undo_step() {
    let mut s = demo();
    let (before, n0, undo0) = (sequence(&mut s), clips(&mut s), undo_labels(&mut s));
    let r = s.execute("edit.batch", json!({"steps": cuts(&[1.0, 2.0, 3.0]), "label": "Agent: three cuts"})).unwrap();
    assert_eq!(r["completed"], 3, "{r}");
    assert_eq!(r["undo"], "Agent: three cuts", "{r}");
    assert_eq!(r["results"].as_array().unwrap().len(), 3);
    assert!(clips(&mut s) > n0, "the cuts were made");
    let mut want = undo0.clone();
    want.push("Agent: three cuts".into());
    assert_eq!(undo_labels(&mut s), want, "one step for the whole batch");
    // one undo takes all of it back, one redo puts it all back
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(sequence(&mut s)["video"], before["video"]);
    assert_eq!(undo_labels(&mut s), undo0);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(clips(&mut s), n0 + r["results"].as_array().unwrap().iter().map(|c| c["cuts"].as_u64().unwrap() as usize).sum::<usize>());
}

#[test]
fn a_failing_step_changes_nothing() {
    let mut s = demo();
    s.execute("playhead.set", json!({"seconds": 0.5})).unwrap();
    s.saved_revision = s.revision; // as if just saved
    let (before, undo0, journal0, rev0) = (sequence(&mut s), undo_labels(&mut s), s.journal.len(), s.revision);
    let steps = json!([
        {"id": "sequence.addEditAllTracks", "params": {"seconds": 1.0}},
        {"id": "playhead.set", "params": {"seconds": 4.0}},
        {"id": "no.such.command"},
        {"id": "sequence.addEditAllTracks", "params": {"seconds": 2.0}},
    ]);
    let e = s.execute("edit.batch", json!({"steps": steps})).unwrap_err().to_string();
    assert!(e.contains("step 3 (`no.such.command`)") && e.contains("rolled back"), "{e}");
    assert_eq!(sequence(&mut s), before, "tracks, playhead and selection as they were");
    assert_eq!(undo_labels(&mut s), undo0);
    assert_eq!(s.journal.len(), journal0, "rolled-back steps are not journaled");
    assert!(!s.is_dirty(), "a saved project stays saved");
    assert!(s.revision > rev0, "caches see a new revision");
    // the redo stack survives a failed batch
    s.execute("sequence.addEditAllTracks", json!({"seconds": 1.0})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    let _ = s.execute("edit.batch", json!({"steps": [{"id": "sequence.addEditAllTracks", "params": {"seconds": 99999.0}}, {"id": "nope"}]}));
    assert!(s.history.can_redo());
}

/// A disabled step (here: Undo with nothing to undo) fails the batch like any other error.
#[test]
fn a_disabled_step_fails_the_batch() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.history.undo.clear();
    let e = s.execute("edit.batch", json!({"steps": [{"id": "edit.undo"}]})).unwrap_err().to_string();
    assert!(e.contains("step 1 (`edit.undo`)") && e.contains("not available"), "{e}");
}

#[test]
fn queries_and_nothing_at_all_leave_the_history_alone() {
    let mut s = demo();
    let undo0 = undo_labels(&mut s);
    let r = s.execute("edit.batch", json!({"steps": [{"id": "sequence.inspect"}, {"id": "project.inspect"}]})).unwrap();
    assert_eq!((r["completed"].as_u64(), r["undo"].is_null()), (Some(2), true), "{r}");
    let r = s.execute("edit.batch", json!({"steps": []})).unwrap();
    assert_eq!(r["completed"], 0);
    assert_eq!(undo_labels(&mut s), undo0);
}

#[test]
fn batches_nest_and_outlast_the_history_limit() {
    let mut s = demo();
    s.history.limit = 3;
    let before = sequence(&mut s)["video"].clone();
    let inner = json!({"id": "edit.batch", "params": {"steps": cuts(&[1.0, 2.0]), "label": "inner"}});
    let mut steps = cuts(&[3.0, 4.0, 5.0, 6.0]).as_array().unwrap().clone();
    steps.insert(0, inner);
    s.execute("edit.batch", json!({"steps": steps, "label": "outer"})).unwrap();
    assert_eq!(s.history.limit, 3, "the limit is back");
    assert_eq!(undo_labels(&mut s).last().map(String::as_str), Some("outer"));
    assert!(s.history.undo.len() <= 3);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(sequence(&mut s)["video"], before, "six cuts in more steps than the history holds, undone at once");
    // a failure after a nested batch takes the nested one back too
    let e = s.execute("edit.batch", json!({"steps": [{"id": "edit.batch", "params": {"steps": cuts(&[1.0])}}, {"id": "nope"}]})).unwrap_err();
    assert!(e.to_string().contains("step 2"), "{e}");
    assert_eq!(sequence(&mut s)["video"], before);
}

/// Hostile parameters are errors, never a panic or a partial edit.
#[test]
fn hostile_parameters() {
    let mut s = demo();
    let before = sequence(&mut s);
    for p in [
        json!({}),
        json!({"steps": 3}),
        json!({"steps": [{"params": {}}]}),
        json!({"steps": [{"id": 7}]}),
        json!({"steps": [{"id": "playhead.set", "params": [1, 2]}]}),
        json!({"steps": [], "label": 5}),
        json!({"steps": [], "label": "  "}),
        json!({"steps": vec![json!({"id": "sequence.inspect"}); crate::batch::MAX_STEPS + 1]}),
    ] {
        let e = s.execute("edit.batch", p.clone());
        assert!(e.is_err(), "{p} was accepted");
    }
    assert_eq!(sequence(&mut s), before);
    // a very long label is cut short
    let r = s.execute("edit.batch", json!({"steps": cuts(&[1.0]), "label": "x".repeat(10_000)})).unwrap();
    assert_eq!(r["undo"].as_str().map(str::len), Some(200));
}

/// `history.step` (a click in the History panel) undoes or redoes several steps at once, and a
/// count beyond the history is not a long loop.
#[test]
fn history_step_jumps_like_the_history_panel() {
    let mut s = demo();
    let before = sequence(&mut s)["video"].clone();
    let base = s.history.undo.len();
    for t in [1.0, 2.0, 3.0] {
        s.execute("sequence.addEditAllTracks", json!({"seconds": t})).unwrap();
    }
    let r = s.execute("history.step", json!({"steps": -3})).unwrap();
    assert_eq!(r["undone"].as_array().unwrap().len(), 3, "{r}");
    assert_eq!(sequence(&mut s)["video"], before);
    let r = s.execute("history.step", json!({"steps": 2})).unwrap();
    assert_eq!(r["redone"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(s.history.undo.len(), base + 2);
    let r = s.execute("history.step", json!({"steps": i64::MIN})).unwrap();
    assert_eq!(r["undone"].as_array().unwrap().len(), base + 2, "{r}");
    assert!(s.execute("history.step", json!({"steps": "x"})).is_err());
    assert!(s.execute("history.step", json!({})).is_err());
}
