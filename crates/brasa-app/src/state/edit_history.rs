//! Undo and redo for a request tab (#109): a linear history of [`Snapshot`]s of what the tab
//! edits, kept free of `gpui` so it is unit tested without a window.
//!
//! Every edit records the state it leaves behind ([`EditHistory::record`]). Typing in one field
//! is coalesced into a single step until the user pauses for [`TYPING_PAUSE`] or moves to another
//! field, so undo walks back word by word bursts instead of letter by letter. The history knows
//! which step was last saved, so undoing back to it makes the tab clean again
//! ([`EditHistory::is_dirty`]).
//!
//! Snapshots are whole copies, simple and always consistent. Bodies can be several megabytes
//! (the response limit is 20 MB and a request body can be as big), so the history keeps at most
//! [`MAX_BYTES`] of snapshots per tab, dropping the oldest steps first, and never fewer than the
//! current one.

use std::time::{Duration, Instant};

use brasa_core::{Auth, Body, KeyValue, Request};

use super::auth_edit::AuthStash;
use super::request_edit::BodyStash;

/// How long typing in one field may pause and still extend the same undo step.
pub const TYPING_PAUSE: Duration = Duration::from_secs(1);

/// The most snapshot bytes one tab keeps (an estimate, see [`Snapshot::approx_size`]).
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

/// Everything an undo restores: the request and what the other body and auth kinds hold for
/// when the user switches back to them.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// The request as edited.
    pub request: Request,
    /// What the other body kinds hold.
    pub body_stash: BodyStash,
    /// What the other auth kinds hold.
    pub auth_stash: AuthStash,
}

impl Snapshot {
    /// About how many bytes this snapshot holds: the length of its text, which is what can grow
    /// (a body, a script). Only used for the [`MAX_BYTES`] budget, so it does not need to be
    /// exact.
    pub fn approx_size(&self) -> usize {
        let request = &self.request;
        let rows = |rows: &[KeyValue]| -> usize {
            rows.iter()
                .map(|row| row.key.len() + row.value.len() + 16)
                .sum()
        };
        let body = match &request.body {
            Body::None => 0,
            Body::Json(text) | Body::Text(text) | Body::Xml(text) => text.len(),
            Body::Form(form) | Body::Multipart(form) => rows(form),
        };
        let auth = match &request.auth {
            Auth::None => 0,
            // Short values (a name, a token): a flat allowance is close enough.
            _ => 256,
        };
        request.url.len()
            + rows(&request.headers)
            + rows(&request.query)
            + body
            + auth
            + request.pre_script.len()
            + request.post_script.len()
            + request.docs.len()
            + self.body_stash.approx_size()
            + 256
    }
}

/// What a step was made by, to decide whether the next edit extends it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
    /// Typing in the field named by this key, last at this time.
    Typing(String, Instant),
    /// A discrete edit (a checkbox, a row, a menu), or the opened file.
    Discrete,
}

/// One tab's history.
#[derive(Debug, Clone, PartialEq)]
pub struct EditHistory {
    /// Every step, oldest first. Never empty: the first is the state the tab opened with.
    entries: Vec<(Snapshot, usize)>,
    /// The step the tab shows now.
    cursor: usize,
    /// The step that matches the file on disk, if it is still in the history.
    saved: Option<usize>,
    /// What made the current step, while it may still be extended.
    origin: Origin,
    /// The sum of the entries' sizes.
    bytes: usize,
}

impl EditHistory {
    /// A history whose only step is `opened`, the state as loaded from disk (so it is the saved
    /// one).
    pub fn new(opened: Snapshot) -> Self {
        let size = opened.approx_size();
        Self {
            entries: vec![(opened, size)],
            cursor: 0,
            saved: Some(0),
            origin: Origin::Discrete,
            bytes: size,
        }
    }

