//! Plain data behind the sidebar's Load tests panel (GitHub #64): the collapsible target tree
//! and the "Runs this session" entries derived from the open load test tabs.

use std::collections::HashSet;
use std::time::Duration;

use rust_i18n::t;

use brasa_core::Method;
use brasa_workspace::Node;

use super::load_test::{LoadTestStatus, LoadTestTab};
use super::number::format_integer;
use super::tabs::OpenTab;

/// What a target row is.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetRowKind {
    /// A folder, load tested as a collection. `expanded` drives the chevron.
    Folder {
        /// Whether its children are listed.
        expanded: bool,
    },
    /// A request. The method is `None` only if it could not be read.
    Request {
        /// The HTTP method shown in its color.
        method: Option<Method>,
    },
}

/// One visible row of the target tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetRow {
    /// The workspace id of the request or folder.
    pub id: String,
    /// The display name.
    pub label: String,
    /// Nesting depth, `0` at the root.
    pub depth: usize,
    /// Folder or request.
    pub kind: TargetRowKind,
}

/// The target tree: every folder and non-broken request of the workspace, with the folders the
/// user expanded. Like the collections sidebar, a folder starts collapsed. The visible rows are computed when the tree is rebuilt or a folder toggled,
/// so render only walks a ready list.
#[derive(Debug, Default)]
pub struct TargetTree {
    all: Vec<TargetRow>,
    expanded: HashSet<String>,
    visible: Vec<TargetRow>,
}

impl TargetTree {
    /// Rebuilds the rows from the workspace tree, keeping the expanded folders.
    pub fn rebuild(&mut self, nodes: &[Node]) {
        self.all.clear();
        flatten(nodes, 0, &mut self.all);
        self.recompute();
    }

    /// Expands the folder `id`, or collapses it if it was expanded.
    pub fn toggle(&mut self, id: &str) {
        if !self.expanded.remove(id) {
            self.expanded.insert(id.to_string());
        }
        self.recompute();
    }

    /// The rows to draw, in tree order.
    pub fn visible(&self) -> &[TargetRow] {
        &self.visible
    }

    fn recompute(&mut self) {
        self.visible.clear();
        // Depth of the collapsed folder whose descendants are being skipped.
        let mut skipping: Option<usize> = None;
        for row in &self.all {
            if let Some(depth) = skipping {
                if row.depth > depth {
                    continue;
                }
                skipping = None;
            }
            let mut row = row.clone();
            if let TargetRowKind::Folder { expanded } = &mut row.kind {
                *expanded = self.expanded.contains(&row.id);
                if !*expanded {
                    skipping = Some(row.depth);
                }
            }
            self.visible.push(row);
        }
    }
}

fn flatten(nodes: &[Node], depth: usize, into: &mut Vec<TargetRow>) {
    for node in nodes {
        match node {
            Node::Folder(folder) => {
                into.push(TargetRow {
                    id: folder.id.clone(),
                    label: folder.name.clone(),
                    depth,
                    kind: TargetRowKind::Folder { expanded: false },
                });
                flatten(&folder.children, depth + 1, into);
            }
            Node::Request(request) => {
                if request.broken.is_some() {
                    continue;
                }
                into.push(TargetRow {
                    id: request.id.clone(),
                    label: request.name.clone(),
                    depth,
                    kind: TargetRowKind::Request {
                        method: request.method.clone(),
                    },
                });
            }
        }
    }
}

/// The color of a run entry's status dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunDot {
    /// Running now (accent).
    Running,
    /// Done without errors (success).
    Done,
    /// Failed to start, or ended with errors (danger).
    Failed,
    /// Not started, or stopped by the user without errors (muted).
    Idle,
}

/// What an entry's meta line says. Kept as data so the text follows the active language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMeta {
    /// A run in progress.
    Running {
        /// Time since the run started.
        elapsed: Duration,
        /// The configured duration, in seconds.
        total_secs: u64,
    },
    /// A run that completed. The p95 latency rounded to whole milliseconds, if a snapshot exists.
    Done(Option<u64>),
    /// A run that was stopped early, with its p95 in milliseconds.
    Stopped(Option<u64>),
    /// A run that could not start.
    Failed,
    /// No run yet.
    NotStarted,
}

/// One entry of "Runs this session": an open load test tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEntry {
    /// The id of the tab, to activate it on click.
    pub tab_id: String,
    /// The tab label.
    pub label: String,
    /// The status dot color.
    pub dot: RunDot,
    /// The meta line contents.
    pub meta: RunMeta,
    /// Whether this is the active document tab.
    pub active: bool,
}

