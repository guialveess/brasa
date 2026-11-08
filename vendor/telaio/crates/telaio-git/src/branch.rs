//! Branches, tags, remote branches and checking them out.

use std::collections::HashSet;

use crate::diff::check_revision;
use crate::{GitError, Repo};

/// What kind of ref a [`BranchInfo`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefKind {
    /// A local branch (`refs/heads/*`).
    Local,
    /// A remote tracking branch (`refs/remotes/<remote>/*`).
    Remote {
        /// The remote name, for example `origin`.
        remote: String,
    },
    /// A tag (`refs/tags/*`).
    Tag,
}

/// A branch or a tag with the data a branch menu shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    /// The short name: `main`, `origin/main` or `v0.4.1`.
    pub name: String,
    /// Local branch, remote branch or tag.
    pub kind: RefKind,
    /// The short id of the commit it points at (an annotated tag is peeled to its commit).
    pub oid: String,
    /// The first line of that commit's message.
    pub subject: String,
    /// The committer date of that commit, in unix seconds. 0 when git printed none.
    pub date: i64,
    /// The upstream of a local branch (`origin/main`), `None` when it has none.
    pub upstream: Option<String>,
    /// Commits the branch has that its upstream lacks, 0 without an upstream.
    pub ahead: u32,
    /// Commits the upstream has that the branch lacks, 0 without an upstream.
    pub behind: u32,
    /// Whether this is the checked out branch.
    pub is_head: bool,
}

/// Separates the fields of the `for-each-ref` format (ASCII unit separator, never in a subject).
const SEP: char = '\u{1f}';

impl Repo {
    /// Every local branch, remote branch and tag, in git's ref order (local, remote, tags, each
    /// sorted by name). The `<remote>/HEAD` symrefs are left out.
    pub fn branches(&self) -> Result<Vec<BranchInfo>, GitError> {
        let format = [
            "%(refname)",
            "%(objectname:short)",
            "%(*objectname:short)",
            "%(subject)",
            "%(*subject)",
            "%(committerdate:unix)",
            "%(*committerdate:unix)",
            "%(upstream:short)",
            "%(upstream:track,nobracket)",
            "%(HEAD)",
        ]
        .join("%1f");
        let args = [
            "for-each-ref",
            &format!("--format={format}"),
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ];
        let output = self.run(&args)?;
        let mut branches = Vec::new();
        for line in String::from_utf8_lossy(&output).lines() {
            if let Some(info) = parse_ref_line(line)? {
                branches.push(info);
            }
        }
        Ok(branches)
    }

    /// The local branches most recently checked out, newest first, at most `limit`. The current
    /// branch comes first. It is read from the reflog of `HEAD`, and only names that still exist
    /// as local branches are kept. Empty (not an error) when there is no reflog yet.
    pub fn recent_branches(&self, limit: usize) -> Result<Vec<String>, GitError> {
        let existing: HashSet<String> = self
            .local_branch_names()?
            .into_iter()
            .collect::<HashSet<_>>();
        let mut recent: Vec<String> = Vec::new();
        let mut push = |name: &str| {
            if existing.contains(name) && !recent.iter().any(|r| r == name) {
                recent.push(name.to_string());
            }
        };
        if let Some(current) = self.current_branch()? {
            push(&current);
        }
        let args = ["reflog", "show", "--format=%gs", "-n", "500", "HEAD"];
        let output = match self.run(&args) {
            Ok(output) => output,
            // A repository without commits has no reflog for HEAD, git fails.
            Err(GitError::Failed { .. }) => Vec::new(),
            Err(err) => return Err(err),
        };
        for line in String::from_utf8_lossy(&output).lines() {
            // "checkout: moving from A to B": B is newer than A.
            let Some(moving) = line.strip_prefix("checkout: moving from ") else {
                continue;
            };
            if let Some((from, to)) = moving.rsplit_once(" to ") {
                push(to);
                push(from);
            }
        }
        recent.truncate(limit);
        Ok(recent)
    }

