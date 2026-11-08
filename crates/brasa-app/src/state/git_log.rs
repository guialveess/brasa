//! The Git log tab (#174, design 4a): the filter, the pages of commits read so far with their
//! graph layout, the selected commit and its detail, and the rows of the branch column. Plain
//! data: `views/git/log_tab.rs` calls `telaio-git` on the background executor and puts the
//! results here, render only reads them.
//!
//! The log is read in pages of [`PAGE_SIZE`] commits. Every page is laid out once when it
//! arrives (the [`GraphLayout`] continues across pages), so render never costs in proportion to
//! the history. Changing the filter bumps [`LogTab::generation`]: a page of an older generation
//! is dropped when it arrives.

use std::collections::HashMap;
use std::sync::Arc;

use brasa_core::Method;
use brasa_workspace::{Node, natural_cmp};
use telaio_git::{
    CommitDetail, GraphLayout, GraphRow, LogCommit, LogQuery, RefKind, RefLabel, RefLabelKind,
};

use crate::state::git::BranchList;
use crate::state::git_branches::sync_label;

/// The id of the log tab. There is one: opening the log again reuses it.
pub const TAB_ID: &str = "git:log";

/// How many commits one read asks for. A page that comes back full means there may be more.
pub const PAGE_SIZE: usize = 200;

/// How many ref badges a row shows before a "+N" badge takes the rest.
pub const MAX_BADGES: usize = 3;

/// The most lanes the graph column draws. Lanes further right are cut: a history this wide is
/// not readable as a drawing anyway.
pub const MAX_LANES: usize = 12;

/// How many colors the graph cycles through (see `views/git/log_tab.rs` for which).
pub const GRAPH_COLORS: usize = 6;

/// The x of the first lane's center, in pixels (design 4a).
const LANE_ORIGIN: f32 = 18.0;
/// The distance between two lanes' centers, in pixels (design 4a).
const LANE_STEP: f32 = 14.0;
/// The narrowest graph column (design 4a: room for three lanes).
const GRAPH_MIN_WIDTH: f32 = 64.0;

/// What the log shows. The branch comes from the branch column (or the Branch chip), the rest
/// from the filter bar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFilter {
    /// Only the history of this branch, tag or revision. `None` is every branch and tag.
    pub branch: Option<String>,
    /// Only commits whose author name or email contains this text.
    pub author: Option<String>,
    /// Only commits that touch this path, relative to the workspace. A folder works.
    pub path: Option<String>,
    /// The message or hash text.
    pub text: String,
}

impl LogFilter {
    /// A filter for the history of `branch`, or of everything for `None`.
    pub fn branch(branch: Option<String>) -> Self {
        LogFilter {
            branch,
            ..LogFilter::default()
        }
    }

    /// The filter of the typed `author`, `path` and `text`: trimmed, empty ones meaning "any".
    pub fn from_inputs(branch: Option<String>, author: &str, path: &str, text: &str) -> Self {
        LogFilter {
            branch,
            author: non_empty(author),
            path: non_empty(path.trim().trim_start_matches("./").trim_matches('/')),
            text: text.trim().to_string(),
        }
    }

    /// Whether anything besides the branch narrows the log. The graph is only drawn when not:
    /// a filtered list skips commits, so its lanes would lead nowhere.
    pub fn narrows(&self) -> bool {
        self.author.is_some() || self.path.is_some() || !self.text.is_empty()
    }

    /// The git query for the page that starts at `skip`. `prefix` is the workspace's place in
    /// the repository ([`telaio_git::Repo::prefix`]), which turns the workspace relative path
    /// into a repository relative one.
    pub fn query(&self, prefix: &str, skip: usize) -> LogQuery {
        LogQuery {
            branch: self.branch.clone(),
            author: self.author.clone(),
            path: self.path.as_ref().map(|path| {
                if prefix.is_empty() {
                    path.clone()
                } else {
                    format!("{prefix}/{path}")
                }
            }),
            text: (!self.text.is_empty()).then(|| self.text.clone()),
            skip,
            limit: PAGE_SIZE,
        }
    }
}

fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Which badge a [`RefLabel`] is drawn as (design 4a): `HEAD` filled with the accent, a local
/// branch tinted, a remote one outlined with a cloud, a tag tinted with the warning color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefBadge {
    Head,
    Local,
    Remote,
    Tag,
}

/// The badge of a kind of ref.
pub fn badge_of(kind: RefLabelKind) -> RefBadge {
    match kind {
        RefLabelKind::Head => RefBadge::Head,
        RefLabelKind::Local => RefBadge::Local,
        RefLabelKind::Remote => RefBadge::Remote,
        RefLabelKind::Tag => RefBadge::Tag,
    }
}

