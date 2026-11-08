//! The Git log tab (#174, design 4a): the branch column on the left, the filter bar and the
//! commit list with its graph in the middle, the selected commit's detail on the right.
//!
//! Commits are read in pages on the background executor ([`AppView::open_git_log`],
//! [`AppView::load_more_git_log`]); every page is laid out when it arrives (`state::git_log`),
//! so render only walks the visible rows. The next page is asked for from the list's range
//! callback when the visible rows get near the end, never from a timer. The only timer is the
//! short one that waits for the user to stop typing in the filter bar; it starts on a keystroke
//! and is dropped by the next one.
//!
//! Graph colors: there is no pink in the palette, so the lanes cycle through `accent_text`,
//! `success`, the PATCH method color (a violet), `info`, `warning` and `fg_muted`, which are
//! legible in both themes. The first three are the design's.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::{Scrollbar, ScrollbarHandle};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use brasa_core::Method;
use rust_i18n::t;
use telaio_git::{GraphRow, LinePart, Repo};
use telaio_ui::components::{
    SearchableSelect, SelectHandle, SelectOption, on_pick, pick, select_state, set_options,
};

use crate::state::format::relative_time;
use crate::state::git_diff::DiffTarget;
use crate::state::git_log::{
    BranchColumn, Detail, DetailFile, DetailView, GRAPH_COLORS, LogBranchRef, LogBranchRow,
    LogFilter, LogRow, LogSection, MAX_LANES, RefBadge, RefClass, TAB_ID, lane_x, request_methods,
    resize_column,
};
use crate::state::locale::plural;
use crate::state::number::format_integer;
use crate::theme::metrics::{INPUT_PADDING_X, RADIUS_MD, RADIUS_SM, SIDEBAR_FILTER_HEIGHT};
use crate::theme::{Palette, PaletteExt, BrasaColors};
use crate::views::components::{
    EmptyState, IconButton, MethodBadge, SecondaryButton, SectionLabel, TreeRow, edit_menu,
};

use super::super::root::AppView;
use super::CheckoutTarget;

/// The height of a commit row (design 4a).
const ROW_HEIGHT: f32 = 30.0;
/// The height of a row of the branch column (design 4a).
const BRANCH_ROW_HEIGHT: f32 = 24.0;
/// The height of a row of the changed files (design 4a).
const FILE_ROW_HEIGHT: f32 = 26.0;
/// The default width of the branch column, and how far it can be resized.
const BRANCH_COLUMN_WIDTH: f32 = 250.0;
const BRANCH_COLUMN_MIN: f32 = 180.0;
const BRANCH_COLUMN_MAX: f32 = 440.0;
/// The default height of the selected commit's message in the detail column, and how far the
/// line under it can move. The changed files take the rest.
const DETAIL_HEADER_HEIGHT: f32 = 280.0;
const DETAIL_HEADER_MIN: f32 = 120.0;
const DETAIL_HEADER_MAX: f32 = 900.0;
/// The default width of the detail column, and how far it can be resized.
const DETAIL_WIDTH: f32 = 340.0;
const DETAIL_MIN: f32 = 260.0;
const DETAIL_MAX: f32 = 620.0;
/// The height of the filter bar and of the column titles.
const FILTER_BAR_HEIGHT: f32 = 44.0;
const TITLES_HEIGHT: f32 = 28.0;
const SEARCH_WIDTH: f32 = 220.0;
/// The default widths of the Author, Date and Commit columns, which the user can drag.
const AUTHOR_WIDTH: f32 = 110.0;
const DATE_WIDTH: f32 = 96.0;
const HASH_WIDTH: f32 = 70.0;
/// The width of the drag handle between two column titles, centered in their gap.
const COLUMN_HANDLE_WIDTH: f32 = 8.0;
const COLUMN_GAP: f32 = 12.0;
/// The narrowest the subject column gets; author and date give way first.
const SUBJECT_MIN_WIDTH: f32 = 140.0;
/// The narrowest a filter input gets when the center column is tight.
const PILL_MIN_WIDTH: f32 = 72.0;
/// The value of the Branch filter's "every branch" option; no ref name can hold a NUL.
const ALL_BRANCHES: &str = "\u{0}all";
/// The width of the Branch filter's select.
const BRANCH_SELECT_WIDTH: f32 = 200.0;
/// How long the filter bar waits after the last keystroke before it reads the log again.
const FILTER_DEBOUNCE: Duration = Duration::from_millis(350);
/// The next page is asked for when the visible rows get this close to the end of the list.
const LOAD_AHEAD: usize = 30;
/// The radius of the dot of a commit, and of the commit `HEAD` points at (outer, stroke included).
const DOT_RADIUS: f32 = 5.0;
const HEAD_DOT_RADIUS: f32 = 6.0;
const STROKE: f32 = 2.0;

/// The live parts of the log tab that are not plain data: the inputs and the scroll positions.
pub(crate) struct LogView {
    search: Entity<InputState>,
    author: Entity<InputState>,
    path: Entity<InputState>,
    branch_filter: Entity<InputState>,
    rows_scroll: UniformListScrollHandle,
    /// The commit list's focus, taken by a click on it, so the arrow keys move the selection.
    rows_focus: FocusHandle,
    branches_scroll: UniformListScrollHandle,
    files_scroll: UniformListScrollHandle,
    /// The selected commit's subject, message and facts, which scroll when they are long.
    header_scroll: ScrollHandle,
    /// Where the line between the message and the changed files sits. Kept here, not in the
    /// group's element state, which is dropped while the next commit's detail loads.
    detail_split: Entity<ResizableState>,
    /// The widths of the Author, Date and Commit columns, in that order. The subject takes the
    /// rest. Kept for the session.
    columns: [f32; 3],
    /// The Branch filter: a select with a search box over every branch, remote branch and tag.
    branch_select: SelectHandle,
    _branch_pick: Subscription,
    /// The branch list the select's options were built from (its `Arc` address), so they are
    /// rebuilt only when the branches change, never on every render.
    branch_options_key: Option<usize>,
    /// The filter value the select last showed, so it follows a click in the branch column.
    synced_branch: Option<String>,
    /// The wait after a keystroke in the filter bar; replacing it drops the previous wait.
    debounce: Option<Task<()>>,
    column: BranchColumn,
}

