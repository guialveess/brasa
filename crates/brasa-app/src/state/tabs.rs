//! Open tabs: requests being edited, environment editors, load test tabs and the welcome tab, in
//! which order, which one is active, and whether each has unsaved changes. No `gpui` types here, so this is
//! unit-tested directly.

use std::time::Instant;

use brasa_core::Request;

use super::auth_edit::{self, AuthKind, AuthStash};
use super::edit_history::{EditHistory, Snapshot};
use super::env_edit::{self, EnvEditTab};
use super::git_diff::{self, DiffTab};
use super::git_log::{self, LogFilter, LogTab};
use super::load_test::LoadTestTab;
use super::request_edit::{self, BodyKind, BodyStash};
use super::welcome::{self, WelcomeTab};
use super::whats_new;

/// What one open tab shows: a request being edited, an environment being edited, or a load test in
/// progress or finished. Dirty state and saving apply to [`TabKind::Request`] and
/// [`TabKind::Environment`].
#[derive(Debug, Clone, PartialEq)]
pub enum TabKind {
    /// A request open for editing. Boxed, like `LoadTest`, since its auth makes it far larger
    /// than the other boxed variants (clippy's `large_enum_variant`).
    Request(Box<Request>),
    /// An environment editor tab (`state::env_edit`). Boxed for the same reason as `LoadTest`.
    Environment(Box<EnvEditTab>),
    /// A load test tab. Boxed: `LoadTestTab` is far larger than `Request` (it carries the run
    /// history and the latest snapshot), and clippy's `large_enum_variant` flags the gap between
    /// the two otherwise.
    LoadTest(Box<LoadTestTab>),
    /// The welcome tab (`state::welcome`). Nothing to save, like a load test.
    Welcome(WelcomeTab),
    /// The git diff of one file (`state::git_diff`). Read only, nothing to save.
    Diff(Box<DiffTab>),
    /// The git log (`state::git_log`). Read only, nothing to save.
    Log(Box<LogTab>),
    /// The release notes of this version (`state::whats_new`). Nothing to save, and nothing to
    /// hold: the notes are embedded in the binary.
    WhatsNew,
}

/// One open tab.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenTab {
    /// This tab's id: a request's workspace path (matching
    /// [`brasa_workspace::RequestEntry::id`]) for [`TabKind::Request`], or a generated,
    /// otherwise-unused id for [`TabKind::LoadTest`] (`state::tabs::TabsState::open_load_test`).
    pub id: String,
    /// What this tab shows.
    pub kind: TabKind,
    /// Whether the tab has unsaved changes. Always `false` for a [`TabKind::LoadTest`] tab: it
    /// has nothing to save.
    pub dirty: bool,
    /// What the body types the request is not showing held, see [`BodyStash`]. Empty for every
    /// other kind of tab.
    pub body_stash: BodyStash,
    /// What the auth types the request is not showing held, see [`AuthStash`]. Empty for every
    /// other kind of tab.
    pub auth_stash: AuthStash,
    /// Whether the Auth tab shows its secret fields in plain text, switched by their eye button.
    /// Never saved: a reopened tab masks them again.
    pub show_auth_secrets: bool,
    /// Undo and redo of the request's edits (#109). `None` for every other kind of tab.
    pub history: Option<EditHistory>,
}

impl OpenTab {
    /// This tab's request, if it is a [`TabKind::Request`].
    pub fn request(&self) -> Option<&Request> {
        match &self.kind {
            TabKind::Request(request) => Some(request.as_ref()),
            TabKind::Environment(_)
            | TabKind::LoadTest(_)
            | TabKind::Welcome(_)
            | TabKind::Diff(_)
            | TabKind::Log(_)
            | TabKind::WhatsNew => None,
        }
    }

    /// This tab's request, mutably, if it is a [`TabKind::Request`].
    pub fn request_mut(&mut self) -> Option<&mut Request> {
        match &mut self.kind {
            TabKind::Request(request) => Some(request.as_mut()),
            TabKind::Environment(_)
            | TabKind::LoadTest(_)
            | TabKind::Welcome(_)
            | TabKind::Diff(_)
            | TabKind::Log(_)
            | TabKind::WhatsNew => None,
        }
    }

    /// Switches this tab's request body to `kind`, keeping what the other body types held (see
    /// [`request_edit::set_body_kind`]). Does nothing for a tab that is not a request.
    pub fn set_body_kind(&mut self, kind: BodyKind) {
        if let TabKind::Request(request) = &mut self.kind {
            request_edit::set_body_kind(request, kind, &mut self.body_stash);
        }
    }