/// The refs of a commit in drawing order: the checked out branch, local branches, remote
/// branches, then tags. Each group keeps git's order.
pub fn ordered_badges(refs: &[RefLabel]) -> Vec<(RefBadge, String)> {
    let mut badges: Vec<(RefBadge, String)> = refs
        .iter()
        .map(|label| (badge_of(label.kind), label.name.clone()))
        .collect();
    let rank = |badge: &RefBadge| match badge {
        RefBadge::Head => 0,
        RefBadge::Local => 1,
        RefBadge::Remote => 2,
        RefBadge::Tag => 3,
    };
    badges.sort_by_key(|(badge, _)| rank(badge));
    badges
}

/// A commit of the list with everything render needs, computed when its page arrived.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRow {
    pub commit: LogCommit,
    /// The lanes to draw around the commit's dot.
    pub graph: GraphRow,
    /// Whether `HEAD` points here: its dot is drawn hollow.
    pub head: bool,
    /// The badges to show, at most [`MAX_BADGES`].
    pub badges: Vec<(RefBadge, String)>,
    /// How many more refs there are than `badges` shows.
    pub hidden: usize,
}

/// The parent of a commit, for the detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentRef {
    pub oid: String,
    pub short: String,
}

/// A file a commit changed, ready for the detail list.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailFile {
    /// The path relative to the repository root, what the diff reads.
    pub repo_path: String,
    /// The path relative to the workspace (the repository path for a file outside it): what the
    /// diff tab shows and, for a request, its id.
    pub path: String,
    /// The name shown: the file name, without the extension for a request.
    pub name: String,
    /// The folder shown after the name, empty for a file at the top.
    pub folder: String,
    /// The status letter: `A`, `M`, `D`, `R`...
    pub letter: char,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    /// The method of a request that is in the workspace.
    pub method: Option<Method>,
}

/// The detail of the selected commit.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailView {
    pub oid: String,
    pub short: String,
    pub subject: String,
    /// The rest of the message, empty for a one line message.
    pub body: String,
    pub author: String,
    pub date: i64,
    pub parents: Vec<ParentRef>,
    pub files: Vec<DetailFile>,
}

impl DetailView {
    /// The detail of `detail`. `prefix` is the workspace's place in the repository and `methods`
    /// the workspace requests' methods by id ([`request_methods`]).
    pub fn new(detail: CommitDetail, prefix: &str, methods: &HashMap<String, Method>) -> Self {
        let (subject, body) = match detail.message.split_once('\n') {
            Some((subject, rest)) => (subject.trim_end(), rest.trim()),
            None => (detail.message.trim(), ""),
        };
        let files = detail
            .files
            .iter()
            .map(|file| {
                let inside = workspace_path(&file.path, prefix);
                let path = inside.unwrap_or(&file.path).to_string();
                let (folder, name) = match path.rsplit_once('/') {
                    Some((folder, name)) => (folder.to_string(), name),
                    None => (String::new(), path.as_str()),
                };
                let is_request = name.ends_with(".brasa");
                DetailFile {
                    repo_path: file.path.clone(),
                    method: inside
                        .filter(|_| is_request)
                        .and_then(|id| methods.get(id))
                        .cloned(),
                    name: name.strip_suffix(".brasa").unwrap_or(name).to_string(),
                    folder,
                    path,
                    letter: file.letter,
                    additions: file.additions,
                    deletions: file.deletions,
                    binary: file.binary,
                }
            })
            .collect();
        DetailView {
            short: short_oid(&detail.oid),
            parents: detail
                .parents
                .iter()
                .map(|oid| ParentRef {
                    oid: oid.clone(),
                    short: short_oid(oid),
                })
                .collect(),
            oid: detail.oid,
            subject: subject.to_string(),
            body: body.to_string(),
            author: detail.author_name,
            date: detail.date,
            files,
        }
    }
}

/// `path` relative to the workspace, or `None` when it is outside it.
fn workspace_path<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(path);
    }
    path.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('/'))
}

/// The abbreviated id the list shows (7 characters).
pub fn short_oid(oid: &str) -> String {
    oid.chars().take(7).collect()
}

/// The method of every request of the workspace tree by id.
pub fn request_methods(tree: &[Node]) -> HashMap<String, Method> {
    fn walk(nodes: &[Node], methods: &mut HashMap<String, Method>) {
        for node in nodes {
            match node {
                Node::Folder(folder) => walk(&folder.children, methods),
                Node::Request(request) => {
                    if let Some(method) = &request.method {
                        methods.insert(request.id.clone(), method.clone());
                    }
                }
            }
        }
    }
    let mut methods = HashMap::new();
    walk(tree, &mut methods);
    methods
}

/// What the detail column shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Detail {
    /// No commit selected.
    #[default]
    None,
    /// The selected commit is being read.
    Loading,
    Ready(Arc<DetailView>),
    /// Why it could not be read (git's message).
    Failed(String),
}

/// What [`LogTab::apply_page`] did with a page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageOutcome {
    /// False when the page was of an older generation and was dropped.
    pub applied: bool,
    /// A commit that became the selection (the first one, when the selection is not in the
    /// list), whose detail has to be read.
    pub select: Option<String>,
}