impl LogView {
    fn new(window: &mut Window, cx: &mut Context<AppView>) -> Self {
        let input = |window: &mut Window, cx: &mut Context<AppView>, placeholder: String| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let search = input(window, cx, t!("git_log.search").into_owned());
        let author = input(window, cx, t!("git_log.filter.any").into_owned());
        let path = input(
            window,
            cx,
            t!("git_log.filter.path_placeholder").into_owned(),
        );
        let branch_filter = input(window, cx, t!("git_log.filter_branches").into_owned());
        // Typing reads the log again once the user pauses; Enter reads it right away.
        for state in [&search, &author, &path] {
            cx.subscribe(state, |view, _, event: &InputEvent, cx| match event {
                InputEvent::Change => view.schedule_log_filter(cx),
                InputEvent::PressEnter { .. } => view.apply_log_inputs(cx),
                _ => {}
            })
            .detach();
        }
        cx.subscribe(&branch_filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let branch_select = select_state(Vec::new(), None, true, window, cx);
        let branch_pick = on_pick(&branch_select, window, cx, |view, value, _, cx| {
            let branch = (value != ALL_BRANCHES).then(|| value.to_string());
            view.set_log_branch(branch, cx);
        });
        LogView {
            search,
            author,
            path,
            branch_filter,
            rows_scroll: UniformListScrollHandle::new(),
            rows_focus: cx.focus_handle(),
            branches_scroll: UniformListScrollHandle::new(),
            files_scroll: UniformListScrollHandle::new(),
            header_scroll: ScrollHandle::new(),
            detail_split: cx.new(|_| ResizableState::default()),
            columns: [AUTHOR_WIDTH, DATE_WIDTH, HASH_WIDTH],
            branch_select,
            _branch_pick: branch_pick,
            branch_options_key: None,
            synced_branch: None,
            debounce: None,
            column: BranchColumn::default(),
        }
    }
}

impl AppView {
    /// Opens the Git log tab, or brings it forward when it is open, and shows the history of
    /// `branch`: a branch, remote branch, tag or revision range such as `main..feature`, or
    /// `None` for every branch and tag. The first page is read on the background executor.
    /// When the tab is open and already shows `branch`, its other filters (author, path, text)
    /// stay; otherwise the list starts over with only the branch filter.
    pub(crate) fn open_git_log(
        &mut self,
        branch: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::debug!("open git log tab");
        let was_open =
            self.state.tabs.index_of(TAB_ID).is_some() && self.git_tasks.log_view.is_some();
        let unchanged = was_open
            && self
                .log_tab()
                .is_some_and(|tab| tab.filter.branch == branch);
        let filter = LogFilter::branch(branch);
        self.state.tabs.open_log(filter.clone());
        if !was_open {
            self.git_tasks.log_view = Some(LogView::new(window, cx));
        }
        if unchanged {
            self.refresh_git_log_if_stale(cx);
        } else {
            self.set_log_inputs(&filter, window, cx);
            self.restart_git_log(filter, cx);
        }
        cx.notify();
    }