    /// Switches this tab's request auth to `kind`, keeping what the other auth types held (see
    /// [`auth_edit::set_auth_kind`]). Does nothing for a tab that is not a request.
    pub fn set_auth_kind(&mut self, kind: AuthKind) {
        if let TabKind::Request(request) = &mut self.kind {
            auth_edit::set_auth_kind(request, kind, &mut self.auth_stash);
        }
    }

    /// What an undo would restore of this tab now, if it is a request tab.
    fn snapshot(&self) -> Option<Snapshot> {
        Some(Snapshot {
            request: self.request()?.clone(),
            body_stash: self.body_stash.clone(),
            auth_stash: self.auth_stash.clone(),
        })
    }

    /// Records the state an edit just left in the undo history, then marks the tab dirty.
    /// `typing` names the field when the edit is typing in it, so a burst of typing is one step
    /// (see [`EditHistory::record`]). Does nothing for a tab that is not a request.
    pub fn record_edit(&mut self, typing: Option<&str>, now: Instant) {
        let Some(snapshot) = self.snapshot() else {
            return;
        };
        if let Some(history) = &mut self.history {
            history.record(snapshot, typing, now);
        }
        self.dirty = true;
    }

    /// Restores the previous step of the undo history. Returns whether something changed.
    pub fn undo(&mut self) -> bool {
        let restored = self
            .history
            .as_mut()
            .and_then(|history| history.undo().cloned());
        self.restore(restored)
    }

    /// Restores the next step of the undo history. Returns whether something changed.
    pub fn redo(&mut self) -> bool {
        let restored = self
            .history
            .as_mut()
            .and_then(|history| history.redo().cloned());
        self.restore(restored)
    }

    /// Puts `snapshot` back into the tab and sets the dirty flag from the history (clean again
    /// when back at the saved step).
    fn restore(&mut self, snapshot: Option<Snapshot>) -> bool {
        let Some(snapshot) = snapshot else {
            return false;
        };
        if let TabKind::Request(request) = &mut self.kind {
            **request = snapshot.request;
            self.body_stash = snapshot.body_stash;
            self.auth_stash = snapshot.auth_stash;
        }
        self.dirty = self.history.as_ref().is_some_and(EditHistory::is_dirty);
        true
    }

    /// This tab's environment editor state, if it is a [`TabKind::Environment`].
    pub fn environment(&self) -> Option<&EnvEditTab> {
        match &self.kind {
            TabKind::Environment(edit) => Some(edit.as_ref()),
            TabKind::Request(_)
            | TabKind::LoadTest(_)
            | TabKind::Welcome(_)
            | TabKind::Diff(_)
            | TabKind::Log(_)
            | TabKind::WhatsNew => None,
        }
    }

    /// This tab's environment editor state, mutably, if it is a [`TabKind::Environment`].
    pub fn environment_mut(&mut self) -> Option<&mut EnvEditTab> {
        match &mut self.kind {
            TabKind::Environment(edit) => Some(edit.as_mut()),
            TabKind::Request(_)
            | TabKind::LoadTest(_)
            | TabKind::Welcome(_)
            | TabKind::Diff(_)
            | TabKind::Log(_)
            | TabKind::WhatsNew => None,
        }
    }

    /// This tab's load test state, if it is a [`TabKind::LoadTest`].
    pub fn load_test(&self) -> Option<&LoadTestTab> {
        match &self.kind {
            TabKind::LoadTest(load_test) => Some(load_test.as_ref()),
            TabKind::Request(_)
            | TabKind::Environment(_)
            | TabKind::Welcome(_)
            | TabKind::Diff(_)
            | TabKind::Log(_)
            | TabKind::WhatsNew => None,
        }
    }

    /// This tab's welcome content, if it is the [`TabKind::Welcome`] tab.
    pub fn welcome(&self) -> Option<&WelcomeTab> {
        match &self.kind {
            TabKind::Welcome(welcome) => Some(welcome),
            TabKind::Request(_)
            | TabKind::Environment(_)
            | TabKind::LoadTest(_)
            | TabKind::Diff(_)
            | TabKind::Log(_)
            | TabKind::WhatsNew => None,
        }
    }

    /// This tab's diff, if it is the [`TabKind::Diff`] tab.
    pub fn diff(&self) -> Option<&DiffTab> {
        match &self.kind {
            TabKind::Diff(diff) => Some(diff.as_ref()),
            _ => None,
        }
    }

    /// This tab's diff, mutably, if it is the [`TabKind::Diff`] tab.
    pub fn diff_mut(&mut self) -> Option<&mut DiffTab> {
        match &mut self.kind {
            TabKind::Diff(diff) => Some(diff.as_mut()),
            _ => None,
        }
    }