/// The state of the log tab.
#[derive(Debug, Clone)]
pub struct LogTab {
    pub filter: LogFilter,
    /// Bumped every time the list restarts; a page carries the generation it was asked for.
    pub generation: u64,
    /// The commits read so far, shared so render clones it in O(1).
    pub rows: Arc<Vec<LogRow>>,
    layout: GraphLayout,
    /// The most lanes any loaded row uses, for the width of the graph column.
    pub lanes: usize,
    /// Whether the last page was full, so another may exist.
    pub has_more: bool,
    /// Whether a page is being read.
    pub loading: bool,
    /// Whether the first page of this generation has arrived.
    pub loaded: bool,
    /// Why the last read failed, shown instead of the list.
    pub error: Option<String>,
    /// The commit `HEAD` pointed at when the list was read, to notice that it moved.
    pub head: Option<String>,
    /// The selected commit, full id.
    pub selected: Option<String>,
    pub detail: Detail,
}

impl PartialEq for LogTab {
    fn eq(&self, other: &Self) -> bool {
        self.filter == other.filter
            && self.generation == other.generation
            && self.rows.len() == other.rows.len()
            && self.selected == other.selected
            && self.loading == other.loading
            && self.error == other.error
    }
}

impl LogTab {
    /// A tab for `filter`, nothing read yet. Call [`Self::restart`] to start the first read.
    pub fn new(filter: LogFilter) -> Self {
        LogTab {
            filter,
            generation: 0,
            rows: Arc::default(),
            layout: GraphLayout::new(),
            lanes: 1,
            has_more: false,
            loading: false,
            loaded: false,
            error: None,
            head: None,
            selected: None,
            detail: Detail::None,
        }
    }

    /// Starts the list over for `filter`: forgets the rows and the lanes and returns the
    /// generation the first page has to carry. `head` is the commit `HEAD` points at now.
    pub fn restart(&mut self, filter: LogFilter, head: Option<String>) -> u64 {
        self.filter = filter;
        self.generation += 1;
        self.rows = Arc::default();
        self.layout = GraphLayout::new();
        self.lanes = 1;
        self.has_more = false;
        self.loading = true;
        self.loaded = false;
        self.error = None;
        self.head = head;
        self.generation
    }

    /// The generation and the `skip` of the next page, when there may be one and none is being
    /// read. Marks the tab as loading.
    pub fn begin_more(&mut self) -> Option<(u64, usize)> {
        if self.loading || !self.loaded || !self.has_more {
            return None;
        }
        self.loading = true;
        Some((self.generation, self.rows.len()))
    }

    /// Puts a read page in the list, unless it is of an older generation. A full page means
    /// more may follow. A failed read keeps what is already listed and shows the message only
    /// when nothing is.
    pub fn apply_page(
        &mut self,
        generation: u64,
        page: Result<Vec<LogCommit>, String>,
    ) -> PageOutcome {
        if generation != self.generation {
            return PageOutcome::default();
        }
        self.loading = false;
        let commits = match page {
            Ok(commits) => commits,
            Err(message) => {
                self.has_more = false;
                if self.rows.is_empty() {
                    self.error = Some(message);
                }
                self.loaded = true;
                return PageOutcome {
                    applied: true,
                    select: None,
                };
            }
        };
        self.has_more = commits.len() >= PAGE_SIZE;
        self.loaded = true;
        let rows = Arc::make_mut(&mut self.rows);
        rows.reserve(commits.len());
        for commit in commits {
            let graph = self.layout.push(&commit.oid, &commit.parents);
            self.lanes = self.lanes.max(graph.width);
            let mut badges = ordered_badges(&commit.refs);
            let hidden = badges.len().saturating_sub(MAX_BADGES);
            badges.truncate(MAX_BADGES);
            rows.push(LogRow {
                head: commit
                    .refs
                    .iter()
                    .any(|label| label.kind == RefLabelKind::Head),
                graph,
                badges,
                hidden,
                commit,
            });
        }
        let select = match &self.selected {
            Some(oid) if self.rows.iter().any(|row| &row.commit.oid == oid) => None,
            _ => self.rows.first().map(|row| row.commit.oid.clone()),
        };
        if let Some(oid) = &select {
            self.selected = Some(oid.clone());
            self.detail = Detail::Loading;
        } else if self.rows.is_empty() {
            self.selected = None;
            self.detail = Detail::None;
        }
        PageOutcome {
            applied: true,
            select,
        }
    }

    /// Selects `oid` and marks its detail as being read. False when it already is the selection
    /// with its detail read or loading.
    pub fn select(&mut self, oid: &str) -> bool {
        if self.selected.as_deref() == Some(oid) && self.detail != Detail::None {
            return false;
        }
        self.selected = Some(oid.to_string());
        self.detail = Detail::Loading;
        true
    }