    /// Sets the filter bar's inputs to `filter`.
    fn set_log_inputs(&mut self, filter: &LogFilter, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.git_tasks.log_view.as_ref() else {
            return;
        };
        for (input, value) in [
            (&view.search, filter.text.clone()),
            (&view.author, filter.author.clone().unwrap_or_default()),
            (&view.path, filter.path.clone().unwrap_or_default()),
        ] {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    /// Sets the placeholders again after a language change.
    pub(crate) fn relocalize_git_log(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The "All branches" label is translated: rebuild the options on the next render.
        if let Some(view) = self.git_tasks.log_view.as_mut() {
            view.branch_options_key = None;
        }
        let Some(view) = self.git_tasks.log_view.as_ref() else {
            return;
        };
        for (input, key) in [
            (&view.search, "git_log.search"),
            (&view.author, "git_log.filter.any"),
            (&view.path, "git_log.filter.path_placeholder"),
            (&view.branch_filter, "git_log.filter_branches"),
        ] {
            input.update(cx, |state, cx| state.set_placeholder(t!(key), window, cx));
        }
    }

    fn log_tab(&self) -> Option<&crate::state::git_log::LogTab> {
        self.state
            .tabs
            .index_of(TAB_ID)
            .and_then(|index| self.state.tabs.open_tabs().get(index))
            .and_then(|tab| tab.log())
    }

    fn log_tab_mut(&mut self) -> Option<&mut crate::state::git_log::LogTab> {
        let index = self.state.tabs.index_of(TAB_ID)?;
        self.state.tabs.get_mut(index).and_then(|tab| tab.log_mut())
    }

    /// Waits for a pause in the typing, then applies the filter bar. A new keystroke drops the
    /// wait and starts another.
    fn schedule_log_filter(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.git_tasks.log_view.as_mut() else {
            return;
        };
        view.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FILTER_DEBOUNCE).await;
            let _ = this.update(cx, |view, cx| view.apply_log_inputs(cx));
        }));
    }

    /// Reads the filter bar and starts the list over when it differs from the one shown.
    fn apply_log_inputs(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.git_tasks.log_view.as_mut() else {
            return;
        };
        view.debounce = None;
        let (text, author, path) = (
            view.search.read(cx).value().to_string(),
            view.author.read(cx).value().to_string(),
            view.path.read(cx).value().to_string(),
        );
        let Some(tab) = self.log_tab() else {
            return;
        };
        let filter = LogFilter::from_inputs(tab.filter.branch.clone(), &author, &path, &text);
        if filter != tab.filter {
            self.restart_git_log(filter, cx);
            cx.notify();
        }
    }

    /// Filters the log by `branch` (`None` is every branch), keeping the other filters.
    fn set_log_branch(&mut self, branch: Option<String>, cx: &mut Context<Self>) {
        let Some(tab) = self.log_tab() else {
            return;
        };
        if tab.filter.branch == branch {
            return;
        }
        let filter = LogFilter {
            branch,
            ..tab.filter.clone()
        };
        self.restart_git_log(filter, cx);
        cx.notify();
    }

    /// Starts the list over for `filter` and reads its first page.
    fn restart_git_log(&mut self, filter: LogFilter, cx: &mut Context<Self>) {
        if let Some(view) = self.git_tasks.log_view.as_mut() {
            view.rows_scroll = UniformListScrollHandle::new();
            view.files_scroll = UniformListScrollHandle::new();
        }
        let repo = self.git.repo().cloned();
        let head = self
            .git
            .status()
            .and_then(|(_, status)| status.head_oid.clone());
        let Some(tab) = self.log_tab_mut() else {
            return;
        };
        let Some(repo) = repo else {
            // Without a repository there is nothing to read: the tab says so, and reads when
            // the repository shows up (`refresh_git_log_if_stale`).
            tab.restart(filter, None);
            tab.loading = false;
            self.git_tasks.log_load = None;
            return;
        };
        let generation = tab.restart(filter.clone(), head);
        self.read_git_log_page(repo, filter, 0, generation, cx);
    }

    /// Reads the page of `filter` that starts at `skip` on the background executor.
    fn read_git_log_page(
        &mut self,
        repo: Repo,
        filter: LogFilter,
        skip: usize,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let query = filter.query(repo.prefix(), skip);
        self.git_tasks.log_load = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { repo.log(&query) }).await;
            let _ = this.update(cx, |view, cx| {
                view.finish_git_log_page(generation, result, cx);
            });
        }));
    }

    /// Puts a page in the list, and starts reading the detail of the commit that got selected.
    fn finish_git_log_page(
        &mut self,
        generation: u64,
        result: Result<Vec<telaio_git::LogCommit>, telaio_git::GitError>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.log_tab_mut() else {
            return;
        };
        let page = result.map_err(|error| {
            log::warn!("git log failed: {error}");
            error.to_string()
        });
        let outcome = tab.apply_page(generation, page);
        if !outcome.applied {
            return;
        }
        if let Some(oid) = outcome.select {
            self.read_git_log_detail(oid, cx);
        }
        cx.notify();
    }

    /// Asks for the next page. Called from the list's range callback near its end; it does
    /// nothing while a page is being read or when the history is exhausted.
    pub(crate) fn load_more_git_log(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        let Some(tab) = self.log_tab_mut() else {
            return;
        };
        let Some((generation, skip)) = tab.begin_more() else {
            return;
        };
        let filter = tab.filter.clone();
        self.read_git_log_page(repo, filter, skip, generation, cx);
        cx.notify();
    }

    /// Reads the log again when the tab is open and it is out of date: nothing was read yet
    /// (the repository was not known when it opened), or `HEAD` moved since. Called when the
    /// git status changes.
    pub(crate) fn refresh_git_log_if_stale(&mut self, cx: &mut Context<Self>) {
        if self.git.repo().is_none() {
            return;
        }
        let head = self
            .git
            .status()
            .and_then(|(_, status)| status.head_oid.clone());
        let Some(tab) = self.log_tab() else {
            return;
        };
        let stale = if tab.loaded || tab.loading {
            !tab.loading && tab.head != head
        } else {
            true
        };
        if stale {
            let filter = tab.filter.clone();
            self.restart_git_log(filter, cx);
            cx.notify();
        }
    }

    /// Selects the commit `oid` and reads its detail.
    fn select_git_log_commit(&mut self, oid: String, cx: &mut Context<Self>) {
        let Some(tab) = self.log_tab_mut() else {
            return;
        };
        if !tab.select(&oid) {
            return;
        }
        if let Some(view) = self.git_tasks.log_view.as_mut() {
            view.files_scroll = UniformListScrollHandle::new();
        }
        self.read_git_log_detail(oid, cx);
        cx.notify();
    }

    /// Reads the detail of `oid` on the background executor.
    fn read_git_log_detail(&mut self, oid: String, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        self.git_tasks.log_detail = Some(cx.spawn(async move |this, cx| {
            let reading = repo.clone();
            let id = oid.clone();
            let result = cx
                .background_spawn(async move { reading.commit_detail(&id) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.finish_git_log_detail(oid, repo.prefix().to_string(), result, cx);
            });
        }));
    }

    /// Puts a read detail in the tab, with the methods of the requests it touched.
    fn finish_git_log_detail(
        &mut self,
        oid: String,
        prefix: String,
        result: Result<telaio_git::CommitDetail, telaio_git::GitError>,
        cx: &mut Context<Self>,
    ) {
        let detail = result
            .map(|detail| {
                let methods = match self.state.workspace.as_ref() {
                    Some(workspace)
                        if detail
                            .files
                            .iter()
                            .any(|file| file.path.ends_with(".brasa")) =>
                    {
                        request_methods(workspace.tree())
                    }
                    _ => Default::default(),
                };
                DetailView::new(detail, &prefix, &methods)
            })
            .map_err(|error| {
                log::warn!("git commit detail failed: {error}");
                error.to_string()
            });
        if let Some(tab) = self.log_tab_mut()
            && tab.apply_detail(&oid, detail)
        {
            cx.notify();
        }
    }

    /// Opens the diff of `file` for the commit `oid`.
    fn open_log_file_diff(&mut self, oid: &str, file: &DetailFile, cx: &mut Context<Self>) {
        let short = crate::state::git_log::short_oid(oid);
        self.open_git_diff(
            file.repo_path.clone(),
            file.path.clone(),
            DiffTarget::Commit {
                oid: oid.to_string(),
                short,
            },
            file.letter,
            cx,
        );
    }

    /// Keeps the Branch select in step with the branches and the filter. Cheap on most renders:
    /// the options are rebuilt only when the branch list changes (a new `Arc`), and the shown
    /// choice only when the filter's branch differs from what the select last showed.
    fn sync_branch_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let branches = self.git.branches.clone();
        let current = self.log_tab().and_then(|tab| tab.filter.branch.clone());
        let Some(view) = self.git_tasks.log_view.as_mut() else {
            return;
        };
        let key = branches
            .as_ref()
            .map_or(0, |list| Arc::as_ptr(list) as usize);
        if view.branch_options_key != Some(key) {
            let mut options = vec![SelectOption::new(
                ALL_BRANCHES,
                t!("git_log.filter.clear_branch").into_owned(),
            )];
            if let Some(list) = &branches {
                // Local branches, then remote ones, then tags, like the branch column.
                let rank = |kind: &telaio_git::RefKind| match kind {
                    telaio_git::RefKind::Local => 0,
                    telaio_git::RefKind::Remote { .. } => 1,
                    telaio_git::RefKind::Tag => 2,
                };
                let mut refs: Vec<_> = list.branches.iter().collect();
                refs.sort_by_key(|branch| rank(&branch.kind));
                options.extend(
                    refs.into_iter()
                        .map(|branch| SelectOption::new(branch.name.clone(), branch.name.clone())),
                );
            }
            set_options(&view.branch_select, options, true, window, cx);
            view.branch_options_key = Some(key);
            view.synced_branch = None;
        }
        let shown = current.unwrap_or_else(|| ALL_BRANCHES.to_string());
        if view.synced_branch.as_deref() != Some(shown.as_str()) {
            pick(&view.branch_select, &shown, window, cx);
            view.synced_branch = Some(shown);
        }
    }

    /// Renders the log tab.
    pub(crate) fn render_log_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_branch_select(window, cx);
        let palette = cx.palette();
        if self.git.repo().is_none() {
            return div()
                .size_full()
                .bg(palette.bg)
                .child(
                    EmptyState::new(t!("git_log.empty.no_repo"))
                        .icon(Icon::new(IconName::GitBranch))
                        .description(t!("git_log.empty.no_repo_text")),
                )
                .into_any_element();
        }
        let Some(tab) = self.log_tab() else {
            return div().into_any_element();
        };
        let snapshot = TabSnapshot {
            filter: tab.filter.clone(),
            rows: tab.rows.clone(),
            has_more: tab.has_more,
            loaded: tab.loaded,
            error: tab.error.clone(),
            selected: tab.selected.clone(),
            detail: tab.detail.clone(),
            show_graph: tab.shows_graph(),
            graph_width: tab.graph_width(),
        };
        let favorites = self.state.settings.git_favorite_branches.clone();
        let branches = self.git.branches.clone();
        let Some(view) = self.git_tasks.log_view.as_mut() else {
            return div().into_any_element();
        };
        let branch_text = view.branch_filter.read(cx).value().to_string();
        let branch_rows = view.column.rows(
            branches.as_ref(),
            &favorites,
            &branch_text,
            snapshot.filter.branch.as_deref(),
        );
        let Some(view) = self.git_tasks.log_view.as_ref() else {
            return div().into_any_element();
        };

        // The branch and detail columns resize like the sidebar and keep their width when the
        // window or the sidebar changes (`fixed`); the commits take the rest.
        // `h_resizable` sizes itself with `size_full()`, so its parent must have a real size.
        div()
            .size_full()
            .bg(palette.bg)
            .child(
                h_resizable("git-log-layout")
                    .child(
                        resizable_panel()
                            .size(px(BRANCH_COLUMN_WIDTH))
                            .size_range(px(BRANCH_COLUMN_MIN)..px(BRANCH_COLUMN_MAX))
                            .fixed()
                            .flex_none()
                            .child(self.render_log_branches(view, branch_rows, &palette, cx)),
                    )
                    .child(
                        resizable_panel()
                            .child(self.render_log_center(view, &snapshot, &palette, cx)),
                    )
                    .child(
                        resizable_panel()
                            .size(px(DETAIL_WIDTH))
                            .size_range(px(DETAIL_MIN)..px(DETAIL_MAX))
                            .fixed()
                            .flex_none()
                            .child(self.render_log_detail(view, &snapshot, &palette, cx)),
                    ),
            )
            .into_any_element()
    }

    /// The branch column: the filter input, the rows and the hint.
    fn render_log_branches(
        &self,
        view: &LogView,
        rows: Arc<Vec<LogBranchRow>>,
        palette: &Palette,
        cx: &Context<Self>,
    ) -> AnyElement {
        let weak = cx.weak_entity();
        let count = rows.len();
        let list_palette = palette.clone();
        let list = uniform_list("git-log-branches", count, move |range, _, _| {
            range
                .map(|index| branch_row(index, &rows[index], &list_palette, &weak))
                .collect::<Vec<_>>()
        })
        .track_scroll(&view.branches_scroll)
        .size_full();
        let list = with_scrollbar(list.into_any_element(), &view.branches_scroll)
            .flex_1()
            .min_h_0();

        v_flex()
            .size_full()
            .py(px(8.0))
            .px(px(6.0))
            .gap(px(1.0))
            .border_r_1()
            .border_color(palette.border)
            .bg(palette.surface)
            .child(div().px(px(4.0)).pb(px(6.0)).child(pill_input(
                "git-log-branch-filter",
                Some(IconName::ListFilter),
                None,
                &view.branch_filter,
                None,
                palette,
                cx,
            )))
            .child(list)
            .child(
                div()
                    .flex_none()
                    .p(px(8.0))
                    .text_size(px(11.5))
                    .line_height(relative(1.5))
                    .text_color(palette.fg_subtle)
                    .child(t!("git_log.branch_hint")),
            )
            .into_any_element()
    }

    /// The middle: the filter bar, the column titles and the commits (or why there are none).
    fn render_log_center(
        &self,
        view: &LogView,
        snapshot: &TabSnapshot,
        palette: &Palette,
        cx: &Context<Self>,
    ) -> AnyElement {
        let weak = cx.weak_entity();
        let body = if let Some(error) = &snapshot.error {
            EmptyState::new(t!("git_log.empty.failed"))
                .icon(Icon::new(IconName::CircleAlert))
                .description(error.clone())
                .into_any_element()
        } else if !snapshot.loaded {
            centered_text(t!("git_log.loading").into_owned(), palette)
        } else if snapshot.rows.is_empty() {
            let (title, text) = if snapshot.filter.narrows() || snapshot.filter.branch.is_some() {
                (
                    t!("git_log.empty.no_match"),
                    t!("git_log.empty.no_match_text"),
                )
            } else {
                (
                    t!("git_log.empty.no_commits"),
                    t!("git_log.empty.no_commits_text"),
                )
            };
            EmptyState::new(title)
                .icon(Icon::new(IconName::GitGraph))
                .description(text)
                .into_any_element()
        } else {
            self.render_log_rows(view, snapshot, palette, cx)
        };

        let left = if snapshot.show_graph {
            snapshot.graph_width
        } else {
            COLUMN_GAP
        };
        let [author_width, date_width, hash_width] = view.columns;
        // A sized column's title, with a drag handle laid over the gap before it, so the handle
        // takes no room and the titles stay aligned with the rows. Dragging it left widens the
        // column.
        let border_strong = palette.border_strong;
        let sized_title = |column: usize, label: String, width: f32| {
            div()
                .relative()
                .flex_none()
                .h_full()
                .flex()
                .items_center()
                .w(px(width))
                .child(div().min_w_0().truncate().child(label))
                .child(
                    div()
                        .id(("git-log-column-handle", column))
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(-(COLUMN_GAP + COLUMN_HANDLE_WIDTH) / 2.0))
                        .w(px(COLUMN_HANDLE_WIDTH))
                        .flex()
                        .justify_center()
                        .cursor_col_resize()
                        .child(
                            div()
                                .w(px(1.0))
                                .h_full()
                                .hover(move |style| style.bg(border_strong)),
                        )
                        .on_drag(ColumnDrag(column), |drag, _, _, cx| {
                            cx.new(|_| drag.clone())
                        }),
                )
        };
        let titles = h_flex()
            .id("git-log-titles")
            .flex_none()
            .h(px(TITLES_HEIGHT))
            .pl(px(left))
            .pr(px(COLUMN_GAP))
            .gap(px(COLUMN_GAP))
            .items_center()
            .border_b_1()
            .border_color(palette.border)
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(px(11.5))
            .text_color(palette.fg_subtle)
            .on_drag_move::<ColumnDrag>({
                let weak = weak.clone();
                move |event, _, cx| {
                    let column = event.drag(cx).0;
                    let right = f32::from(event.bounds.right()) - COLUMN_GAP;
                    let x = f32::from(event.event.position.x);
                    let _ = weak.update(cx, |view, cx| {
                        if let Some(log) = view.git_tasks.log_view.as_mut() {
                            log.columns = resize_column(log.columns, column, right, x, COLUMN_GAP);
                            cx.notify();
                        }
                    });
                }
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(SUBJECT_MIN_WIDTH))
                    .truncate()
                    .child(t!("git_log.col.subject")),
            )
            .child(
                sized_title(0, t!("git_log.col.author").into_owned(), author_width)
                    .flex_shrink(1.0)
                    .min_w_0(),
            )
            .child(
                sized_title(1, t!("git_log.col.date").into_owned(), date_width)
                    .flex_shrink(1.0)
                    .min_w_0(),
            )
            .child(sized_title(
                2,
                t!("git_log.col.commit").into_owned(),
                hash_width,
            ));

        v_flex()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .child(self.render_log_filter_bar(view, palette, cx))
            .child(titles)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// The search input, the Branch select, the Author and Path filters and the refresh button.
    fn render_log_filter_bar(
        &self,
        view: &LogView,
        palette: &Palette,
        cx: &Context<Self>,
    ) -> AnyElement {
        let branch_chip = h_flex()
            .flex_none()
            .gap(px(6.0))
            .items_center()
            .child(
                div()
                    .text_color(palette.fg_muted)
                    .child(t!("git_log.filter.branch")),
            )
            .child(
                SearchableSelect::new("git-log-branch-select", &view.branch_select)
                    .width(BRANCH_SELECT_WIDTH),
            );
        let weak = cx.weak_entity();
        let refresh = {
            let weak = weak.clone();
            IconButton::new("git-log-refresh", IconName::RefreshCw)
                .tooltip(t!("git_log.refresh"))
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |view, cx| {
                        if let Some(tab) = view.log_tab() {
                            let filter = tab.filter.clone();
                            view.restart_git_log(filter, cx);
                            cx.notify();
                        }
                    });
                })
        };
        h_flex()
            .flex_none()
            .h(px(FILTER_BAR_HEIGHT))
            .px(px(12.0))
            .gap(px(8.0))
            .items_center()
            .overflow_hidden()
            .border_b_1()
            .border_color(palette.border)
            .child(pill_input(
                "git-log-search",
                Some(IconName::Search),
                None,
                &view.search,
                Some(SEARCH_WIDTH),
                palette,
                cx,
            ))
            .child(branch_chip)
            .child(pill_input(
                "git-log-author",
                None,
                Some(t!("git_log.filter.author").into_owned()),
                &view.author,
                Some(AUTHOR_WIDTH + 70.0),
                palette,
                cx,
            ))
            .child(pill_input(
                "git-log-path",
                None,
                Some(t!("git_log.filter.path").into_owned()),
                &view.path,
                Some(AUTHOR_WIDTH + 50.0),
                palette,
                cx,
            ))
            .child(div().flex_none().child(refresh))
            .into_any_element()
    }

    /// The virtualized commit list with the graph painted on the left of each row.
    fn render_log_rows(
        &self,
        view: &LogView,
        snapshot: &TabSnapshot,
        palette: &Palette,
        cx: &Context<Self>,
    ) -> AnyElement {
        let rows = snapshot.rows.clone();
        let count = rows.len();
        let context = RowContext {
            palette: palette.clone(),
            colors: graph_colors(palette),
            mono: cx.theme().mono_font_family.clone(),
            selected: snapshot.selected.clone(),
            show_graph: snapshot.show_graph,
            graph_width: snapshot.graph_width,
            columns: view.columns,
            weak: cx.weak_entity(),
        };
        let has_more = snapshot.has_more;
        let weak = cx.weak_entity();
        let list = uniform_list("git-log-rows", count, move |range, _, cx| {
            // Near the end of what is loaded: ask for the next page after this frame. The call
            // does nothing while one is being read.
            if has_more && range.end + LOAD_AHEAD >= count {
                let weak = weak.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |view, cx| view.load_more_git_log(cx));
                });
            }
            let now = chrono::Utc::now().timestamp();
            range
                .map(|index| context.row(index, &rows, now))
                .collect::<Vec<_>>()
        })
        .track_scroll(&view.rows_scroll)
        .size_full();
        // A click on a row focuses the list (gpui focuses a tracked handle on mouse down).
        div()
            .size_full()
            .track_focus(&view.rows_focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                view.on_git_log_key(event, cx);
            }))
            .child(with_scrollbar(list.into_any_element(), &view.rows_scroll).size_full())
            .into_any_element()
    }

    /// The selected commit's detail.
    fn render_log_detail(
        &self,
        view: &LogView,
        snapshot: &TabSnapshot,
        palette: &Palette,
        cx: &Context<Self>,
    ) -> AnyElement {
        let column = v_flex()
            .size_full()
            .border_l_1()
            .border_color(palette.border)
            .bg(palette.bg);
        match &snapshot.detail {
            Detail::None => column
                .child(centered_text(
                    t!("git_log.detail.select").into_owned(),
                    palette,
                ))
                .into_any_element(),
            Detail::Loading => column
                .child(centered_text(
                    t!("git_log.detail.loading").into_owned(),
                    palette,
                ))
                .into_any_element(),
            Detail::Failed(message) => column
                .child(
                    EmptyState::new(t!("git_log.detail.failed"))
                        .icon(Icon::new(IconName::CircleAlert))
                        .description(message.clone())
                        .compact(true),
                )
                .into_any_element(),
            Detail::Ready(detail) => {
                // The message and the changed files share the column; the line between them
                // resizes the message, which keeps its height when the window changes.
                let files = self.render_log_files(view, detail, palette, cx);
                column
                    .child(
                        div().flex_1().min_h_0().w_full().child(
                            v_resizable("git-log-detail")
                                .with_state(&view.detail_split)
                                .child(
                                    resizable_panel()
                                        .size(px(DETAIL_HEADER_HEIGHT))
                                        .size_range(px(DETAIL_HEADER_MIN)..px(DETAIL_HEADER_MAX))
                                        .fixed()
                                        .child(
                                            self.render_log_commit_header(
                                                view, detail, palette, cx,
                                            ),
                                        ),
                                )
                                .child(resizable_panel().child(files)),
                        ),
                    )
                    .into_any_element()
            }
        }
    }

    /// The subject, message, author, date, commit and parents, and the two buttons.
    fn render_log_commit_header(
        &self,
        view: &LogView,
        detail: &Arc<DetailView>,
        palette: &Palette,
        cx: &Context<Self>,
    ) -> AnyElement {
        let weak = cx.weak_entity();
        let mono = cx.theme().mono_font_family.clone();
        let label = |text: String| div().text_color(palette.fg_subtle).child(text);

        let copy = {
            let oid = detail.oid.clone();
            h_flex()
                .id("git-log-copy-id")
                .gap(px(6.0))
                .items_center()
                .font_family(mono.clone())
                .cursor_pointer()
                .child(detail.short.clone())
                .child(
                    Icon::new(IconName::Copy)
                        .size(px(12.0))
                        .text_color(palette.fg_subtle),
                )
                .tooltip(move |window, cx| {
                    Tooltip::new(SharedString::from(t!("git_log.detail.copy"))).build(window, cx)
                })
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(oid.clone()));
                })
        };
        let parents = h_flex()
            .gap(px(8.0))
            .flex_wrap()
            .font_family(mono)
            .children(detail.parents.iter().enumerate().map(|(index, parent)| {
                let weak = weak.clone();
                let oid = parent.oid.clone();
                div()
                    .id(("git-log-parent", index))
                    .cursor_pointer()
                    .text_color(palette.accent_text)
                    .child(parent.short.clone())
                    .on_click(move |_, _, cx| {
                        let oid = oid.clone();
                        let _ = weak.update(cx, |view, cx| view.go_to_git_log_commit(oid, cx));
                    })
            }));

        // A plain two column layout: the label column is fixed, the value takes the rest.
        let row = |label: AnyElement, value: AnyElement| {
            h_flex()
                .gap(px(8.0))
                .items_start()
                .child(div().flex_none().w(px(64.0)).child(label))
                .child(div().flex_1().min_w_0().child(value))
        };
        let facts = v_flex()
            .gap(px(6.0))
            .mt(px(4.0))
            .text_size(px(12.0))
            .child(row(
                label(t!("git_log.detail.author").into_owned()).into_any_element(),
                div().child(detail.author.clone()).into_any_element(),
            ))
            .child(row(
                label(t!("git_log.detail.date").into_owned()).into_any_element(),
                div().child(full_date(detail.date)).into_any_element(),
            ))
            .child(row(
                label(t!("git_log.detail.commit").into_owned()).into_any_element(),
                copy.into_any_element(),
            ))
            .when(!detail.parents.is_empty(), |facts| {
                facts.child(row(
                    label(plural("git_log.detail.parent", detail.parents.len())).into_any_element(),
                    parents.into_any_element(),
                ))
            });

        let buttons = {
            let (new_weak, checkout_weak) = (weak.clone(), weak);
            let (new_oid, checkout_oid) = (detail.oid.clone(), detail.oid.clone());
            h_flex()
                .flex_wrap()
                .gap(px(6.0))
                .mt(px(4.0))
                .child(
                    SecondaryButton::new("git-log-new-branch", t!("git_log.detail.new_branch"))
                        .icon(IconName::GitBranchPlus)
                        .on_click(move |_, window, cx| {
                            let oid = new_oid.clone();
                            let _ = new_weak.update(cx, |view, cx| {
                                view.new_branch_dialog(Some(oid), String::new(), window, cx);
                            });
                        }),
                )
                .child(
                    SecondaryButton::new(
                        "git-log-checkout-revision",
                        t!("git_log.detail.checkout_revision"),
                    )
                    .icon(IconName::Undo2)
                    .on_click(move |_, window, cx| {
                        let oid = checkout_oid.clone();
                        let _ = checkout_weak.update(cx, |view, cx| {
                            view.request_checkout(CheckoutTarget::Revision(oid), window, cx);
                        });
                    }),
                )
        };

        // A longer message than the panel scrolls inside it, with a scrollbar.
        let content = v_flex()
            .id("git-log-commit-header")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&view.header_scroll)
            .p(px(16.0))
            .gap(px(10.0))
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(relative(1.35))
                    .child(detail.subject.clone()),
            )
            .when(!detail.body.is_empty(), |header| {
                header.child(
                    div()
                        .text_color(palette.fg_muted)
                        .line_height(relative(1.5))
                        .child(detail.body.clone()),
                )
            })
            .child(facts)
            .child(buttons);
        v_flex()
            .relative()
            .size_full()
            .child(content)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(Scrollbar::vertical(&view.header_scroll)),
            )
            .into_any_element()
    }

    /// Moves the selection `offset` rows within the loaded list, scrolls to it and reads its
    /// detail. Reading a detail replaces the previous read, so holding a key only lands the last.
    fn move_git_log_selection(&mut self, offset: isize, cx: &mut Context<Self>) {
        let Some(tab) = self.log_tab() else {
            return;
        };
        let Some(index) = tab.row_from_selected(offset) else {
            return;
        };
        let oid = tab.rows[index].commit.oid.clone();
        if let Some(view) = self.git_tasks.log_view.as_ref() {
            view.rows_scroll
                .scroll_to_item(index, ScrollStrategy::Nearest);
        }
        self.select_git_log_commit(oid, cx);
    }

    /// The keys of the focused commit list: Up and Down move one row, Page Up and Page Down
    /// one screen, Home and End to the first and last loaded row.
    fn on_git_log_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.shift {
            return;
        }
        let page = || {
            let height = self.git_tasks.log_view.as_ref().map_or(0.0, |view| {
                f32::from(view.rows_scroll.0.borrow().base_handle.bounds().size.height)
            });
            ((height / ROW_HEIGHT) as isize - 1).max(1)
        };
        let offset = match keystroke.key.as_str() {
            "up" => -1,
            "down" => 1,
            "pageup" => -page(),
            "pagedown" => page(),
            "home" => isize::MIN,
            "end" => isize::MAX,
            _ => return,
        };
        cx.stop_propagation();
        self.move_git_log_selection(offset, cx);
    }

    /// Selects `oid` and scrolls to it when it is in the loaded list.
    fn go_to_git_log_commit(&mut self, oid: String, cx: &mut Context<Self>) {
        let index = self.log_tab().and_then(|tab| tab.index_of(&oid));
        if let (Some(index), Some(view)) = (index, self.git_tasks.log_view.as_ref()) {
            view.rows_scroll
                .scroll_to_item(index, ScrollStrategy::Center);
        }
        self.select_git_log_commit(oid, cx);
    }

    /// The "Changed files" list of the selected commit.
    fn render_log_files(
        &self,
        view: &LogView,
        detail: &Arc<DetailView>,
        palette: &Palette,
        cx: &Context<Self>,
    ) -> AnyElement {
        let weak = cx.weak_entity();
        let mono = cx.theme().mono_font_family.clone();
        let count = detail.files.len();
        let list_detail = detail.clone();
        let list_palette = palette.clone();
        let list = uniform_list("git-log-files", count, move |range, _, _| {
            range
                .map(|index| {
                    file_row(
                        index,
                        &list_detail,
                        &list_palette,
                        mono.clone(),
                        weak.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&view.files_scroll)
        .size_full();
        let list = with_scrollbar(list.into_any_element(), &view.files_scroll)
            .flex_1()
            .min_h_0();

        let title = if count == 0 {
            t!("git_log.detail.no_files").into_owned()
        } else {
            plural("git_log.detail.files", count)
        };
        v_flex()
            .size_full()
            .px(px(10.0))
            .pt(px(12.0))
            .child(
                div()
                    .flex_none()
                    .px(px(6.0))
                    .pb(px(6.0))
                    .child(SectionLabel::new(title)),
            )
            .child(list)
            .when(count > 0, |files| {
                files.child(
                    div()
                        .flex_none()
                        .px(px(6.0))
                        .py(px(8.0))
                        .text_size(px(11.5))
                        .text_color(palette.fg_subtle)
                        .child(t!("git_log.detail.files_hint")),
                )
            })
            .into_any_element()
    }
}

/// What render reads from the tab, copied once per frame (every part is O(1) or small).
struct TabSnapshot {
    filter: LogFilter,
    rows: Arc<Vec<LogRow>>,
    has_more: bool,
    loaded: bool,
    error: Option<String>,
    selected: Option<String>,
    detail: Detail,
    show_graph: bool,
    graph_width: f32,
}

/// The colors the lanes cycle through, see the module documentation.
fn graph_colors(palette: &Palette) -> [Hsla; GRAPH_COLORS] {
    [
        palette.accent_text,
        palette.success,
        palette.method_color(&Method::Patch),
        palette.info,
        palette.warning,
        palette.fg_muted,
    ]
}

/// A line of subtle text in the middle of the available room.
fn centered_text(text: String, palette: &Palette) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(palette.fg_subtle)
        .child(text)
        .into_any_element()
}

/// The column a drag on a title handle resizes: 0 Author, 1 Date, 2 Commit.
#[derive(Clone)]
struct ColumnDrag(usize);

impl Render for ColumnDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // The pointer and the column itself show the drag, there is no preview.
        div()
    }
}