    /// Records `snapshot`, the state an edit just left. `typing` names the field when the edit
    /// is typing in it (`"url"`, `"headers:2:value"`, ...); then, if the current step was typing
    /// in the same field less than [`TYPING_PAUSE`] before `now`, it is replaced instead of a new
    /// step being added. Any step after the cursor (what redo would bring back) is dropped.
    pub fn record(&mut self, snapshot: Snapshot, typing: Option<&str>, now: Instant) {
        let extends = match (&self.origin, typing) {
            (Origin::Typing(field, at), Some(typing)) => {
                field == typing
                    && now.saturating_duration_since(*at) < TYPING_PAUSE
                    && self.cursor + 1 == self.entries.len()
                    && self.cursor > 0
                    && self.saved != Some(self.cursor)
            }
            _ => false,
        };

        self.truncate_after_cursor();
        let size = snapshot.approx_size();
        if extends {
            let (entry, old_size) = &mut self.entries[self.cursor];
            *entry = snapshot;
            self.bytes = self.bytes - *old_size + size;
            *old_size = size;
        } else {
            self.entries.push((snapshot, size));
            self.bytes += size;
            self.cursor = self.entries.len() - 1;
        }
        self.origin = match typing {
            Some(field) => Origin::Typing(field.to_string(), now),
            None => Origin::Discrete,
        };
        self.enforce_budget();
    }