    /// Whether `name` is acceptable as a new branch name (`git check-ref-format --branch`).
    pub fn check_branch_name(&self, name: &str) -> Result<bool, GitError> {
        if name.is_empty() || name.starts_with('-') {
            return Ok(false);
        }
        match self.run(&["check-ref-format", "--branch", name]) {
            Ok(_) => Ok(true),
            Err(GitError::Failed { .. }) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Checks out `branch`. A local branch name switches to it. A remote branch name such as
    /// `origin/feature/x` switches to the local branch `feature/x` when it exists, and otherwise
    /// creates it tracking the remote one. Git refuses with
    /// [`FailureKind::LocalChanges`](crate::FailureKind::LocalChanges) when uncommitted changes
    /// would be lost.
    pub fn checkout(&self, branch: &str) -> Result<(), GitError> {
        check_revision(branch)?;
        if self.ref_exists(&format!("refs/heads/{branch}"))? {
            return self.run(&["switch", branch]).map(|_| ());
        }
        if self.ref_exists(&format!("refs/remotes/{branch}"))? {
            if let Some((_, local)) = branch.split_once('/')
                && self.ref_exists(&format!("refs/heads/{local}"))?
            {
                return self.run(&["switch", local]).map(|_| ());
            }
            return self.run(&["switch", "--track", branch]).map(|_| ());
        }
        // Not a known ref: let git decide (it may guess a remote branch, or fail clearly).
        self.run(&["switch", branch]).map(|_| ())
    }

    /// Checks out any revision (a commit, a tag, a remote branch) with a detached `HEAD`.
    pub fn checkout_detached(&self, rev: &str) -> Result<(), GitError> {
        check_revision(rev)?;
        self.run(&["switch", "--detach", rev]).map(|_| ())
    }

    /// Creates the branch `name` at `start` (default `HEAD`), and switches to it when `checkout`
    /// is set. Starting from a remote branch makes it track that branch.
    pub fn create_branch(
        &self,
        name: &str,
        start: Option<&str>,
        checkout: bool,
    ) -> Result<(), GitError> {
        check_revision(name)?;
        let mut args = if checkout {
            vec!["switch", "-c", name]
        } else {
            vec!["branch", name]
        };
        if let Some(start) = start {
            check_revision(start)?;
            args.push(start);
        }
        self.run(&args).map(|_| ())
    }

    /// Renames the local branch `old` to `new`.
    pub fn rename_branch(&self, old: &str, new: &str) -> Result<(), GitError> {
        check_revision(old)?;
        check_revision(new)?;
        self.run(&["branch", "-m", old, new]).map(|_| ())
    }

    /// Deletes the local branch `name`. Without `force` git refuses a branch with unmerged
    /// commits, as [`FailureKind::NotMerged`](crate::FailureKind::NotMerged).
    pub fn delete_branch(&self, name: &str, force: bool) -> Result<(), GitError> {
        check_revision(name)?;
        let flag = if force { "-D" } else { "-d" };
        self.run(&["branch", flag, name]).map(|_| ())
    }

    fn local_branch_names(&self) -> Result<Vec<String>, GitError> {
        let output = self.run(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])?;
        Ok(String::from_utf8_lossy(&output)
            .lines()
            .map(str::to_string)
            .collect())
    }

    fn ref_exists(&self, full_name: &str) -> Result<bool, GitError> {
        match self.run(&["show-ref", "--verify", "--quiet", full_name]) {
            Ok(_) => Ok(true),
            Err(GitError::Failed { .. }) => Ok(false),
            Err(err) => Err(err),
        }
    }
}

/// Parses one line of the `for-each-ref` format of [`Repo::branches`]. `None` for the symrefs
/// `refs/remotes/<remote>/HEAD`.
fn parse_ref_line(line: &str) -> Result<Option<BranchInfo>, GitError> {
    let fields: Vec<&str> = line.split(SEP).collect();
    let [
        refname,
        oid,
        peeled_oid,
        subject,
        peeled_subject,
        date,
        peeled_date,
        upstream,
        track,
        head,
    ] = fields[..]
    else {
        return Err(GitError::Parse(format!("bad ref line {line:?}")));
    };
    // For an annotated tag the plain fields describe the tag object and the `*` ones the commit.
    let (name, kind) = if let Some(name) = refname.strip_prefix("refs/heads/") {
        (name.to_string(), RefKind::Local)
    } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
        let Some((remote, rest)) = name.split_once('/') else {
            return Ok(None);
        };
        if rest == "HEAD" {
            return Ok(None);
        }
        (
            name.to_string(),
            RefKind::Remote {
                remote: remote.to_string(),
            },
        )
    } else if let Some(name) = refname.strip_prefix("refs/tags/") {
        (name.to_string(), RefKind::Tag)
    } else {
        return Ok(None);
    };
    let (ahead, behind) = parse_track(track);
    Ok(Some(BranchInfo {
        name,
        kind,
        oid: pick(oid, peeled_oid).to_string(),
        subject: pick(subject, peeled_subject).to_string(),
        date: pick(date, peeled_date).parse().unwrap_or(0),
        upstream: (!upstream.is_empty()).then(|| upstream.to_string()),
        ahead,
        behind,
        is_head: head == "*",
    }))
}