/// `content` with a vertical scrollbar laid over it, which shows while scrolling or hovering
/// like every scrollbar of the app.
fn with_scrollbar<H: ScrollbarHandle + Clone>(content: AnyElement, handle: &H) -> Div {
    div().relative().child(content).child(
        div()
            .absolute()
            .inset_0()
            .child(Scrollbar::vertical(handle)),
    )
}

/// A bordered pill with a borderless input inside, like the sidebar's filter, with an optional
/// icon or label before it.
fn pill_input(
    id: &'static str,
    icon: Option<IconName>,
    label: Option<String>,
    state: &Entity<InputState>,
    width: Option<f32>,
    palette: &Palette,
    cx: &App,
) -> AnyElement {
    h_flex()
        .id(id)
        // Shares the bar: grows up to `width`, shrinks down to a usable minimum.
        .flex_1()
        .min_w(px(PILL_MIN_WIDTH))
        .when_some(width, |pill, width| pill.max_w(px(width)))
        .h(px(SIDEBAR_FILTER_HEIGHT))
        .items_center()
        .gap_2()
        .px(px(9.0))
        .rounded(px(RADIUS_MD - 1.0))
        .bg(palette.raised)
        .border_1()
        .border_color(palette.border)
        .hover({
            let border_strong = palette.border_strong;
            move |style| style.border_color(border_strong)
        })
        .text_color(palette.fg_subtle)
        .children(icon.map(|icon| Icon::new(icon).small()))
        .children(label.map(|label| div().flex_none().text_color(palette.fg_muted).child(label)))
        .child(
            Input::new(state)
                .context_menu(edit_menu(state, cx))
                .pl_0()
                .pr(px(INPUT_PADDING_X))
                .h(px(SIDEBAR_FILTER_HEIGHT))
                .py_0()
                .bordered(false)
                .text_color(palette.fg)
                .bg(palette.bg.opacity(0.0)),
        )
        .into_any_element()
}

