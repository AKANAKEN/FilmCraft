//! `edit.batch`: several commands as one edit. Either every step succeeds and the whole batch is
//! one undo step (one ⌘Z takes an agent's whole change back), or a step fails and nothing changes:
//! the project, the editor state (selection, playhead, …), the undo history and the journal are put
//! back as they were before the first step.
//!
//! | command | does |
//! |---|---|
//! | `edit.batch {steps: [{id, params?}], label?}` | run the steps in order; all or nothing, one undo step named `label` |
//!
//! What a batch cannot take back: files a step wrote (exports, saves), preferences, background jobs
//! it started, and, in the desktop app, frontend state such as the tool, the workspace or playback.
//!
//! [`Session::begin_transaction`], [`Session::commit`] and [`Session::rollback`] are public so a
//! frontend can run the steps through its own dispatcher (the desktop app routes UI-only commands
//! such as `tool.razor` there) with the same guarantees.

use std::sync::Arc;

use filmcraft_project::Project;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad};
use crate::{EditorState, EngineError, Result, Session};

/// Steps one batch may hold (hostile input: a caller can't make the session loop for hours).
pub const MAX_STEPS: usize = 10_000;

/// Undo label of a batch that doesn't name itself.
pub const DEFAULT_LABEL: &str = "Batch Edit";

/// Everything a failed batch restores. Made by [`Session::begin_transaction`]; give it back to
/// [`Session::commit`] or [`Session::rollback`].
pub struct Transaction {
    project: Arc<Project>,
    state: EditorState,
    undo: Vec<(String, Arc<Project>)>,
    redo: Vec<(String, Arc<Project>)>,
    merge_key: Option<String>,
    limit: usize,
    journal: usize,
    path: Option<String>,
    was_clean: bool,
}

impl Session {
    /// Start a transaction: until [`Session::commit`] or [`Session::rollback`], undo steps are kept
    /// however many the steps push (none fall off the front of the history).
    pub fn begin_transaction(&mut self) -> Transaction {
        let t = Transaction {
            project: self.project.clone(),
            state: self.state.clone(),
            undo: self.history.undo.clone(),
            redo: self.history.redo.clone(),
            merge_key: self.history.merge_key.clone(),
            limit: self.history.limit,
            journal: self.journal.len(),
            path: self.path.clone(),
            was_clean: !self.is_dirty(),
        };
        self.history.limit = usize::MAX;
        t
    }

    /// Keep what the steps did, as one undo step named `label` (none when the project is unchanged).
    pub fn commit(&mut self, t: Transaction, label: &str) {
        self.history.limit = t.limit;
        if Arc::ptr_eq(&self.project, &t.project) {
            // nothing to undo: queries, UI commands, or edits that changed nothing
            self.history.undo = t.undo;
            self.history.redo = t.redo;
            self.history.merge_key = t.merge_key;
            return;
        }
        let mut undo = t.undo;
        undo.push((label.to_string(), t.project));
        let excess = undo.len().saturating_sub(self.history.limit.max(1));
        undo.drain(..excess);
        self.history.undo = undo;
        self.history.redo.clear();
        self.history.merge_key = None;
    }

    /// Undo everything the steps did: project, editor state, history and journal as they were.
    pub fn rollback(&mut self, t: Transaction) {
        let changed = !Arc::ptr_eq(&self.project, &t.project);
        self.project = t.project;
        self.state = t.state;
        self.history.undo = t.undo;
        self.history.redo = t.redo;
        self.history.merge_key = t.merge_key;
        self.history.limit = t.limit;
        // the journal may have been trimmed from the front meanwhile; then it is left as it is
        if self.journal.len() >= t.journal {
            self.journal.truncate(t.journal);
        }
        self.path = t.path;
        self.fix_state();
        if changed {
            // frame caches key on the revision, so the restored project gets a new one
            self.bump();
            if t.was_clean {
                self.saved_revision = self.revision;
            }
        }
    }
}

/// One step of a batch: a command id and its parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub id: String,
    pub params: Value,
}

/// The steps and undo label of `edit.batch` parameters.
pub fn parse(p: &Value) -> Result<(Vec<Step>, String)> {
    let steps = p.get("steps").and_then(Value::as_array).ok_or_else(|| bad("edit.batch", "need `steps`: [{\"id\": …, \"params\": {…}}]"))?;
    if steps.len() > MAX_STEPS {
        return Err(bad("edit.batch", format!("at most {MAX_STEPS} steps, got {}", steps.len())));
    }
    let mut out = Vec::with_capacity(steps.len());
    for (i, st) in steps.iter().enumerate() {
        let id = st.get("id").and_then(Value::as_str).ok_or_else(|| bad("edit.batch", format!("step {} has no `id`", i + 1)))?;
        let params = st.get("params").cloned().unwrap_or_else(|| json!({}));
        if !params.is_object() {
            return Err(bad("edit.batch", format!("step {} (`{id}`): `params` must be an object", i + 1)));
        }
        out.push(Step { id: id.to_string(), params });
    }
    let label = match p.get("label") {
        None | Some(Value::Null) => DEFAULT_LABEL.to_string(),
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().chars().take(200).collect(),
        Some(_) => return Err(bad("edit.batch", "`label` must be a non-empty string")),
    };
    Ok((out, label))
}

/// The error of a batch whose step `index` (from 0) failed; the batch has been rolled back.
pub fn failed(index: usize, id: &str, error: &str) -> EngineError {
    EngineError::Other(format!("step {} (`{id}`) failed: {error}; the batch was rolled back, nothing changed", index + 1))
}

/// Run `steps` through `run` as one transaction of `session(ctx)`. Returns
/// `{completed, results, undo}` (`undo`: the label of the undo step, null when nothing changed).
pub fn run_steps<C>(
    ctx: &mut C,
    session: fn(&mut C) -> &mut Session,
    steps: Vec<Step>,
    label: &str,
    mut run: impl FnMut(&mut C, &str, Value) -> std::result::Result<Value, String>,
) -> Result<Value> {
    let t = session(ctx).begin_transaction();
    let before = t.project.clone();
    let mut results = Vec::with_capacity(steps.len());
    for (i, st) in steps.into_iter().enumerate() {
        match run(ctx, &st.id, st.params) {
            Ok(v) => results.push(v),
            Err(e) => {
                session(ctx).rollback(t);
                return Err(failed(i, &st.id, &e));
            }
        }
    }
    let s = session(ctx);
    let changed = !Arc::ptr_eq(&s.project, &before);
    s.commit(t, label);
    Ok(json!({"completed": results.len(), "results": results, "undo": changed.then_some(label)}))
}

fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    let (steps, label) = parse(p)?;
    if s.trim_play.active() {
        s.settle_trim_playback("edit.batch");
    }
    run_steps(s, |s| s, steps, &label, |s, id, params| s.execute(id, params).map_err(|e| e.to_string()))
}

pub fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "edit.batch",
        label: "Batch Edit",
        menu: &[],
        shortcut: None,
        params: r#"{"steps":[{"id":"timeline.razor","params":{"seconds":2}}],"label":str?} all steps or none; one undo step"#,
        enabled: always,
        run: batch,
        // the steps journal themselves
        journal: false,
    }]
}