/// The peeled value when there is one, else the plain one.
fn pick<'a>(plain: &'a str, peeled: &'a str) -> &'a str {
    if peeled.is_empty() { plain } else { peeled }
}

/// Parses `%(upstream:track,nobracket)`: `ahead 2, behind 1`, `ahead 2`, `behind 1`, `gone` or
/// empty. A gone upstream counts as 0 and 0.
fn parse_track(track: &str) -> (u32, u32) {
    let (mut ahead, mut behind) = (0, 0);
    for part in track.split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tracking_states() {
        assert_eq!(parse_track("ahead 2, behind 1"), (2, 1));
        assert_eq!(parse_track("ahead 3"), (3, 0));
        assert_eq!(parse_track("behind 4"), (0, 4));
        assert_eq!(parse_track("gone"), (0, 0));
        assert_eq!(parse_track(""), (0, 0));
    }

    fn line(fields: [&str; 10]) -> String {
        fields.join("\u{1f}")
    }

    #[test]
    fn parses_ref_lines() {
        let local = line([
            "refs/heads/feature/x",
            "abc1234",
            "",
            "Add x",
            "",
            "1700000000",
            "",
            "origin/feature/x",
            "ahead 1",
            "*",
        ]);
        let Some(info) = parse_ref_line(&local).ok().flatten() else {
            panic!("a local branch parses");
        };
        assert_eq!(info.name, "feature/x");
        assert_eq!(info.kind, RefKind::Local);
        assert_eq!(info.upstream.as_deref(), Some("origin/feature/x"));
        assert_eq!((info.ahead, info.behind, info.is_head), (1, 0, true));
        assert_eq!(info.date, 1_700_000_000);

        let remote = line([
            "refs/remotes/origin/main",
            "abc1234",
            "",
            "s",
            "",
            "5",
            "",
            "",
            "",
            " ",
        ]);
        let info = parse_ref_line(&remote).ok().flatten();
        assert_eq!(
            info.map(|i| i.kind),
            Some(RefKind::Remote {
                remote: "origin".into()
            })
        );

        let head = line([
            "refs/remotes/origin/HEAD",
            "a",
            "",
            "s",
            "",
            "5",
            "",
            "",
            "",
            " ",
        ]);
        assert_eq!(parse_ref_line(&head).ok().flatten(), None);
    }

    #[test]
    fn annotated_tags_use_the_peeled_commit() {
        let tag = line([
            "refs/tags/v1",
            "tagobj1",
            "commit1",
            "Release notes",
            "Commit subject",
            "",
            "1700000001",
            "",
            "",
            " ",
        ]);
        let info = parse_ref_line(&tag).ok().flatten();
        let Some(info) = info else {
            panic!("a tag parses");
        };
        assert_eq!(info.kind, RefKind::Tag);
        assert_eq!(info.oid, "commit1");
        assert_eq!(info.subject, "Commit subject");
        assert_eq!(info.date, 1_700_000_001);
    }

    #[test]
    fn rejects_malformed_lines() {
        assert!(parse_ref_line("refs/heads/x").is_err());
    }
}