/// "Oct 26, 2026, 22:08" style date and time in the local zone, `YYYY-MM-DD HH:MM`.
fn full_date(unix_seconds: i64) -> String {
    chrono::DateTime::from_timestamp(unix_seconds, 0)
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// One row of the branch column.
fn branch_row(
    index: usize,
    row: &LogBranchRow,
    palette: &Palette,
    weak: &WeakEntity<AppView>,
) -> AnyElement {
    match row {
        LogBranchRow::All { selected } => {
            let weak = weak.clone();
            TreeRow::new(("git-log-branch", index), t!("git_log.all_branches"))
                .selected(*selected)
                .leading(
                    Icon::new(IconName::GitFork)
                        .small()
                        .text_color(palette.accent_text),
                )
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |view, cx| view.set_log_branch(None, cx));
                })
                .into_any_element()
        }
        LogBranchRow::Header(section) => {
            let text = match section {
                LogSection::Local => t!("git_log.section.local").into_owned(),
                LogSection::Remote(remote) => {
                    format!("{} \u{b7} {remote}", t!("git_log.section.remote"))
                }
                LogSection::Tags => t!("git_log.section.tags").into_owned(),
            };
            h_flex()
                .h(px(BRANCH_ROW_HEIGHT))
                .px(px(8.0))
                .items_center()
                .child(SectionLabel::new(text))
                .into_any_element()
        }
        LogBranchRow::Ref(reference) => ref_row(index, reference, palette, weak),
    }
}