/// Derives the dot and meta line of one load test tab.
pub fn run_status(load_test: &LoadTestTab) -> (RunDot, RunMeta) {
    let p95_ms = load_test
        .snapshot
        .as_ref()
        .map(|snapshot| (u64::from(snapshot.p95) + 500) / 1000);
    let has_errors = load_test
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.error_rate > 0.0);
    match &load_test.status {
        LoadTestStatus::NotStarted => (RunDot::Idle, RunMeta::NotStarted),
        LoadTestStatus::Running => (
            RunDot::Running,
            RunMeta::Running {
                elapsed: load_test
                    .snapshot
                    .as_ref()
                    .map_or(Duration::ZERO, |snapshot| snapshot.elapsed),
                total_secs: load_test.config.resolved().duration_secs,
            },
        ),
        LoadTestStatus::Finished => (
            if has_errors {
                RunDot::Failed
            } else {
                RunDot::Done
            },
            RunMeta::Done(p95_ms),
        ),
        LoadTestStatus::Stopped => (
            if has_errors {
                RunDot::Failed
            } else {
                RunDot::Idle
            },
            RunMeta::Stopped(p95_ms),
        ),
        LoadTestStatus::Failed(_) => (RunDot::Failed, RunMeta::Failed),
    }
}

/// One entry per load test tab of `tabs`, in tab order. `label` names a tab.
pub fn run_entries(
    tabs: &[OpenTab],
    active_index: Option<usize>,
    label: impl Fn(&LoadTestTab) -> String,
) -> Vec<RunEntry> {
    tabs.iter()
        .enumerate()
        .filter_map(|(index, tab)| {
            let load_test = tab.load_test()?;
            let (dot, meta) = run_status(load_test);
            Some(RunEntry {
                tab_id: tab.id.clone(),
                label: label(load_test),
                dot,
                meta,
                active: active_index == Some(index),
            })
        })
        .collect()
}