    /// Puts the read detail of `oid` in place, unless another commit was selected meanwhile.
    pub fn apply_detail(&mut self, oid: &str, detail: Result<DetailView, String>) -> bool {
        if self.selected.as_deref() != Some(oid) {
            return false;
        }
        self.detail = match detail {
            Ok(view) => Detail::Ready(Arc::new(view)),
            Err(message) => Detail::Failed(message),
        };
        true
    }

    /// The index of `oid` in the loaded list.
    pub fn index_of(&self, oid: &str) -> Option<usize> {
        self.rows.iter().position(|row| row.commit.oid == oid)
    }

    /// The row `offset` rows away from the selected one, clamped to the loaded list: where the
    /// arrow, page, Home and End keys move the selection. With no selection, or one that is not
    /// loaded, it starts from the first row. `None` when the list is empty.
    pub fn row_from_selected(&self, offset: isize) -> Option<usize> {
        let last = self.rows.len().checked_sub(1)?;
        let Some(current) = self.selected.as_deref().and_then(|oid| self.index_of(oid)) else {
            return Some(0);
        };
        Some(current.saturating_add_signed(offset).min(last))
    }

    /// Whether the graph is drawn: not while the filter narrows the list.
    pub fn shows_graph(&self) -> bool {
        !self.filter.narrows()
    }

    /// The width of the graph column in pixels, 0 when the graph is not drawn.
    pub fn graph_width(&self) -> f32 {
        if self.shows_graph() {
            graph_width(self.lanes)
        } else {
            0.0
        }
    }
}

/// The x of the center of `lane`, in pixels from the left of the graph column.
pub fn lane_x(lane: usize) -> f32 {
    LANE_ORIGIN + lane as f32 * LANE_STEP
}

/// The width of a graph column that holds `lanes` lanes: 64 px up to three lanes, then one
/// lane step more per lane, [`MAX_LANES`] at the most.
pub fn graph_width(lanes: usize) -> f32 {
    let lanes = lanes.clamp(1, MAX_LANES);
    (lane_x(lanes - 1) + LANE_ORIGIN).max(GRAPH_MIN_WIDTH)
}

/// The kind of a row of the branch column's refs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefClass {
    Local,
    Remote,
    Tag,
}

/// A group of the branch column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogSection {
    Local,
    /// The branches of one remote.
    Remote(String),
    Tags,
}

/// A branch, remote branch or tag of the branch column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogBranchRef {
    /// The name git knows it by: `main`, `origin/main`, `v1`.
    pub name: String,
    pub class: RefClass,
    /// Whether it is checked out.
    pub current: bool,
    pub favorite: bool,
    /// "↑1 ↓2", empty when in sync or without an upstream.
    pub sync: String,
    /// Whether the log is filtered by it.
    pub selected: bool,
}

/// One row of the branch column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogBranchRow {
    /// "All branches", selected while no branch filters the log.
    All {
        selected: bool,
    },
    Header(LogSection),
    Ref(LogBranchRef),
}

/// The rows of the branch column: "All branches", then Local, one Remote group per remote and
/// Tags. `filter` narrows the names (ignoring case), `selected` is the branch filtering the log.
/// The current branch leads Local, then the starred ones, then natural order; tags are newest
/// version first. Empty groups are dropped.
pub fn log_branch_rows(
    list: &BranchList,
    favorites: &[String],
    filter: &str,
    selected: Option<&str>,
) -> Vec<LogBranchRow> {
    let needle = filter.trim().to_lowercase();
    let matches = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);
    let make = |branch: &telaio_git::BranchInfo, class: RefClass| LogBranchRef {
        name: branch.name.clone(),
        class,
        current: branch.is_head,
        favorite: favorites.contains(&branch.name),
        sync: sync_label(branch.ahead, branch.behind),
        selected: selected == Some(branch.name.as_str()),
    };
    let sorted = |mut refs: Vec<LogBranchRef>| {
        refs.sort_by(|a, b| {
            b.current
                .cmp(&a.current)
                .then_with(|| b.favorite.cmp(&a.favorite))
                .then_with(|| natural_cmp(&a.name, &b.name))
        });
        refs
    };

    let mut rows = vec![LogBranchRow::All {
        selected: selected.is_none(),
    }];
    let locals = sorted(
        list.branches
            .iter()
            .filter(|branch| branch.kind == RefKind::Local && matches(&branch.name))
            .map(|branch| make(branch, RefClass::Local))
            .collect(),
    );
    if !locals.is_empty() {
        rows.push(LogBranchRow::Header(LogSection::Local));
        rows.extend(locals.into_iter().map(LogBranchRow::Ref));
    }

    let mut remotes: Vec<&str> = list
        .branches
        .iter()
        .filter_map(|branch| match &branch.kind {
            RefKind::Remote { remote } => Some(remote.as_str()),
            _ => None,
        })
        .collect();
    remotes.sort_by(|a, b| natural_cmp(a, b));
    remotes.dedup();
    for remote in remotes {
        let refs = sorted(
            list.branches
                .iter()
                .filter(|branch| {
                    matches!(&branch.kind, RefKind::Remote { remote: r } if r == remote)
                        && matches(&branch.name)
                })
                .map(|branch| make(branch, RefClass::Remote))
                .collect(),
        );
        if !refs.is_empty() {
            rows.push(LogBranchRow::Header(LogSection::Remote(remote.to_string())));
            rows.extend(refs.into_iter().map(LogBranchRow::Ref));
        }
    }

    let mut tags: Vec<LogBranchRef> = list
        .branches
        .iter()
        .filter(|branch| branch.kind == RefKind::Tag && matches(&branch.name))
        .map(|branch| make(branch, RefClass::Tag))
        .collect();
    tags.sort_by(|a, b| natural_cmp(&b.name, &a.name));
    if !tags.is_empty() {
        rows.push(LogBranchRow::Header(LogSection::Tags));
        rows.extend(tags.into_iter().map(LogBranchRow::Ref));
    }
    rows
}