    /// This tab's log, if it is the [`TabKind::Log`] tab.
    pub fn log(&self) -> Option<&LogTab> {
        match &self.kind {
            TabKind::Log(log) => Some(log.as_ref()),
            _ => None,
        }
    }

    /// This tab's log, mutably, if it is the [`TabKind::Log`] tab.
    pub fn log_mut(&mut self) -> Option<&mut LogTab> {
        match &mut self.kind {
            TabKind::Log(log) => Some(log.as_mut()),
            _ => None,
        }
    }

    /// This tab's load test state, mutably, if it is a [`TabKind::LoadTest`].
    pub fn load_test_mut(&mut self) -> Option<&mut LoadTestTab> {
        match &mut self.kind {
            TabKind::LoadTest(load_test) => Some(load_test.as_mut()),
            TabKind::Request(_)
            | TabKind::Environment(_)
            | TabKind::Welcome(_)
            | TabKind::Diff(_)
            | TabKind::Log(_)
            | TabKind::WhatsNew => None,
        }
    }
}

/// The set of open tabs and which one is active, if any.
#[derive(Debug, Default)]
pub struct TabsState {
    open: Vec<OpenTab>,
    active: Option<usize>,
    /// The next id [`Self::open_load_test`] hands out, so every load test tab gets a fresh,
    /// never-reused id even after earlier ones are closed (unlike a request's id, a load test
    /// tab's id has nothing to naturally dedupe on).
    next_load_test_id: u32,
}

impl TabsState {
    /// The open tabs, in the order they were opened.
    pub fn open_tabs(&self) -> &[OpenTab] {
        &self.open
    }

    /// The index of the active tab, if any tab is open.
    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    /// The active tab, if any.
    pub fn active(&self) -> Option<&OpenTab> {
        self.active.and_then(|index| self.open.get(index))
    }

    /// The open tab at `index`. `None` if `index` is out of range.
    pub fn get(&self, index: usize) -> Option<&OpenTab> {
        self.open.get(index)
    }