/// A branch, remote branch or tag: click filters the log, double click checks it out, right
/// click offers the same and "New branch from…".
fn ref_row(
    index: usize,
    reference: &LogBranchRef,
    palette: &Palette,
    weak: &WeakEntity<AppView>,
) -> AnyElement {
    let (icon, color) = if reference.current {
        (IconName::Check, palette.accent_text)
    } else if reference.favorite {
        (IconName::Star, palette.warning)
    } else {
        let icon = match reference.class {
            RefClass::Local => IconName::GitBranch,
            RefClass::Remote => IconName::Cloud,
            RefClass::Tag => IconName::Tag,
        };
        (icon, palette.fg_subtle)
    };
    let target = checkout_target(reference);
    let name = reference.name.clone();
    let click = {
        let (weak, name, target) = (weak.clone(), name.clone(), target.clone());
        move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
            if event.click_count() >= 2 {
                let target = target.clone();
                let _ = weak.update(cx, |view, cx| view.request_checkout(target, window, cx));
            } else {
                let name = name.clone();
                let _ = weak.update(cx, |view, cx| view.set_log_branch(Some(name), cx));
            }
        }
    };
    let mut row = TreeRow::new(("git-log-branch", index), reference.name.clone())
        .selected(reference.selected)
        .leading(Icon::new(icon).small().text_color(color))
        .on_click(click);
    if !reference.sync.is_empty() {
        row = row.suffix(
            div()
                .text_size(px(11.0))
                .text_color(palette.accent_text)
                .child(reference.sync.clone()),
        );
    }
    let weak = weak.clone();
    div()
        .id(("git-log-branch-menu", index))
        .child(row)
        .context_menu(move |menu, _, _| {
            let (checkout_weak, from_weak, show_weak) = (weak.clone(), weak.clone(), weak.clone());
            let (target, start, shown) = (target.clone(), name.clone(), name.clone());
            menu.item(PopupMenuItem::new(t!("git_log.menu.checkout")).on_click(
                move |_, window, cx| {
                    let target = target.clone();
                    let _ = checkout_weak
                        .update(cx, |view, cx| view.request_checkout(target, window, cx));
                },
            ))
            .item(
                PopupMenuItem::new(t!("git_log.menu.new_branch_from")).on_click(
                    move |_, window, cx| {
                        let start = start.clone();
                        let _ = from_weak.update(cx, |view, cx| {
                            view.new_branch_dialog(Some(start), String::new(), window, cx);
                        });
                    },
                ),
            )
            .separator()
            .item(
                PopupMenuItem::new(t!("git_log.menu.show_log")).on_click(move |_, _, cx| {
                    let shown = shown.clone();
                    let _ = show_weak.update(cx, |view, cx| view.set_log_branch(Some(shown), cx));
                }),
            )
        })
        .into_any_element()
}