/// The branch column's rows, rebuilt only when what they come from changes: the branches read,
/// the starred ones, the filter text or the selected branch. Render asks it on every frame, and
/// the answer is the same `Arc` until one of them moves.
#[derive(Debug, Default)]
pub struct BranchColumn {
    source: Option<Arc<BranchList>>,
    favorites: Vec<String>,
    text: String,
    selected: Option<String>,
    rows: Arc<Vec<LogBranchRow>>,
    built: bool,
}

impl BranchColumn {
    /// The rows for `list` (none read yet: only "All branches"), see [`log_branch_rows`].
    pub fn rows(
        &mut self,
        list: Option<&Arc<BranchList>>,
        favorites: &[String],
        text: &str,
        selected: Option<&str>,
    ) -> Arc<Vec<LogBranchRow>> {
        let same_source = match (&self.source, list) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !(self.built
            && same_source
            && self.favorites == favorites
            && self.text == text
            && self.selected.as_deref() == selected)
        {
            self.rows = Arc::new(log_branch_rows(
                list.map_or(&BranchList::default(), |list| list.as_ref()),
                favorites,
                text,
                selected,
            ));
            self.source = list.cloned();
            self.favorites = favorites.to_vec();
            self.text = text.to_string();
            self.selected = selected.map(str::to_string);
            self.built = true;
        }
        self.rows.clone()
    }
}

/// How narrow and wide a dragged column of the commit list can get.
pub const COLUMN_MIN: f32 = 48.0;
pub const COLUMN_MAX: f32 = 400.0;