    /// The open tab at `index`, mutably. `None` if `index` is out of range.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut OpenTab> {
        self.open.get_mut(index)
    }

    /// The index of the open tab for `id`, if it is already open.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.open.iter().position(|tab| tab.id == id)
    }

    /// Opens `request` as a tab and makes it active. If `id` is already open, its tab is made
    /// active instead of opening a second one, like a normal editor's "one tab per file".
    /// Returns the tab's index.
    pub fn open(&mut self, id: impl Into<String>, request: Request) -> usize {
        let id = id.into();
        if let Some(index) = self.index_of(&id) {
            self.active = Some(index);
            return index;
        }
        self.push_and_activate(id, TabKind::Request(Box::new(request)))
    }

    /// Opens `edit` as the editor tab of its environment and makes it active. If that
    /// environment already has a tab, it is made active instead and `edit` is dropped. Returns
    /// the tab's index.
    pub fn open_environment(&mut self, edit: EnvEditTab) -> usize {
        let id = env_edit::tab_id(&edit.name);
        if let Some(index) = self.index_of(&id) {
            self.active = Some(index);
            return index;
        }
        self.push_and_activate(id, TabKind::Environment(Box::new(edit)))
    }

    /// The name of the environment shown by the active tab, if the active tab is an environment
    /// editor.
    pub fn active_environment_name(&self) -> Option<&str> {
        self.active()
            .and_then(OpenTab::environment)
            .map(|edit| edit.name.as_str())
    }

    /// Points the editor tab of the environment `old` at `new` after a rename: its id and its
    /// state. Does nothing if `old` has no tab.
    pub fn rename_environment(&mut self, old: &str, new: &str) {
        let Some(index) = self.index_of(&env_edit::tab_id(old)) else {
            return;
        };
        if let Some(tab) = self.open.get_mut(index) {
            tab.id = env_edit::tab_id(new);
            if let Some(edit) = tab.environment_mut() {
                edit.name = new.to_string();
            }
        }
    }

    /// Opens the welcome tab with `welcome` and makes it active. If it is already open, it is
    /// made active and its content replaced, so its recent workspaces are fresh. Returns the
    /// tab's index.
    pub fn open_welcome(&mut self, welcome: WelcomeTab) -> usize {
        if let Some(index) = self.index_of(welcome::TAB_ID) {
            if let Some(tab) = self.open.get_mut(index) {
                tab.kind = TabKind::Welcome(welcome);
            }
            self.active = Some(index);
            return index;
        }
        self.push_and_activate(welcome::TAB_ID.to_string(), TabKind::Welcome(welcome))
    }

    /// Shows `diff` in the diff tab, opening it when it is not open yet, and makes it active.
    /// There is only one diff tab: a new file replaces the one it showed, keeping its place.
    pub fn open_diff(&mut self, diff: DiffTab) -> usize {
        if let Some(index) = self.index_of(git_diff::TAB_ID) {
            if let Some(tab) = self.open.get_mut(index) {
                tab.kind = TabKind::Diff(Box::new(diff));
            }
            self.active = Some(index);
            return index;
        }
        self.push_and_activate(git_diff::TAB_ID.to_string(), TabKind::Diff(Box::new(diff)))
    }

    /// Opens the log tab for `filter` and makes it active, or just makes it active when it is
    /// open. Returns whether it was open, and its index.
    pub fn open_log(&mut self, filter: LogFilter) -> (bool, usize) {
        if let Some(index) = self.index_of(git_log::TAB_ID) {
            self.active = Some(index);
            return (true, index);
        }
        let index = self.push_and_activate(
            git_log::TAB_ID.to_string(),
            TabKind::Log(Box::new(LogTab::new(filter))),
        );
        (false, index)
    }

    /// Opens the What's new tab and makes it active, or just makes it active when it is open.
    /// Returns its index.
    pub fn open_whats_new(&mut self) -> usize {
        if let Some(index) = self.index_of(whats_new::TAB_ID) {
            self.active = Some(index);
            return index;
        }
        self.push_and_activate(whats_new::TAB_ID.to_string(), TabKind::WhatsNew)
    }

    /// Whether the active tab is the What's new tab.
    pub fn is_whats_new_active(&self) -> bool {
        self.active()
            .is_some_and(|tab| tab.kind == TabKind::WhatsNew)
    }

    /// Opens a new load test tab and makes it active. Always a fresh tab, never deduplicated:
    /// unlike a request, several load test tabs can target the same request or collection at once.
    /// Returns the tab's id and index.
    pub fn open_load_test(&mut self, load_test: LoadTestTab) -> (String, usize) {
        let id = format!("load-test:{}", self.next_load_test_id);
        self.next_load_test_id += 1;
        let index = self.push_and_activate(id.clone(), TabKind::LoadTest(Box::new(load_test)));
        (id, index)
    }

    /// Appends a new tab and makes it active. Shared by [`Self::open`] and
    /// [`Self::open_load_test`].
    fn push_and_activate(&mut self, id: String, kind: TabKind) -> usize {
        let history = match &kind {
            TabKind::Request(request) => Some(EditHistory::new(Snapshot {
                request: request.as_ref().clone(),
                body_stash: BodyStash::default(),
                auth_stash: AuthStash::default(),
            })),
            TabKind::Environment(_)
            | TabKind::LoadTest(_)
            | TabKind::Welcome(_)
            | TabKind::Diff(_)
            | TabKind::Log(_)
            | TabKind::WhatsNew => None,
        };
        self.open.push(OpenTab {
            id,
            kind,
            dirty: false,
            body_stash: BodyStash::default(),
            auth_stash: AuthStash::default(),
            show_auth_secrets: false,
            history,
        });
        let index = self.open.len() - 1;
        self.active = Some(index);
        index
    }

    /// Closes the tab at `index`. If it was the active tab, the tab that takes its place (the
    /// next one, or the previous one if the closed tab was last) becomes active. Does nothing if
    /// `index` is out of range.
    pub fn close(&mut self, index: usize) {
        if index >= self.open.len() {
            return;
        }
        self.open.remove(index);
        self.active = self.active.and_then(|active| {
            if self.open.is_empty() {
                None
            } else if active < index {
                Some(active)
            } else if active > index {
                Some(active - 1)
            } else {
                Some(index.min(self.open.len() - 1))
            }
        });
    }

    /// Makes the tab at `index` active. Does nothing if `index` is out of range.
    pub fn set_active(&mut self, index: usize) {
        if index < self.open.len() {
            self.active = Some(index);
        }
    }

    /// Moves the tab at `from` to position `to` (dragging a tab in the open-tabs bar), shifting
    /// the tabs in between by one. The active tab stays the same tab, wherever it ends up. `to`
    /// past the end means the last position. Does nothing if `from` is out of range.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.open.len() {
            return;
        }
        let to = to.min(self.open.len() - 1);
        if from == to {
            return;
        }
        let active_id = self.active().map(|tab| tab.id.clone());
        let tab = self.open.remove(from);
        self.open.insert(to, tab);
        self.active = active_id.and_then(|id| self.index_of(&id));
    }

    /// Activates the tab after the active one, wrapping from the last to the first. Does nothing
    /// when no tab is open.
    pub fn activate_next(&mut self) {
        let count = self.open.len();
        if let Some(active) = self.active {
            self.active = Some((active + 1) % count);
        }
    }

    /// Activates the tab before the active one, wrapping from the first to the last. Does
    /// nothing when no tab is open.
    pub fn activate_previous(&mut self) {
        let count = self.open.len();
        if let Some(active) = self.active {
            self.active = Some((active + count - 1) % count);
        }
    }

    /// Marks the tab at `index` as saved (no unsaved changes). Does nothing if out of range.
    pub fn mark_saved(&mut self, index: usize) {
        if let Some(tab) = self.open.get_mut(index) {
            tab.dirty = false;
            if let Some(history) = &mut tab.history {
                history.mark_saved();
            }
        }
    }

    /// Sets the dirty flag of the tab at `index`. Does nothing if out of range.
    pub fn set_dirty(&mut self, index: usize, dirty: bool) {
        if let Some(tab) = self.open.get_mut(index) {
            tab.dirty = dirty;
        }
    }

    /// Updates the id of every open tab under `old_prefix` to the same path under `new_prefix`,
    /// after [`brasa_workspace::Workspace::rename`] moves a request or a whole folder. A tab's
    /// id matches when it equals `old_prefix` exactly (the renamed item was itself a request) or
    /// starts with `old_prefix` followed by `/` (the renamed item was a folder and the tab is
    /// one of its descendants).
    pub fn rename_prefix(&mut self, old_prefix: &str, new_prefix: &str) {
        for tab in &mut self.open {
            if let Some(rest) = matching_rest(&tab.id, old_prefix) {
                tab.id = format!("{new_prefix}{rest}");
            }
        }
    }

    /// Closes every open tab under `prefix` (matched the same way as [`Self::rename_prefix`]),
    /// after [`brasa_workspace::Workspace::delete`] removes a request or a whole folder.
    pub fn close_prefix(&mut self, prefix: &str) {
        let mut index = 0;
        while index < self.open.len() {
            if matching_rest(&self.open[index].id, prefix).is_some() {
                self.close(index);
            } else {
                index += 1;
            }
        }
    }
}

