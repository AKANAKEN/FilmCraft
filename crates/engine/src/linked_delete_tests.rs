//! Premiere's Alt-click: one part of a linked clip (its sound on A1, its music on A2) selected on
//! its own, and Delete / Ripple Delete / Cut / Enable then act on that part only. A plain click
//! selects the whole linked clip, and those edits act on all of it, as before.

use filmcraft_project::{ClipId, TrackKind};
use serde_json::json;

use crate::Session;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.state.selection.clear();
    s
}

/// The first picture clip with a linked sound clip, and that sound clip.
fn linked_pair(s: &Session) -> (ClipId, ClipId) {
    let q = s.active_sequence().unwrap();
    for v in &q.video_tracks[0].items {
        let Some(l) = v.link else { continue };
        if let Some(a) = q.audio_tracks.iter().flat_map(|t| t.items.iter()).find(|a| a.link == Some(l)) {
            return (v.id, a.id);
        }
    }
    panic!("the demo has no linked picture and sound");
}

fn exists(s: &Session, c: ClipId) -> bool {
    s.active_sequence().unwrap().find_item(c).is_some()
}

#[test]
fn alt_selected_sound_is_deleted_alone() {
    let mut s = demo();
    let (video, audio) = linked_pair(&s);
    let r = s.execute("timeline.select", json!({"clips": [audio.0], "linked": false})).unwrap();
    assert_eq!(r["selection"], json!([audio.0]), "only the sound is selected");
    s.execute("edit.clear", json!({})).unwrap();
    assert!(!exists(&s, audio), "the sound is gone");
    assert!(exists(&s, video), "the picture stays");
    // one undo brings it back
    s.execute("edit.undo", json!({})).unwrap();
    assert!(exists(&s, audio));
}

#[test]
fn a_plain_click_still_deletes_the_whole_linked_clip() {
    let mut s = demo();
    let (video, audio) = linked_pair(&s);
    s.execute("timeline.select", json!({"clips": [audio.0]})).unwrap();
    assert_eq!(s.state.selection.len(), 2, "a click takes the linked picture along");
    s.execute("edit.clear", json!({})).unwrap();
    assert!(!exists(&s, audio) && !exists(&s, video));
}

#[test]
fn ripple_delete_of_an_alt_selected_part_leaves_the_picture() {
    let mut s = demo();
    let (video, audio) = linked_pair(&s);
    let picture = |s: &Session| s.active_sequence().unwrap().find_item(video).map(|(_, i)| (i.start, i.end()));
    let before = picture(&s);
    s.execute("timeline.select", json!({"clips": [audio.0], "linked": false})).unwrap();
    // with V1 sync-locked, closing the gap on A1 alone would pull the sound out of sync: refused
    let e = s.execute("edit.rippleDelete", json!({})).unwrap_err().to_string();
    assert!(e.contains("V1"), "{e}");
    assert!(exists(&s, audio));
    // sync lock off on the other tracks: the gap on the sound's track closes on its own
    let (own, _) = s.active_sequence().unwrap().find_item(audio).unwrap();
    let others: Vec<u64> = s.active_sequence().unwrap().all_tracks().map(|t| t.id).filter(|t| *t != own).map(|t| t.0).collect();
    for t in others {
        s.execute("timeline.setTrack", json!({"track": t, "syncLock": false})).unwrap();
    }
    s.execute("timeline.select", json!({"clips": [audio.0], "linked": false})).unwrap();
    s.execute("edit.rippleDelete", json!({})).unwrap();
    assert!(!exists(&s, audio));
    assert_eq!(picture(&s), before, "the picture is neither deleted nor moved");
}

#[test]
fn cut_copy_and_enable_take_only_the_alt_selected_part() {
    let mut s = demo();
    let (video, audio) = linked_pair(&s);
    s.execute("timeline.select", json!({"clips": [audio.0], "linked": false})).unwrap();
    s.execute("clip.enable", json!({})).unwrap();
    let enabled = |s: &Session, c: ClipId| s.active_sequence().unwrap().find_item(c).map(|(_, i)| i.enabled);
    assert_eq!((enabled(&s, video), enabled(&s, audio)), (Some(true), Some(false)), "only the sound is disabled");
    s.execute("edit.copy", json!({})).unwrap();
    assert_eq!(s.state.clipboard.len(), 1, "only the sound is copied");
    assert_eq!(s.state.clipboard[0].0, TrackKind::Audio);
    s.execute("edit.cut", json!({})).unwrap();
    assert!(!exists(&s, audio) && exists(&s, video));
}

/// Named targets (agents) are what a click on them selects: linked partners included while
/// Linked Selection is on, unless `linked: false`.
#[test]
fn named_clips_take_their_partners_unless_told_not_to() {
    let mut s = demo();
    let (video, audio) = linked_pair(&s);
    s.execute("edit.clear", json!({"clips": [audio.0], "linked": false})).unwrap();
    assert!(!exists(&s, audio) && exists(&s, video));
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.clear", json!({"clips": [audio.0]})).unwrap();
    assert!(!exists(&s, audio) && !exists(&s, video));
}