/// Formats a duration as `m:ss`, for example `1:42`.
fn format_clock(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// The translated meta line text of `meta`.
pub fn meta_text(meta: &RunMeta) -> String {
    match meta {
        RunMeta::Running {
            elapsed,
            total_secs,
        } => t!(
            "navigation.run.running",
            elapsed = format_clock(*elapsed),
            total = format_clock(Duration::from_secs(*total_secs))
        )
        .into_owned(),
        RunMeta::Done(Some(p95)) => {
            t!("navigation.run.done_p95", p95 = format_integer(*p95)).into_owned()
        }
        RunMeta::Done(None) => t!("navigation.run.done").into_owned(),
        RunMeta::Stopped(Some(p95)) => {
            t!("navigation.run.stopped_p95", p95 = format_integer(*p95)).into_owned()
        }
        RunMeta::Stopped(None) => t!("navigation.run.stopped").into_owned(),
        RunMeta::Failed => t!("navigation.run.failed").into_owned(),
        RunMeta::NotStarted => t!("navigation.run.not_started").into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::load_test::LoadTestFailure;
    use brasa_load::LoadSnapshot;
    use brasa_workspace::{Folder, RequestEntry};
    use pretty_assertions::assert_eq;

    fn request(id: &str, name: &str, broken: bool) -> Node {
        Node::Request(RequestEntry {
            id: id.to_string(),
            name: name.to_string(),
            broken: broken.then(|| "broken".to_string()),
            method: (!broken).then_some(Method::Get),
        })
    }

    fn folder(id: &str, children: Vec<Node>) -> Node {
        Node::Folder(Folder {
            id: id.to_string(),
            name: id.to_string(),
            children,
        })
    }

    fn sample() -> Vec<Node> {
        vec![
            folder(
                "a",
                vec![
                    folder("a/b", vec![request("a/b/x.brasa", "x", false)]),
                    request("a/y.brasa", "y", false),
                    request("a/bad.brasa", "bad", true),
                ],
            ),
            request("z.brasa", "z", false),
        ]
    }

    fn ids(tree: &TargetTree) -> Vec<&str> {
        tree.visible().iter().map(|row| row.id.as_str()).collect()
    }

    #[test]
    fn folders_start_collapsed() {
        let mut tree = TargetTree::default();
        tree.rebuild(&sample());
        assert_eq!(ids(&tree), vec!["a", "z.brasa"]);
        assert_eq!(
            tree.visible()[0].kind,
            TargetRowKind::Folder { expanded: false }
        );
    }

    #[test]
    fn tree_lists_folders_and_requests_without_broken_ones() {
        let mut tree = TargetTree::default();
        tree.rebuild(&sample());
        tree.toggle("a");
        tree.toggle("a/b");
        assert_eq!(
            ids(&tree),
            vec!["a", "a/b", "a/b/x.brasa", "a/y.brasa", "z.brasa"]
        );
        assert_eq!(tree.visible()[2].depth, 2);
    }

    #[test]
    fn collapsing_a_folder_hides_every_descendant_and_survives_a_rebuild() {
        let mut tree = TargetTree::default();
        tree.rebuild(&sample());
        tree.toggle("a");
        tree.toggle("a/b");
        tree.toggle("a");
        assert_eq!(ids(&tree), vec!["a", "z.brasa"]);
        assert_eq!(
            tree.visible()[0].kind,
            TargetRowKind::Folder { expanded: false }
        );

        tree.rebuild(&sample());
        assert_eq!(ids(&tree), vec!["a", "z.brasa"]);

        tree.toggle("a");
        tree.toggle("a/b");
        assert_eq!(ids(&tree), vec!["a", "a/b", "a/y.brasa", "z.brasa"]);
    }

    fn snapshot(p95_us: u32, error_rate: f64, elapsed_secs: u64) -> LoadSnapshot {
        LoadSnapshot {
            elapsed: Duration::from_secs(elapsed_secs),
            active_vus: 0,
            total: 0,
            rps: 0.0,
            p50: 0,
            p95: p95_us,
            p99: 0,
            error_rate,
            series: Vec::new(),
            histogram: [0; 26],
            status_counts: Vec::new(),
            per_target: Vec::new(),
            error_examples: Vec::new(),
        }
    }

    fn tab(status: LoadTestStatus, snapshot: Option<LoadSnapshot>) -> LoadTestTab {
        let mut tab = LoadTestTab::unset();
        tab.status = status;
        tab.snapshot = snapshot;
        tab
    }

    #[test]
    fn not_started_is_idle() {
        let (dot, meta) = run_status(&tab(LoadTestStatus::NotStarted, None));
        assert_eq!((dot, meta), (RunDot::Idle, RunMeta::NotStarted));
    }

    #[test]
    fn running_carries_elapsed_and_the_configured_duration() {
        let mut running = tab(LoadTestStatus::Running, Some(snapshot(0, 0.0, 102)));
        running.config.duration_secs = "180".to_string();
        let (dot, meta) = run_status(&running);
        assert_eq!(dot, RunDot::Running);
        assert_eq!(
            meta,
            RunMeta::Running {
                elapsed: Duration::from_secs(102),
                total_secs: 180
            }
        );
        assert_eq!(meta_text(&meta), "running \u{b7} 1:42 / 3:00");
    }

    #[test]
    fn finished_is_done_with_p95_in_milliseconds() {
        let (dot, meta) = run_status(&tab(
            LoadTestStatus::Finished,
            Some(snapshot(182_400, 0.0, 60)),
        ));
        assert_eq!(
            (dot, meta.clone()),
            (RunDot::Done, RunMeta::Done(Some(182)))
        );
        assert_eq!(meta_text(&meta), "done \u{b7} p95 182 ms");
    }

    #[test]
    fn errors_turn_the_dot_danger() {
        let (dot, _) = run_status(&tab(
            LoadTestStatus::Finished,
            Some(snapshot(1000, 0.1, 60)),
        ));
        assert_eq!(dot, RunDot::Failed);
        let (dot, _) = run_status(&tab(LoadTestStatus::Stopped, Some(snapshot(1000, 0.1, 20))));
        assert_eq!(dot, RunDot::Failed);
        let (dot, meta) = run_status(&tab(LoadTestStatus::Stopped, Some(snapshot(1000, 0.0, 20))));
        assert_eq!((dot, meta), (RunDot::Idle, RunMeta::Stopped(Some(1))));
    }

    #[test]
    fn a_setup_failure_is_failed() {
        let (dot, meta) = run_status(&tab(
            LoadTestStatus::Failed(LoadTestFailure::NoTarget),
            None,
        ));
        assert_eq!((dot, meta), (RunDot::Failed, RunMeta::Failed));
        assert_eq!(meta_text(&RunMeta::NotStarted), "not started");
    }
}