/// If `id` equals `prefix` or starts with `prefix` followed by `/`, returns the rest of `id`
/// after `prefix` (an empty string for an exact match). `None` otherwise.
fn matching_rest<'a>(id: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = id.strip_prefix(prefix)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request::default()
    }

    #[test]
    fn open_adds_a_new_tab_and_makes_it_active() {
        let mut tabs = TabsState::default();
        let index = tabs.open("a.brasa", request());
        assert_eq!(index, 0);
        assert_eq!(tabs.active_index(), Some(0));
        assert_eq!(tabs.open_tabs().len(), 1);
        assert_eq!(tabs.active().map(|tab| tab.id.as_str()), Some("a.brasa"));
    }

    #[test]
    fn open_reuses_an_already_open_tab_instead_of_duplicating_it() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.open("b.brasa", request());
        let index = tabs.open("a.brasa", request());
        assert_eq!(index, 0);
        assert_eq!(tabs.open_tabs().len(), 2);
        assert_eq!(tabs.active_index(), Some(0));
    }

    #[test]
    fn closing_the_active_tab_activates_the_tab_that_took_its_place() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.open("b.brasa", request());
        tabs.open("c.brasa", request());
        tabs.set_active(1);

        tabs.close(1);

        assert_eq!(tabs.open_tabs().len(), 2);
        assert_eq!(tabs.active_index(), Some(1));
        assert_eq!(tabs.open_tabs()[1].id, "c.brasa");
    }

    fn ids(tabs: &TabsState) -> Vec<&str> {
        tabs.open_tabs().iter().map(|tab| tab.id.as_str()).collect()
    }

    fn three_tabs() -> TabsState {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.open("b.brasa", request());
        tabs.open("c.brasa", request());
        tabs
    }

    #[test]
    fn move_tab_moves_forward_and_backward() {
        let mut tabs = three_tabs();
        tabs.move_tab(0, 2);
        assert_eq!(ids(&tabs), ["b.brasa", "c.brasa", "a.brasa"]);
        tabs.move_tab(2, 0);
        assert_eq!(ids(&tabs), ["a.brasa", "b.brasa", "c.brasa"]);
        tabs.move_tab(1, 0);
        assert_eq!(ids(&tabs), ["b.brasa", "a.brasa", "c.brasa"]);
    }

    #[test]
    fn move_tab_keeps_the_same_tab_active() {
        let mut tabs = three_tabs();
        tabs.set_active(1);

        tabs.move_tab(1, 0);
        assert_eq!(tabs.active().map(|tab| tab.id.as_str()), Some("b.brasa"));
        assert_eq!(tabs.active_index(), Some(0));

        tabs.move_tab(2, 0);
        assert_eq!(ids(&tabs), ["c.brasa", "b.brasa", "a.brasa"]);
        assert_eq!(tabs.active().map(|tab| tab.id.as_str()), Some("b.brasa"));
        assert_eq!(tabs.active_index(), Some(1));
    }

    #[test]
    fn move_tab_clamps_the_target_and_ignores_a_bad_source() {
        let mut tabs = three_tabs();
        tabs.move_tab(0, 99);
        assert_eq!(ids(&tabs), ["b.brasa", "c.brasa", "a.brasa"]);
        tabs.move_tab(7, 0);
        assert_eq!(ids(&tabs), ["b.brasa", "c.brasa", "a.brasa"]);
    }

    #[test]
    fn closing_the_last_remaining_tab_leaves_nothing_active() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());

        tabs.close(0);

        assert!(tabs.open_tabs().is_empty());
        assert_eq!(tabs.active_index(), None);
    }

    #[test]
    fn closing_a_tab_before_the_active_one_shifts_the_active_index_left() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.open("b.brasa", request());
        tabs.set_active(1);

        tabs.close(0);

        assert_eq!(tabs.active_index(), Some(0));
        assert_eq!(tabs.open_tabs()[0].id, "b.brasa");
    }

    #[test]
    fn closing_an_out_of_range_index_does_nothing() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());

        tabs.close(5);

        assert_eq!(tabs.open_tabs().len(), 1);
    }

    #[test]
    fn activate_next_moves_right_and_wraps_to_the_first_tab() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.open("b.brasa", request());
        tabs.open("c.brasa", request());
        tabs.set_active(0);

        tabs.activate_next();
        assert_eq!(tabs.active_index(), Some(1));
        tabs.activate_next();
        assert_eq!(tabs.active_index(), Some(2));
        tabs.activate_next();
        assert_eq!(tabs.active_index(), Some(0));
    }

    #[test]
    fn activate_previous_moves_left_and_wraps_to_the_last_tab() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.open("b.brasa", request());
        tabs.open("c.brasa", request());
        tabs.set_active(1);

        tabs.activate_previous();
        assert_eq!(tabs.active_index(), Some(0));
        tabs.activate_previous();
        assert_eq!(tabs.active_index(), Some(2));
    }

    #[test]
    fn cycling_tabs_with_none_open_or_a_single_tab_is_harmless() {
        let mut tabs = TabsState::default();
        tabs.activate_next();
        tabs.activate_previous();
        assert_eq!(tabs.active_index(), None);

        tabs.open("a.brasa", request());
        tabs.activate_next();
        assert_eq!(tabs.active_index(), Some(0));
        tabs.activate_previous();
        assert_eq!(tabs.active_index(), Some(0));
    }

    #[test]
    fn mark_saved_clears_the_dirty_flag() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.set_dirty(0, true);

        tabs.mark_saved(0);

        assert!(!tabs.open_tabs()[0].dirty);
    }

    #[test]
    fn an_edit_marks_the_tab_dirty_and_undo_to_the_saved_state_cleans_it() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        let tab = tabs.get_mut(0).expect("tab");

        if let Some(request) = tab.request_mut() {
            request.url = "https://changed.test".to_string();
        }
        tab.record_edit(None, Instant::now());
        assert!(tab.dirty);

        assert!(tab.undo());
        assert!(!tab.dirty);
        assert_eq!(tab.request(), Some(&request()));

        assert!(tab.redo());
        assert!(tab.dirty);
        assert_eq!(
            tab.request().map(|request| request.url.as_str()),
            Some("https://changed.test")
        );
    }

    #[test]
    fn undo_restores_what_the_other_body_kinds_held() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        let tab = tabs.get_mut(0).expect("tab");
        if let Some(request) = tab.request_mut() {
            request.body = brasa_core::Body::Json("{}".to_string());
        }
        tab.record_edit(Some("body"), Instant::now());
        tab.set_body_kind(BodyKind::Form);
        tab.record_edit(None, Instant::now());

        assert!(tab.undo());
        assert_eq!(
            tab.request().map(|request| &request.body),
            Some(&brasa_core::Body::Json("{}".to_string()))
        );
        tab.set_body_kind(BodyKind::Form);
        tab.set_body_kind(BodyKind::Json);
        assert_eq!(
            tab.request().map(|request| &request.body),
            Some(&brasa_core::Body::Json("{}".to_string())),
            "the JSON text survives a switch to Form and back after the undo"
        );
    }

    #[test]
    fn undo_with_nothing_to_undo_or_on_another_kind_of_tab_changes_nothing() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        assert!(!tabs.get_mut(0).expect("tab").undo());
        assert!(!tabs.get_mut(0).expect("tab").redo());

        tabs.open_welcome(WelcomeTab::default());
        let welcome = tabs.get_mut(1).expect("welcome tab");
        welcome.record_edit(None, Instant::now());
        assert!(!welcome.dirty);
        assert!(!welcome.undo());
    }

    #[test]
    fn get_returns_the_tab_at_index() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.open("b.brasa", request());

        let tab = tabs.get(1).expect("index 1 is open");
        assert_eq!(tab.id, "b.brasa");
        assert!(tabs.get(5).is_none());
    }

    #[test]
    fn get_mut_returns_the_tab_at_index() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.open("b.brasa", request());

        let tab = tabs.get_mut(0).expect("index 0 is open");
        assert_eq!(tab.id, "a.brasa");
    }

    #[test]
    fn rename_prefix_updates_an_exact_match() {
        let mut tabs = TabsState::default();
        tabs.open("auth/login.brasa", request());

        tabs.rename_prefix("auth/login.brasa", "auth/sign-in.brasa");

        assert_eq!(tabs.open_tabs()[0].id, "auth/sign-in.brasa");
    }

    #[test]
    fn rename_prefix_updates_descendants_of_a_renamed_folder() {
        let mut tabs = TabsState::default();
        tabs.open("auth/login.brasa", request());
        tabs.open("other.brasa", request());

        tabs.rename_prefix("auth", "identity");

        assert_eq!(tabs.open_tabs()[0].id, "identity/login.brasa");
        assert_eq!(tabs.open_tabs()[1].id, "other.brasa");
    }

    #[test]
    fn rename_prefix_does_not_match_a_sibling_with_a_shared_prefix() {
        let mut tabs = TabsState::default();
        tabs.open("auth-extra.brasa", request());

        tabs.rename_prefix("auth", "identity");

        assert_eq!(tabs.open_tabs()[0].id, "auth-extra.brasa");
    }

    #[test]
    fn close_prefix_closes_an_exact_match_and_descendants() {
        let mut tabs = TabsState::default();
        tabs.open("auth/login.brasa", request());
        tabs.open("auth/logout.brasa", request());
        tabs.open("other.brasa", request());

        tabs.close_prefix("auth");

        assert_eq!(tabs.open_tabs().len(), 1);
        assert_eq!(tabs.open_tabs()[0].id, "other.brasa");
    }

    #[test]
    fn open_load_test_opens_a_new_tab_and_makes_it_active() {
        let mut tabs = TabsState::default();
        let (id, index) = tabs.open_load_test(LoadTestTab::unset());
        assert_eq!(index, 0);
        assert_eq!(tabs.active_index(), Some(0));
        assert_eq!(tabs.open_tabs()[0].id, id);
        assert!(tabs.open_tabs()[0].load_test().is_some());
        assert!(!tabs.open_tabs()[0].dirty);
    }

    #[test]
    fn open_load_test_never_deduplicates_even_with_the_same_target() {
        let mut tabs = TabsState::default();
        let (first_id, _) = tabs.open_load_test(LoadTestTab::unset());
        let (second_id, _) = tabs.open_load_test(LoadTestTab::unset());
        assert_ne!(first_id, second_id);
        assert_eq!(tabs.open_tabs().len(), 2);
    }

    /// #150: with a request's load test active, opening the request again goes back to its own
    /// tab, edits included, next to the load test.
    #[test]
    fn opening_a_request_from_its_load_test_focuses_the_request_tab() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.set_dirty(0, true);
        tabs.open_load_test(LoadTestTab::unset());

        tabs.open("a.brasa", request());

        assert_eq!(tabs.open_tabs().len(), 2);
        let active = tabs.active().expect("a tab is active");
        assert_eq!(active.id, "a.brasa");
        assert!(active.request().is_some());
        assert!(active.dirty);
    }

    #[test]
    fn a_request_tabs_kind_exposes_its_request_but_not_a_load_test() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        let tab = tabs.active().expect("just opened");
        assert!(tab.request().is_some());
        assert!(tab.load_test().is_none());
    }

    #[test]
    fn a_load_test_tabs_kind_exposes_its_load_test_but_not_a_request() {
        let mut tabs = TabsState::default();
        tabs.open_load_test(LoadTestTab::unset());
        let tab = tabs.active().expect("just opened");
        assert!(tab.load_test().is_some());
        assert!(tab.request().is_none());
    }

    #[test]
    fn rename_prefix_does_not_touch_a_load_test_tab() {
        let mut tabs = TabsState::default();
        let (load_test_id, _) = tabs.open_load_test(LoadTestTab::unset());

        tabs.rename_prefix("load-test", "renamed");

        assert_eq!(tabs.open_tabs()[0].id, load_test_id);
    }

    #[test]
    fn closing_a_tab_leaves_other_tabs_kind_untouched() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.open_load_test(LoadTestTab::unset());

        tabs.close(0);

        assert_eq!(tabs.open_tabs().len(), 1);
        assert!(tabs.open_tabs()[0].load_test().is_some());
    }

    fn env_tab(name: &str) -> EnvEditTab {
        EnvEditTab::load(
            name,
            &brasa_workspace::EnvLayers {
                base: Some(Vec::new()),
                local: None,
            },
        )
    }

    #[test]
    fn open_environment_dedupes_by_name_and_never_collides_with_requests() {
        let mut tabs = TabsState::default();
        tabs.open("local", request());
        let first = tabs.open_environment(env_tab("local"));
        tabs.open("b.brasa", request());
        let second = tabs.open_environment(env_tab("local"));
        assert_eq!(first, 1);
        assert_eq!(second, 1);
        assert_eq!(tabs.open_tabs().len(), 3);
        assert_eq!(tabs.active_environment_name(), Some("local"));
        assert_eq!(tabs.open_tabs()[1].id, "env:local");
    }

    #[test]
    fn an_environment_tab_exposes_its_editor_only() {
        let mut tabs = TabsState::default();
        tabs.open_environment(env_tab("local"));
        let tab = tabs.active().expect("just opened");
        assert!(tab.environment().is_some());
        assert!(tab.request().is_none());
        assert!(tab.load_test().is_none());
    }

    #[test]
    fn rename_environment_updates_the_id_and_the_name() {
        let mut tabs = TabsState::default();
        tabs.open_environment(env_tab("local"));
        tabs.rename_environment("local", "dev");
        assert_eq!(tabs.open_tabs()[0].id, "env:dev");
        assert_eq!(tabs.active_environment_name(), Some("dev"));
    }

    #[test]
    fn rename_prefix_and_close_prefix_leave_environment_tabs_alone() {
        let mut tabs = TabsState::default();
        tabs.open_environment(env_tab("auth"));
        tabs.rename_prefix("auth", "x");
        tabs.close_prefix("auth");
        assert_eq!(tabs.open_tabs()[0].id, "env:auth");
    }

    #[test]
    fn set_dirty_sets_and_clears_the_flag() {
        let mut tabs = TabsState::default();
        tabs.open("a.brasa", request());
        tabs.set_dirty(0, true);
        assert!(tabs.open_tabs()[0].dirty);
        tabs.set_dirty(0, false);
        assert!(!tabs.open_tabs()[0].dirty);
        tabs.set_dirty(9, true);
    }

    #[test]
    fn open_welcome_dedupes_and_refreshes_its_content() {
        let mut tabs = TabsState::default();
        let first = tabs.open_welcome(WelcomeTab::default());
        tabs.open("a.brasa", request());
        let refreshed = WelcomeTab {
            recents: vec![crate::state::welcome::RecentWorkspace {
                name: "api".to_string(),
                path_label: "~/api".to_string(),
                path: std::path::PathBuf::from("/home/ana/api"),
            }],
        };
        let second = tabs.open_welcome(refreshed.clone());
        assert_eq!(first, 0);
        assert_eq!(second, 0);
        assert_eq!(tabs.open_tabs().len(), 2);
        assert_eq!(tabs.active_index(), Some(0));
        assert_eq!(tabs.open_tabs()[0].welcome(), Some(&refreshed));
    }

    #[test]
    fn the_welcome_tab_is_only_a_welcome_tab() {
        let mut tabs = TabsState::default();
        tabs.open_welcome(WelcomeTab::default());
        let tab = tabs.active().expect("just opened");
        assert!(tab.welcome().is_some());
        assert!(tab.request().is_none());
        assert!(tab.environment().is_none());
        assert!(tab.load_test().is_none());
        assert!(!tab.dirty);
    }

    #[test]
    fn there_is_one_whats_new_tab() {
        let mut tabs = TabsState::default();
        assert!(!tabs.is_whats_new_active());
        let first = tabs.open_whats_new();
        tabs.open("a.brasa", request());
        assert!(!tabs.is_whats_new_active());
        assert_eq!(tabs.open_whats_new(), first);
        assert_eq!(tabs.open_tabs().len(), 2);
        assert!(tabs.is_whats_new_active());
        let tab = tabs.active().expect("just opened");
        assert!(tab.request().is_none() && tab.welcome().is_none() && !tab.dirty);
    }
}