/// What checking out a row of the branch column does: a tag leaves `HEAD` detached, a branch
/// (remote ones through their local tracking branch) is checked out.
fn checkout_target(reference: &LogBranchRef) -> CheckoutTarget {
    match reference.class {
        RefClass::Tag => CheckoutTarget::Revision(reference.name.clone()),
        RefClass::Local | RefClass::Remote => CheckoutTarget::Branch(reference.name.clone()),
    }
}

/// What every commit row reads, cloned once per frame into the list's closure.
struct RowContext {
    palette: Palette,
    colors: [Hsla; GRAPH_COLORS],
    mono: SharedString,
    selected: Option<String>,
    show_graph: bool,
    graph_width: f32,
    /// The Author, Date and Commit widths of the column titles.
    columns: [f32; 3],
    weak: WeakEntity<AppView>,
}

impl RowContext {
    /// Row `index` of `rows`: the graph, the subject with its badges, the author, the date and
    /// the short hash.
    fn row(&self, index: usize, rows: &Arc<Vec<LogRow>>, now: i64) -> AnyElement {
        let palette = &self.palette;
        let row = &rows[index];
        let commit = &row.commit;
        let selected = self.selected.as_deref() == Some(commit.oid.as_str());
        let left = if self.show_graph {
            self.graph_width
        } else {
            COLUMN_GAP
        };
        let subject_color = if commit.parents.len() > 1 {
            palette.fg_muted
        } else {
            palette.fg
        };

        let badges = row
            .badges
            .iter()
            .map(|(badge, name)| ref_badge(*badge, name.clone(), palette, self.mono.clone()))
            .chain((row.hidden > 0).then(|| more_badge(row.hidden, palette, self.mono.clone())));
        let subject = h_flex()
            .flex_1()
            .min_w(px(SUBJECT_MIN_WIDTH))
            .overflow_hidden()
            .gap(px(6.0))
            .items_center()
            .children(badges)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(subject_color)
                    .child(commit.subject.clone()),
            );

        let graph = self.show_graph.then(|| {
            let (rows, colors) = (rows.clone(), self.colors);
            let hollow = if selected {
                palette.list_active
            } else {
                palette.bg
            };
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    paint_graph(bounds, &rows[index], &colors, hollow, window);
                },
            )
            .absolute()
            .left_0()
            .top_0()
            .w(px(self.graph_width))
            .h(px(ROW_HEIGHT))
        });

        let oid = commit.oid.clone();
        let weak = self.weak.clone();
        let hover = palette.hover;
        h_flex()
            .id(("git-log-row", index))
            .relative()
            .w_full()
            .h(px(ROW_HEIGHT))
            .pl(px(left))
            .pr(px(COLUMN_GAP))
            .gap(px(COLUMN_GAP))
            .items_center()
            .cursor_pointer()
            .when(selected, |row| row.bg(palette.list_active))
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .children(graph)
            .child(subject)
            .child(
                div()
                    .flex_shrink(1.0)
                    .min_w_0()
                    .w(px(self.columns[0]))
                    .truncate()
                    .text_color(palette.fg_muted)
                    .child(commit.author_name.clone()),
            )
            .child(
                div()
                    .flex_shrink(1.0)
                    .min_w_0()
                    .w(px(self.columns[1]))
                    .truncate()
                    .text_color(palette.fg_muted)
                    .child(relative_time(commit.date, now)),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(self.columns[2]))
                    .font_family(self.mono.clone())
                    .text_size(px(12.0))
                    .text_color(palette.fg_subtle)
                    .child(commit.short.clone()),
            )
            .on_click({
                let (weak, oid) = (weak.clone(), oid.clone());
                move |_, _, cx| {
                    let oid = oid.clone();
                    let _ = weak.update(cx, |view, cx| view.select_git_log_commit(oid, cx));
                }
            })
            .context_menu(move |menu, _, _| {
                let (new_weak, checkout_weak) = (weak.clone(), weak.clone());
                let (new_oid, checkout_oid, copy_oid) = (oid.clone(), oid.clone(), oid.clone());
                menu.item(
                    PopupMenuItem::new(t!("git_log.menu.new_branch_here")).on_click(
                        move |_, window, cx| {
                            let oid = new_oid.clone();
                            let _ = new_weak.update(cx, |view, cx| {
                                view.new_branch_dialog(Some(oid), String::new(), window, cx);
                            });
                        },
                    ),
                )
                .item(
                    PopupMenuItem::new(t!("git_log.menu.checkout_revision")).on_click(
                        move |_, window, cx| {
                            let oid = checkout_oid.clone();
                            let _ = checkout_weak.update(cx, |view, cx| {
                                view.request_checkout(CheckoutTarget::Revision(oid), window, cx);
                            });
                        },
                    ),
                )
                .separator()
                .item(
                    PopupMenuItem::new(t!("git_log.menu.copy_id")).on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_oid.clone()));
                    }),
                )
            })
            .into_any_element()
    }
}