    /// Steps back and returns the snapshot to restore, or `None` at the oldest step.
    pub fn undo(&mut self) -> Option<&Snapshot> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        self.origin = Origin::Discrete;
        Some(&self.entries[self.cursor].0)
    }

    /// Steps forward again and returns the snapshot to restore, or `None` at the newest step.
    pub fn redo(&mut self) -> Option<&Snapshot> {
        if self.cursor + 1 >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        self.origin = Origin::Discrete;
        Some(&self.entries[self.cursor].0)
    }

    /// Marks the current step as the one on disk. Typing after a save starts a new step, so
    /// undo can always return to exactly what was saved.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.cursor);
        self.origin = Origin::Discrete;
    }

    /// Whether the current step differs from the one on disk.
    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.cursor)
    }

    /// Drops the steps after the cursor.
    fn truncate_after_cursor(&mut self) {
        while self.entries.len() > self.cursor + 1 {
            if let Some((_, size)) = self.entries.pop() {
                self.bytes -= size;
            }
        }
        if self.saved.is_some_and(|saved| saved > self.cursor) {
            self.saved = None;
        }
    }

    /// Drops the oldest steps while the history is over [`MAX_BYTES`], keeping the current one.
    fn enforce_budget(&mut self) {
        while self.bytes > MAX_BYTES && self.cursor > 0 {
            let (_, size) = self.entries.remove(0);
            self.bytes -= size;
            self.cursor -= 1;
            self.saved = match self.saved {
                Some(0) | None => None,
                Some(saved) => Some(saved - 1),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn snapshot(url: &str) -> Snapshot {
        Snapshot {
            request: Request {
                url: url.to_string(),
                ..Request::default()
            },
            body_stash: BodyStash::default(),
            auth_stash: AuthStash::default(),
        }
    }

    fn url(snapshot: Option<&Snapshot>) -> Option<&str> {
        snapshot.map(|snapshot| snapshot.request.url.as_str())
    }

    #[test]
    fn a_new_history_has_nothing_to_undo_and_is_clean() {
        let mut history = EditHistory::new(snapshot("a"));
        assert!(!history.is_dirty());
        assert_eq!(history.undo(), None);
        assert_eq!(history.redo(), None);
    }

    #[test]
    fn discrete_edits_are_one_step_each() {
        let now = Instant::now();
        let mut history = EditHistory::new(snapshot("a"));
        history.record(snapshot("b"), None, now);
        history.record(snapshot("c"), None, now);
        assert_eq!(url(history.undo()), Some("b"));
        assert_eq!(url(history.undo()), Some("a"));
        assert_eq!(url(history.undo()), None);
        assert_eq!(url(history.redo()), Some("b"));
        assert_eq!(url(history.redo()), Some("c"));
        assert_eq!(url(history.redo()), None);
    }

    #[test]
    fn typing_in_one_field_without_a_pause_is_one_step() {
        let start = Instant::now();
        let mut history = EditHistory::new(snapshot(""));
        history.record(snapshot("h"), Some("url"), start);
        history.record(
            snapshot("ht"),
            Some("url"),
            start + Duration::from_millis(300),
        );
        history.record(
            snapshot("htt"),
            Some("url"),
            start + Duration::from_millis(600),
        );
        assert_eq!(url(history.undo()), Some(""));
        assert_eq!(url(history.redo()), Some("htt"));
    }

    #[test]
    fn a_pause_or_another_field_starts_a_new_step() {
        let start = Instant::now();
        let mut history = EditHistory::new(snapshot(""));
        history.record(snapshot("a"), Some("url"), start);
        history.record(snapshot("ab"), Some("url"), start + TYPING_PAUSE);
        history.record(snapshot("abc"), Some("docs"), start + TYPING_PAUSE);
        assert_eq!(url(history.undo()), Some("ab"));
        assert_eq!(url(history.undo()), Some("a"));
        assert_eq!(url(history.undo()), Some(""));
    }

    #[test]
    fn a_discrete_edit_ends_a_typing_step() {
        let start = Instant::now();
        let mut history = EditHistory::new(snapshot(""));
        history.record(snapshot("a"), Some("url"), start);
        history.record(snapshot("a!"), None, start);
        history.record(snapshot("a!b"), Some("url"), start);
        assert_eq!(url(history.undo()), Some("a!"));
        assert_eq!(url(history.undo()), Some("a"));
    }

    #[test]
    fn typing_after_an_undo_starts_a_new_step_and_drops_the_redo() {
        let start = Instant::now();
        let mut history = EditHistory::new(snapshot(""));
        history.record(snapshot("a"), Some("url"), start);
        history.record(snapshot("b"), None, start);
        assert_eq!(url(history.undo()), Some("a"));
        history.record(snapshot("ax"), Some("url"), start);
        assert_eq!(history.redo(), None);
        assert_eq!(url(history.undo()), Some("a"));
    }

    #[test]
    fn undo_back_to_the_saved_step_is_clean() {
        let now = Instant::now();
        let mut history = EditHistory::new(snapshot("a"));
        history.record(snapshot("b"), None, now);
        assert!(history.is_dirty());
        history.mark_saved();
        assert!(!history.is_dirty());
        history.record(snapshot("c"), None, now);
        assert!(history.is_dirty());
        history.undo();
        assert!(!history.is_dirty());
        history.undo();
        assert!(
            history.is_dirty(),
            "the opened state is no longer what is on disk"
        );
    }

    #[test]
    fn typing_after_a_save_does_not_extend_the_saved_step() {
        let start = Instant::now();
        let mut history = EditHistory::new(snapshot(""));
        history.record(snapshot("a"), Some("url"), start);
        history.mark_saved();
        history.record(
            snapshot("ab"),
            Some("url"),
            start + Duration::from_millis(100),
        );
        assert_eq!(url(history.undo()), Some("a"));
        assert!(!history.is_dirty());
    }

    #[test]
    fn dropping_the_redo_branch_forgets_a_saved_step_in_it() {
        let now = Instant::now();
        let mut history = EditHistory::new(snapshot("a"));
        history.record(snapshot("b"), None, now);
        history.mark_saved();
        history.undo();
        history.record(snapshot("c"), None, now);
        assert!(history.is_dirty());
        history.undo();
        assert!(
            history.is_dirty(),
            "the saved step is gone, nothing here matches the disk"
        );
    }

    #[test]
    fn the_oldest_steps_go_when_the_budget_is_exceeded() {
        let now = Instant::now();
        let big = "x".repeat(MAX_BYTES / 3);
        let mut history = EditHistory::new(snapshot(&format!("{big}0")));
        for step in 1..=5 {
            history.record(snapshot(&format!("{big}{step}")), None, now);
        }
        let mut steps = 0;
        while history.undo().is_some() {
            steps += 1;
        }
        assert!(steps < 5, "older steps were dropped, kept {steps}");
        assert!(steps >= 1);
        assert!(
            history.is_dirty(),
            "the opened step was dropped with the saved mark"
        );
    }

    #[test]
    fn a_single_snapshot_over_the_budget_is_still_kept() {
        let now = Instant::now();
        let huge = "x".repeat(MAX_BYTES + 1);
        let mut history = EditHistory::new(snapshot("a"));
        history.record(snapshot(&huge), None, now);
        assert_eq!(history.redo(), None);
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].0.request.url.len(), huge.len());
    }
}
