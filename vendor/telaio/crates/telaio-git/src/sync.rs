//! Talking to a remote: push, pull, fetch and the list of commits a push would send.

use crate::{GitError, Repo};

/// The most commits [`Repo::unpushed_commits`] returns.
const MAX_COMMITS: usize = 100;

/// Where [`Repo::push`] sends the current branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushTarget {
    /// The remote name, for example `origin`.
    pub remote: String,
    /// The branch name on the remote.
    pub branch: String,
    /// Whether the push must also set the upstream, because the branch has none yet.
    pub set_upstream: bool,
}

/// How [`Repo::pull`] joins the remote commits with the local ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullMode {
    /// Honor `pull.rebase`; when it is unset, merge (git refuses divergent branches otherwise).
    FollowConfig,
    /// Always merge.
    Merge,
    /// Always rebase.
    Rebase,
}

/// A commit in a list: its short id and its first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSummary {
    /// The short commit id.
    pub oid: String,
    /// The first line of the message.
    pub subject: String,
}

impl Repo {
    /// Where a push of the current branch would go, `None` when `HEAD` is detached or has no
    /// commit, or when no remote can be chosen. A branch with an upstream pushes to it; one
    /// without pushes to `remote.pushDefault`, else `origin`, else the only remote, and sets
    /// the upstream.
    pub fn push_target(&self) -> Result<Option<PushTarget>, GitError> {
        if !self.has_head()? {
            return Ok(None);
        }
        let Some(branch) = self.current_branch()? else {
            return Ok(None);
        };
        let remotes = self.remotes()?;
        if remotes.is_empty() {
            return Ok(None);
        }
        let remote = self.config(&format!("branch.{branch}.remote"))?;
        let merge = self.config(&format!("branch.{branch}.merge"))?;
        if let (Some(remote), Some(merge)) = (remote, merge)
            && remotes.contains(&remote)
        {
            let name = merge.strip_prefix("refs/heads/").unwrap_or(&merge);
            return Ok(Some(PushTarget {
                remote,
                branch: name.to_string(),
                set_upstream: false,
            }));
        }
        let push_default = self.config("remote.pushDefault")?;
        let remote = push_default
            .filter(|r| remotes.contains(r))
            .or_else(|| remotes.iter().find(|r| *r == "origin").cloned())
            .or_else(|| (remotes.len() == 1).then(|| remotes[0].clone()));
        Ok(remote.map(|remote| PushTarget {
            remote,
            branch,
            set_upstream: true,
        }))
    }

    /// Pushes the current branch to `target`. The first push of a branch also sets its upstream.
    pub fn push(&self, target: &PushTarget) -> Result<(), GitError> {
        let refspec = format!("HEAD:refs/heads/{}", target.branch);
        let args: Vec<&str> = if target.set_upstream {
            vec!["push", "--porcelain", "-u", &target.remote, &refspec]
        } else {
            vec!["push", "--porcelain", &target.remote, &refspec]
        };
        self.run(&args).map(|_| ())
    }

    /// Brings in the remote commits of the current branch. A merge never opens an editor.
    pub fn pull(&self, mode: PullMode) -> Result<(), GitError> {
        let merge = match mode {
            PullMode::Merge => true,
            PullMode::Rebase => false,
            PullMode::FollowConfig => self.config("pull.rebase")?.is_none(),
        };
        let args: &[&str] = match (mode, merge) {
            (PullMode::FollowConfig, false) => &["pull"],
            (_, true) => &["pull", "--no-rebase", "--no-edit"],
            (_, false) => &["pull", "--rebase"],
        };
        self.run(args).map(|_| ())
    }

    /// Downloads the remote state without changing the work tree, dropping remote branches that
    /// no longer exist.
    pub fn fetch(&self) -> Result<(), GitError> {
        self.run(&["fetch", "--quiet", "--prune"]).map(|_| ())
    }

    /// The commits a push to `target` would send, newest first, at most 100. Without an upstream
    /// those are the commits of `HEAD` that no branch of the remote has.
    pub fn unpushed_commits(&self, target: &PushTarget) -> Result<Vec<CommitSummary>, GitError> {
        if !self.has_head()? {
            return Ok(Vec::new());
        }
        let remotes = format!("--remotes={}", target.remote);
        let limit = format!("--max-count={MAX_COMMITS}");
        let mut args = vec!["log", "-z", "--format=%h%x1f%s", &limit];
        if target.set_upstream {
            args.extend(["HEAD", "--not", &remotes]);
        } else {
            args.push("@{u}..HEAD");
        }
        let output = self.run(&args)?;
        let text = String::from_utf8_lossy(&output);
        Ok(text
            .split('\0')
            .filter_map(|record| record.trim_start_matches('\n').split_once('\x1f'))
            .map(|(oid, subject)| CommitSummary {
                oid: oid.to_string(),
                subject: subject.to_string(),
            })
            .collect())
    }

    /// The checked out branch, `None` when `HEAD` is detached.
    pub(crate) fn current_branch(&self) -> Result<Option<String>, GitError> {
        match self.run(&["symbolic-ref", "--quiet", "--short", "HEAD"]) {
            Ok(output) => Ok(Some(String::from_utf8_lossy(&output).trim().to_string())),
            Err(GitError::Failed { code: Some(1), .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn remotes(&self) -> Result<Vec<String>, GitError> {
        let output = self.run(&["remote"])?;
        Ok(String::from_utf8_lossy(&output)
            .lines()
            .map(str::to_string)
            .collect())
    }
}
