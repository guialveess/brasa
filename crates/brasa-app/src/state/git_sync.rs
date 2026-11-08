//! Plain data behind commit, push, pull and fetch (#171, #172): the operation in flight for the
//! status bar, the commit box's summary line and checks, the staged secrets of design 6d, when
//! to fetch on focus, and the last failure with git's raw output. `views/git/sync.rs` runs the
//! commands and stores the results here.

use std::time::{Duration, Instant};

use rust_i18n::t;
use telaio_git::{FailureKind, NumStat, Status};

/// The least time between two automatic fetches, so coming back to the window often costs at
/// most one fetch every five minutes.
pub const AUTO_FETCH_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// A git command that talks to the history or the remote, shown in the status bar while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOp {
    /// `git commit`, also with `--amend`.
    Commit,
    /// `git push`.
    Push,
    /// `git pull`.
    Pull,
    /// `git fetch`.
    Fetch,
    /// Switching to another branch or commit, with the stash around it.
    Checkout,
    /// Creating, renaming or deleting a branch.
    Branch,
}

impl GitOp {
    /// The status bar's progress text, "Pushing…".
    pub fn progress_label(self) -> String {
        match self {
            GitOp::Commit => t!("git.sync.committing"),
            GitOp::Push => t!("git.sync.pushing"),
            GitOp::Pull => t!("git.sync.pulling"),
            GitOp::Fetch => t!("git.sync.fetching"),
            GitOp::Checkout => t!("git.sync.checking_out"),
            GitOp::Branch => t!("git.sync.branching"),
        }
        .into_owned()
    }
}

/// A failed [`GitOp`], kept so "Show git output" can show what git printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFailure {
    /// What was running.
    pub op: GitOp,
    /// Why it failed, as far as git's messages tell.
    pub kind: FailureKind,
    /// Git's own output, verbatim.
    pub output: String,
}

impl GitFailure {
    /// The notification's title, "Push rejected".
    pub fn title(&self) -> String {
        match (self.op, self.kind) {
            (GitOp::Push, FailureKind::Rejected) => t!("git.failure.push_rejected"),
            (GitOp::Commit, _) => t!("git.failure.commit"),
            (GitOp::Push, _) => t!("git.failure.push"),
            (GitOp::Pull, FailureKind::Conflicts) => t!("git.failure.pull_conflicts"),
            (GitOp::Pull, _) => t!("git.failure.pull"),
            (GitOp::Fetch, _) => t!("git.failure.fetch"),
            (GitOp::Checkout, _) => t!("git.failure.checkout"),
            (GitOp::Branch, _) => t!("git.failure.branch"),
        }
        .into_owned()
    }

    /// The plain language explanation under the title. `upstream` names the remote branch in
    /// the rejected push message.
    pub fn message(&self, upstream: Option<&str>) -> String {
        match self.kind {
            FailureKind::Rejected => match upstream {
                Some(upstream) => t!("git.failure.rejected_text", upstream = upstream),
                None => t!("git.failure.rejected_text_plain"),
            },
            FailureKind::Auth => t!("git.failure.auth_text"),
            FailureKind::NoAuthor => t!("git.failure.no_author_text"),
            FailureKind::NothingToCommit => t!("git.failure.nothing_text"),
            FailureKind::Conflicts => t!("git.failure.conflicts_text"),
            FailureKind::Network => t!("git.failure.network_text"),
            FailureKind::Hook => t!("git.failure.hook_text"),
            FailureKind::NotMerged => t!("git.failure.not_merged_text"),
            FailureKind::LocalChanges => t!("git.failure.local_changes_text"),
            FailureKind::Other => t!("git.failure.other_text"),
        }
        .into_owned()
    }
}

/// The commit box's summary line: "4 files · +31 −9".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommitStats {
    /// Files staged for the next commit.
    pub files: usize,
    /// Lines added by them.
    pub additions: u64,
    /// Lines removed by them.
    pub deletions: u64,
}

impl CommitStats {
    /// The totals of `numstat` (what `git diff --cached --numstat` printed).
    pub fn from_numstat(numstat: &[NumStat]) -> CommitStats {
        CommitStats {
            files: numstat.len(),
            additions: numstat.iter().map(|n| u64::from(n.additions)).sum(),
            deletions: numstat.iter().map(|n| u64::from(n.deletions)).sum(),
        }
    }
}

/// The commit box: whether Amend is on and what the last commit check found.
#[derive(Debug, Clone, Default)]
pub struct CommitBox {
    /// The Amend checkbox.
    pub amend: bool,
    /// What the next commit would include, read after every status change.
    pub stats: CommitStats,
}

/// The sync state of the window: the command in flight, the last fetch and the last failure.
#[derive(Debug, Clone, Default)]
pub struct SyncState {
    /// The command running, shown in the status bar.
    pub running: Option<GitOp>,
    /// When the last automatic or manual fetch started.
    pub last_fetch: Option<Instant>,
    /// The last failure, for "Show git output".
    pub failure: Option<GitFailure>,
}