/// A ref badge (design 4a): `HEAD` filled with the accent, a local branch tinted, a remote one
/// outlined with a cloud, a tag tinted with the warning color.
fn ref_badge(badge: RefBadge, name: String, palette: &Palette, mono: SharedString) -> AnyElement {
    let (color, background, ring, icon) = match badge {
        RefBadge::Head => (palette.accent_fg, palette.accent, None, IconName::GitBranch),
        RefBadge::Local => (
            palette.accent_text,
            palette.accent_subtle,
            None,
            IconName::GitBranch,
        ),
        RefBadge::Remote => (
            palette.info,
            palette.info.opacity(0.0),
            Some(palette.info.opacity(0.45)),
            IconName::Cloud,
        ),
        RefBadge::Tag => (palette.warning, palette.warning_subtle, None, IconName::Tag),
    };
    h_flex()
        .flex_none()
        .h(px(18.0))
        .px(px(6.0))
        .gap(px(4.0))
        .items_center()
        .rounded(px(RADIUS_SM))
        .font_family(mono)
        .text_size(px(11.0))
        .text_color(color)
        .bg(background)
        .when_some(ring, |badge, ring| badge.border_1().border_color(ring))
        .child(Icon::new(icon).size(px(10.0)))
        .child(name)
        .into_any_element()
}

/// The "+N" badge for the refs a row does not show.
fn more_badge(hidden: usize, palette: &Palette, mono: SharedString) -> AnyElement {
    h_flex()
        .flex_none()
        .h(px(18.0))
        .px(px(6.0))
        .items_center()
        .rounded(px(RADIUS_SM))
        .font_family(mono)
        .text_size(px(11.0))
        .text_color(palette.fg_muted)
        .bg(palette.raised)
        .child(format!("+{hidden}"))
        .into_any_element()
}

/// Paints the lanes of `row` and its dot inside `bounds` (one row of the list). Lines are 2 px
/// strokes: straight when the lane stays, a cubic curve between the edge of the row and the dot
/// when it moves. The dot of the commit `HEAD` points at is larger and hollow.
fn paint_graph(
    bounds: Bounds<Pixels>,
    row: &LogRow,
    colors: &[Hsla; GRAPH_COLORS],
    hollow: Hsla,
    window: &mut Window,
) {
    let graph: &GraphRow = &row.graph;
    let top = bounds.origin.y;
    let middle = top + bounds.size.height / 2.0;
    let bottom = top + bounds.size.height;
    let x = |lane: usize| bounds.origin.x + px(lane_x(lane.min(MAX_LANES - 1)));
    let color_of = |index: usize| colors[index % GRAPH_COLORS];

    for line in &graph.lines {
        let (start, end) = match line.part {
            LinePart::Top => (point(x(line.from), top), point(x(line.to), middle)),
            LinePart::Bottom => (point(x(line.from), middle), point(x(line.to), bottom)),
        };
        let mut path = PathBuilder::stroke(px(STROKE));
        path.move_to(start);
        if line.from == line.to {
            path.line_to(end);
        } else {
            let reach = (end.y - start.y) * 0.6;
            path.cubic_bezier_to(
                end,
                point(start.x, start.y + reach),
                point(end.x, end.y - reach),
            );
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, color_of(line.color));
        }
    }

    let radius = px(if row.head {
        HEAD_DOT_RADIUS
    } else {
        DOT_RADIUS
    });
    let color = color_of(graph.color);
    window.paint_quad(quad(
        Bounds {
            origin: point(x(graph.lane) - radius, middle - radius),
            size: size(radius * 2.0, radius * 2.0),
        },
        radius,
        if row.head { hollow } else { color },
        px(STROKE),
        color,
        BorderStyle::Solid,
    ));
}

/// One row of the changed files of a commit: the method of a request, its name and folder, the
/// line counts and the status letter. Click opens the diff of that file for the commit.
fn file_row(
    index: usize,
    detail: &Arc<DetailView>,
    palette: &Palette,
    mono: SharedString,
    weak: WeakEntity<AppView>,
) -> AnyElement {
    let file = &detail.files[index];
    let counts = if file.binary {
        div()
            .flex_none()
            .text_size(px(11.0))
            .text_color(palette.fg_subtle)
            .child(t!("git_log.detail.binary"))
            .into_any_element()
    } else {
        h_flex()
            .flex_none()
            .gap(px(6.0))
            .font_family(mono.clone())
            .text_size(px(11.0))
            .when(file.additions > 0, |counts| {
                counts.child(
                    div()
                        .text_color(palette.success)
                        .child(format!("+{}", format_integer(file.additions as u64))),
                )
            })
            .when(file.deletions > 0, |counts| {
                counts.child(
                    div()
                        .text_color(palette.danger)
                        .child(format!("\u{2212}{}", format_integer(file.deletions as u64))),
                )
            })
            .into_any_element()
    };
    let oid = detail.oid.clone();
    let clicked = file.clone();
    h_flex()
        .id(("git-log-file", index))
        // The list's width, so a long folder truncates instead of pushing the counts out.
        .w_full()
        .h(px(FILE_ROW_HEIGHT))
        .px(px(6.0))
        .gap(px(6.0))
        .items_center()
        .rounded(px(RADIUS_SM))
        .cursor_pointer()
        .hover({
            let hover = palette.hover;
            move |style| style.bg(hover)
        })
        .children(file.method.clone().map(MethodBadge::label))
        .child(
            div()
                .flex_none()
                .max_w(px(150.0))
                .truncate()
                .child(file.name.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.5))
                .text_color(palette.fg_subtle)
                .child(file.folder.clone()),
        )
        .child(counts)
        .child(
            div()
                .flex_none()
                .w(px(12.0))
                .text_center()
                .font_family(mono)
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(super::panel::letter_color(file.letter, palette))
                .child(file.letter.to_string()),
        )
        .on_click(move |_, _, cx| {
            let (oid, clicked) = (oid.clone(), clicked.clone());
            let _ = weak.update(cx, |view, cx| view.open_log_file_diff(&oid, &clicked, cx));
        })
        .into_any_element()
}