/// The Author, Date and Commit widths after dragging the handle of `column` (0, 1 or 2) to `x`.
/// `right` is where the last column ends, and each column ends `gap` before the next one
/// starts; the handle sits in the middle of that gap. Clamped, so a column never vanishes or
/// eats the row.
pub fn resize_column(
    mut columns: [f32; 3],
    column: usize,
    right: f32,
    x: f32,
    gap: f32,
) -> [f32; 3] {
    let after: f32 = columns[column + 1..].iter().map(|width| width + gap).sum();
    columns[column] = (right - after - x - gap / 2.0).clamp(COLUMN_MIN, COLUMN_MAX);
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dragged_column_ends_where_the_next_one_starts() {
        // Commit ends at 1000, Date (96) ends 12 before Commit (70) starts, at 918.
        let columns = [110.0, 96.0, 70.0];
        assert_eq!(
            resize_column(columns, 2, 1000.0, 900.0, 12.0),
            [110.0, 96.0, 94.0]
        );
        assert_eq!(
            resize_column(columns, 1, 1000.0, 700.0, 12.0),
            [110.0, 212.0, 70.0]
        );
        // Clamped at both ends.
        assert_eq!(
            resize_column(columns, 0, 1000.0, 999.0, 12.0)[0],
            COLUMN_MIN
        );
        assert_eq!(resize_column(columns, 0, 1000.0, 0.0, 12.0)[0], COLUMN_MAX);
    }
    use pretty_assertions::assert_eq;
    use telaio_git::{BranchInfo, CommitFile};

    fn commit(oid: &str, parents: &[&str], refs: Vec<RefLabel>) -> LogCommit {
        LogCommit {
            oid: oid.to_string(),
            short: short_oid(oid),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author_name: "Ana".to_string(),
            author_email: "ana@example.com".to_string(),
            date: 1_000,
            subject: format!("Commit {oid}"),
            refs,
        }
    }

    fn label(kind: RefLabelKind, name: &str) -> RefLabel {
        RefLabel {
            kind,
            name: name.to_string(),
        }
    }

    fn branch(name: &str, kind: RefKind, head: bool) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            kind,
            oid: "abc1234".to_string(),
            subject: String::new(),
            date: 0,
            upstream: None,
            ahead: 0,
            behind: 0,
            is_head: head,
        }
    }

    fn remote(name: &str) -> RefKind {
        RefKind::Remote {
            remote: name.to_string(),
        }
    }

    fn page(count: usize, from: usize) -> Vec<LogCommit> {
        (from..from + count)
            .map(|i| {
                commit(
                    &format!("{i:040}"),
                    &[&format!("{:040}", i + 1)],
                    Vec::new(),
                )
            })
            .collect()
    }

    #[test]
    fn badges_map_and_order_by_kind() {
        assert_eq!(badge_of(RefLabelKind::Head), RefBadge::Head);
        assert_eq!(badge_of(RefLabelKind::Remote), RefBadge::Remote);
        let refs = vec![
            label(RefLabelKind::Tag, "v1"),
            label(RefLabelKind::Remote, "origin/main"),
            label(RefLabelKind::Local, "main"),
            label(RefLabelKind::Head, "feature"),
        ];
        let kinds: Vec<RefBadge> = ordered_badges(&refs).iter().map(|(b, _)| *b).collect();
        assert_eq!(
            kinds,
            vec![
                RefBadge::Head,
                RefBadge::Local,
                RefBadge::Remote,
                RefBadge::Tag
            ]
        );
    }

    #[test]
    fn a_row_keeps_three_badges_and_counts_the_rest() {
        let refs = (0..5)
            .map(|i| label(RefLabelKind::Local, &format!("b{i}")))
            .chain([label(RefLabelKind::Head, "h")])
            .collect();
        let mut tab = LogTab::new(LogFilter::default());
        let generation = tab.restart(LogFilter::default(), None);
        tab.apply_page(generation, Ok(vec![commit("a", &[], refs)]));
        let row = &tab.rows[0];
        assert!(row.head);
        assert_eq!(row.badges.len(), MAX_BADGES);
        assert_eq!(row.badges[0], (RefBadge::Head, "h".to_string()));
        assert_eq!(row.hidden, 3);
    }

    #[test]
    fn filter_inputs_are_trimmed_and_mapped_to_the_repository() {
        let filter = LogFilter::from_inputs(None, "  ana ", " ./auth/ ", " fix ");
        assert_eq!(filter.author.as_deref(), Some("ana"));
        assert_eq!(filter.path.as_deref(), Some("auth"));
        assert_eq!(filter.text, "fix");
        assert!(filter.narrows());
        let query = filter.query("api/tests", 400);
        assert_eq!(query.path.as_deref(), Some("api/tests/auth"));
        assert_eq!(query.text.as_deref(), Some("fix"));
        assert_eq!((query.skip, query.limit), (400, PAGE_SIZE));
        let at_root = filter.query("", 0);
        assert_eq!(at_root.path.as_deref(), Some("auth"));
        let empty = LogFilter::from_inputs(Some("main".into()), " ", "", "");
        assert!(!empty.narrows());
        assert_eq!(empty.query("x", 0).path, None);
        assert_eq!(empty.query("x", 0).branch.as_deref(), Some("main"));
    }

    #[test]
    fn a_full_page_means_more_and_the_next_skips_what_is_loaded() {
        let mut tab = LogTab::new(LogFilter::default());
        assert_eq!(tab.begin_more(), None);
        let generation = tab.restart(LogFilter::default(), None);
        assert_eq!(tab.begin_more(), None, "still reading the first page");
        let outcome = tab.apply_page(generation, Ok(page(PAGE_SIZE, 0)));
        assert!(outcome.applied);
        assert_eq!(outcome.select, Some(format!("{:040}", 0)));
        assert!(tab.has_more);
        assert_eq!(tab.begin_more(), Some((generation, PAGE_SIZE)));
        assert_eq!(tab.begin_more(), None, "one read at a time");
        tab.apply_page(generation, Ok(page(10, PAGE_SIZE)));
        assert_eq!(tab.rows.len(), PAGE_SIZE + 10);
        assert!(!tab.has_more);
        assert_eq!(tab.begin_more(), None);
        // The selection survives later pages.
        assert_eq!(tab.selected, Some(format!("{:040}", 0)));
    }

    #[test]
    fn the_keys_move_the_selection_within_the_loaded_rows() {
        let mut tab = LogTab::new(LogFilter::default());
        assert_eq!(tab.row_from_selected(1), None, "an empty list");
        let generation = tab.restart(LogFilter::default(), None);
        tab.apply_page(generation, Ok(page(5, 0)));
        tab.selected = None;
        assert_eq!(
            tab.row_from_selected(1),
            Some(0),
            "nothing selected starts at the top"
        );
        tab.select(&format!("{:040}", 2));
        assert_eq!(tab.row_from_selected(1), Some(3));
        assert_eq!(tab.row_from_selected(-1), Some(1));
        assert_eq!(
            tab.row_from_selected(10),
            Some(4),
            "clamped to the last row"
        );
        assert_eq!(
            tab.row_from_selected(-10),
            Some(0),
            "clamped to the first row"
        );
        assert_eq!(tab.row_from_selected(isize::MAX), Some(4), "End");
        assert_eq!(tab.row_from_selected(isize::MIN), Some(0), "Home");
        tab.select("not loaded");
        assert_eq!(tab.row_from_selected(1), Some(0));
    }

    #[test]
    fn a_page_of_an_older_generation_is_dropped() {
        let mut tab = LogTab::new(LogFilter::default());
        let old = tab.restart(LogFilter::default(), None);
        let new = tab.restart(LogFilter::branch(Some("dev".into())), None);
        assert_ne!(old, new);
        assert_eq!(tab.apply_page(old, Ok(page(3, 0))), PageOutcome::default());
        assert!(tab.rows.is_empty());
        assert!(tab.loading);
        assert!(tab.apply_page(new, Ok(page(3, 0))).applied);
        assert_eq!(tab.rows.len(), 3);
        assert_eq!(tab.filter.branch.as_deref(), Some("dev"));
    }

    #[test]
    fn the_graph_continues_across_pages() {
        let mut tab = LogTab::new(LogFilter::default());
        let generation = tab.restart(LogFilter::default(), None);
        // Page one ends with a commit whose parent is the first commit of page two.
        let mut first = page(PAGE_SIZE, 0);
        tab.apply_page(generation, Ok(std::mem::take(&mut first)));
        tab.apply_page(generation, Ok(page(2, PAGE_SIZE)));
        let Some(last) = tab.rows.last() else {
            panic!("rows");
        };
        assert_eq!(last.graph.lane, 0);
        assert_eq!(tab.lanes, 1);
    }

    #[test]
    fn a_failed_first_page_shows_the_error_and_a_failed_later_one_keeps_the_list() {
        let mut tab = LogTab::new(LogFilter::default());
        let generation = tab.restart(LogFilter::default(), None);
        tab.apply_page(generation, Err("boom".into()));
        assert_eq!(tab.error.as_deref(), Some("boom"));
        assert!(!tab.loading);

        let generation = tab.restart(LogFilter::default(), None);
        assert_eq!(tab.error, None);
        tab.apply_page(generation, Ok(page(PAGE_SIZE, 0)));
        tab.begin_more();
        tab.apply_page(generation, Err("later".into()));
        assert_eq!(tab.error, None);
        assert_eq!(tab.rows.len(), PAGE_SIZE);
        assert!(!tab.has_more);
    }

    #[test]
    fn selection_moves_to_the_first_commit_when_it_is_filtered_out() {
        let mut tab = LogTab::new(LogFilter::default());
        let generation = tab.restart(LogFilter::default(), None);
        tab.apply_page(generation, Ok(page(3, 0)));
        let second = format!("{:040}", 1);
        assert!(tab.select(&second));
        assert!(!tab.select(&second));
        let generation = tab.restart(LogFilter::branch(Some("dev".into())), None);
        // The selected commit is in the new list: kept.
        assert_eq!(tab.apply_page(generation, Ok(page(2, 1))).select, None);
        let generation = tab.restart(LogFilter::default(), None);
        let outcome = tab.apply_page(generation, Ok(page(2, 10)));
        assert_eq!(outcome.select, Some(format!("{:040}", 10)));
        assert_eq!(tab.detail, Detail::Loading);
        // Nothing found: nothing selected.
        let generation = tab.restart(LogFilter::default(), None);
        tab.apply_page(generation, Ok(Vec::new()));
        assert_eq!(tab.selected, None);
        assert_eq!(tab.detail, Detail::None);
    }

    #[test]
    fn a_detail_for_another_commit_is_ignored() {
        let mut tab = LogTab::new(LogFilter::default());
        tab.select("a");
        tab.select("b");
        assert!(!tab.apply_detail("a", Err("x".into())));
        assert_eq!(tab.detail, Detail::Loading);
        assert!(tab.apply_detail("b", Err("x".into())));
        assert_eq!(tab.detail, Detail::Failed("x".into()));
    }

    #[test]
    fn graph_geometry_follows_the_design() {
        assert_eq!(lane_x(0), 18.0);
        assert_eq!(lane_x(2), 46.0);
        assert_eq!(graph_width(1), 64.0);
        assert_eq!(graph_width(3), 64.0);
        assert_eq!(graph_width(5), 18.0 + 4.0 * 14.0 + 18.0);
        assert_eq!(graph_width(100), graph_width(MAX_LANES));
        let mut tab = LogTab::new(LogFilter::from_inputs(None, "ana", "", ""));
        assert_eq!(tab.graph_width(), 0.0);
        tab.filter = LogFilter::default();
        assert_eq!(tab.graph_width(), 64.0);
    }

    #[test]
    fn the_branch_column_groups_sorts_and_filters() {
        let mut main = branch("main", RefKind::Local, false);
        main.ahead = 1;
        let list = BranchList {
            branches: vec![
                main,
                branch("zeta", RefKind::Local, false),
                branch("feature/x", RefKind::Local, true),
                branch("alpha", RefKind::Local, false),
                branch("origin/main", remote("origin"), false),
                branch("up/dev", remote("up"), false),
                branch("origin/dev", remote("origin"), false),
                branch("v0.4.0", RefKind::Tag, false),
                branch("v0.10.0", RefKind::Tag, false),
            ],
            recent: Vec::new(),
        };
        let favorites = vec!["zeta".to_string()];
        let rows = log_branch_rows(&list, &favorites, "", Some("main"));
        let names: Vec<String> = rows
            .iter()
            .map(|row| match row {
                LogBranchRow::All { selected } => format!("all:{selected}"),
                LogBranchRow::Header(LogSection::Local) => "#local".into(),
                LogBranchRow::Header(LogSection::Remote(r)) => format!("#remote {r}"),
                LogBranchRow::Header(LogSection::Tags) => "#tags".into(),
                LogBranchRow::Ref(r) => {
                    format!("{}{}", r.name, if r.selected { "*" } else { "" })
                }
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "all:false",
                "#local",
                "feature/x",
                "zeta",
                "alpha",
                "main*",
                "#remote origin",
                "origin/dev",
                "origin/main",
                "#remote up",
                "up/dev",
                "#tags",
                "v0.10.0",
                "v0.4.0",
            ]
        );
        let LogBranchRow::Ref(current) = &rows[2] else {
            panic!("a ref");
        };
        assert!(current.current);
        let LogBranchRow::Ref(starred) = &rows[3] else {
            panic!("a ref");
        };
        assert!(starred.favorite);
        let LogBranchRow::Ref(main) = &rows[5] else {
            panic!("a ref");
        };
        assert_eq!(main.sync, "\u{2191}1");

        let narrowed = log_branch_rows(&list, &favorites, "DEV", None);
        assert_eq!(narrowed.len(), 1 + 2 + 2);
        assert_eq!(narrowed[0], LogBranchRow::All { selected: true });
        assert!(
            !narrowed
                .iter()
                .any(|row| matches!(row, LogBranchRow::Header(LogSection::Local)))
        );
    }

    #[test]
    fn the_branch_column_is_rebuilt_only_when_its_sources_change() {
        let list = Arc::new(BranchList {
            branches: vec![branch("main", RefKind::Local, true)],
            recent: Vec::new(),
        });
        let mut column = BranchColumn::default();
        assert_eq!(column.rows(None, &[], "", None).len(), 1);
        let first = column.rows(Some(&list), &[], "", None);
        assert_eq!(first.len(), 3);
        assert!(Arc::ptr_eq(
            &first,
            &column.rows(Some(&list), &[], "", None)
        ));
        let selected = column.rows(Some(&list), &[], "", Some("main"));
        assert!(!Arc::ptr_eq(&first, &selected));
        assert_eq!(selected[0], LogBranchRow::All { selected: false });
        let narrowed = column.rows(Some(&list), &[], "zzz", Some("main"));
        assert_eq!(narrowed.len(), 1);
    }

    #[test]
    fn the_detail_splits_the_message_and_places_the_files() {
        let detail = CommitDetail {
            oid: "a41c9e2".repeat(6) + "abcd",
            parents: vec!["9b3f0d1".repeat(6) + "abcd"],
            author_name: "Ana".into(),
            author_email: "ana@example.com".into(),
            date: 5,
            message: "Add refresh\n\nStores the token.\nSecond line.".into(),
            files: vec![
                CommitFile {
                    path: "tests/auth/refresh.brasa".into(),
                    orig_path: None,
                    letter: 'A',
                    additions: 14,
                    deletions: 0,
                    binary: false,
                },
                CommitFile {
                    path: "README.md".into(),
                    orig_path: None,
                    letter: 'M',
                    additions: 1,
                    deletions: 1,
                    binary: false,
                },
            ],
        };
        let methods = HashMap::from([("auth/refresh.brasa".to_string(), Method::Post)]);
        let view = DetailView::new(detail, "tests", &methods);
        assert_eq!(view.subject, "Add refresh");
        assert_eq!(view.body, "Stores the token.\nSecond line.");
        assert_eq!(view.short, "a41c9e2");
        assert_eq!(view.parents[0].short, "9b3f0d1");
        let request = &view.files[0];
        assert_eq!(request.name, "refresh");
        assert_eq!(request.folder, "auth");
        assert_eq!(request.path, "auth/refresh.brasa");
        assert_eq!(request.repo_path, "tests/auth/refresh.brasa");
        assert_eq!(request.method, Some(Method::Post));
        let outside = &view.files[1];
        assert_eq!(outside.path, "README.md");
        assert_eq!(outside.method, None);
        assert_eq!(outside.name, "README.md");
    }

    #[test]
    fn a_one_line_message_has_no_body() {
        let detail = CommitDetail {
            oid: "abc".into(),
            parents: Vec::new(),
            author_name: "Ana".into(),
            author_email: String::new(),
            date: 0,
            message: "Only subject".into(),
            files: Vec::new(),
        };
        let view = DetailView::new(detail, "", &HashMap::new());
        assert_eq!(view.subject, "Only subject");
        assert_eq!(view.body, "");
    }
}