/// Whether a file holds secrets that should not be committed: Brasa's local environment files
/// (`*.local.env`). Pure, it only looks at the name.
pub fn is_secret_file(path: &str) -> bool {
    path.ends_with(".local.env")
}

/// The staged files that hold secrets (design 6d), repository relative. The whole index counts,
/// files outside the workspace included, since all of it goes into the commit.
pub fn staged_secrets(status: &Status) -> Vec<String> {
    status
        .entries
        .iter()
        .filter(|entry| {
            !entry.untracked
                && entry.index != telaio_git::Change::Unmodified
                && entry.index != telaio_git::Change::Deleted
                && is_secret_file(&entry.path)
        })
        .map(|entry| entry.path.clone())
        .collect()
}

/// Whether the commit message can be written: something is staged, or Amend is ticked (which may
/// only reword). With nothing to commit the message box stays disabled.
pub fn can_write_message(stats: CommitStats, amend: bool) -> bool {
    amend || stats.files > 0
}

/// Whether the commit buttons can run: a message (unless amending, which keeps the old one), and
/// something staged (unless amending, which may only reword), with nothing else running.
pub fn can_commit(message: &str, stats: CommitStats, amend: bool, running: bool) -> bool {
    !running && !message.trim().is_empty() && can_write_message(stats, amend)
}

/// Whether coming back to the window should fetch: the setting is on, the branch tracks a
/// remote, nothing else is running, and the last fetch is old enough.
pub fn auto_fetch_due(
    enabled: bool,
    has_upstream: bool,
    running: bool,
    last: Option<Instant>,
    now: Instant,
) -> bool {
    enabled
        && has_upstream
        && !running
        && last.is_none_or(|last| now.saturating_duration_since(last) >= AUTO_FETCH_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use telaio_git::{Change, Entry, Head};

    fn entry(path: &str, index: Change, untracked: bool) -> Entry {
        Entry {
            path: path.to_string(),
            orig_path: None,
            index,
            worktree: Change::Unmodified,
            conflicted: false,
            untracked,
        }
    }

    fn status(entries: Vec<Entry>) -> Status {
        Status {
            head: Head::Branch("main".to_string()),
            head_oid: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            entries,
        }
    }

    #[test]
    fn recognizes_secret_files() {
        assert!(is_secret_file("environments/dev.local.env"));
        assert!(is_secret_file("prod.local.env"));
        assert!(!is_secret_file("environments/dev.env"));
        assert!(!is_secret_file("local.env.bak"));
    }

    #[test]
    fn staged_secrets_finds_only_staged_local_env_files() {
        let status = status(vec![
            entry("environments/staging.local.env", Change::Added, false),
            entry("environments/prod.local.env", Change::Unmodified, false),
            entry("environments/dev.local.env", Change::Unmodified, true),
            entry("environments/old.local.env", Change::Deleted, false),
            entry("environments/staging.env", Change::Modified, false),
        ]);
        assert_eq!(
            staged_secrets(&status),
            vec!["environments/staging.local.env".to_string()]
        );
    }

    #[test]
    fn stats_add_up_the_numstat() {
        let numstat = [
            NumStat {
                path: "a".into(),
                additions: 3,
                deletions: 1,
                binary: false,
            },
            NumStat {
                path: "b".into(),
                additions: 0,
                deletions: 0,
                binary: true,
            },
        ];
        assert_eq!(
            CommitStats::from_numstat(&numstat),
            CommitStats {
                files: 2,
                additions: 3,
                deletions: 1
            }
        );
    }

    #[test]
    fn commit_needs_a_message_and_staged_files_unless_amending() {
        let staged = CommitStats {
            files: 1,
            ..CommitStats::default()
        };
        let empty = CommitStats::default();
        assert!(can_commit("Fix", staged, false, false));
        assert!(!can_commit("  \n", staged, false, false));
        assert!(!can_commit("Fix", empty, false, false));
        assert!(can_commit("Reword", empty, true, false));
        assert!(!can_commit("Fix", staged, false, true));
    }

    #[test]
    fn the_message_needs_something_staged_unless_amending() {
        let staged = CommitStats {
            files: 2,
            ..CommitStats::default()
        };
        assert!(can_write_message(staged, false));
        assert!(!can_write_message(CommitStats::default(), false));
        assert!(can_write_message(CommitStats::default(), true));
    }

    #[test]
    fn auto_fetch_waits_five_minutes() {
        let now = Instant::now();
        assert!(auto_fetch_due(true, true, false, None, now));
        assert!(!auto_fetch_due(false, true, false, None, now));
        assert!(!auto_fetch_due(true, false, false, None, now));
        assert!(!auto_fetch_due(true, true, true, None, now));
        let recent = now - Duration::from_secs(60);
        assert!(!auto_fetch_due(true, true, false, Some(recent), now));
        let old = now - AUTO_FETCH_INTERVAL;
        assert!(auto_fetch_due(true, true, false, Some(old), now));
    }
}
