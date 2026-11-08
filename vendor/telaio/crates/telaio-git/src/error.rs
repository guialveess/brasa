//! The error types of this crate.

use std::io;
use std::path::PathBuf;

use crate::Version;

/// Why no usable `git` executable could be found. See [`crate::detect`].
#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    /// There is no `git` executable on the `PATH` (or at the configured path).
    #[error("git was not found")]
    NotFound,

    /// macOS only: `/usr/bin/git` is a shim that offers to install the Xcode Command Line Tools,
    /// and they are not installed. The shim is never run, since it pops up the installer.
    #[error("git needs the Xcode Command Line Tools, which are not installed")]
    XcodeToolsMissing,

    /// A `git` was found but it is older than [`crate::MIN_VERSION`].
    #[error("git {version} at {path:?} is too old (2.30.0 or newer is required)")]
    TooOld {
        /// The executable that was found.
        path: PathBuf,
        /// Its version.
        version: Version,
    },

    /// A `git` was found but `git --version` failed or printed something unexpected.
    #[error("could not run git: {0}")]
    Failed(String),
}

/// Everything that can go wrong while running a git command.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// The process could not be started, or a file operation around it failed.
    #[error("could not run git: {0}")]
    Spawn(#[from] io::Error),

    /// The command ran and exited with a failure status.
    #[error("{command} failed{}: {}", code.map(|c| format!(" with code {c}")).unwrap_or_default(), if stderr.is_empty() { stdout } else { stderr })]
    Failed {
        /// The command line, for example `git add -A -- a.txt`.
        command: String,
        /// The exit code, `None` when the process was killed by a signal.
        code: Option<i32>,
        /// What git printed on its standard error, trimmed.
        stderr: String,
        /// What git printed on its standard output, trimmed. Some failures are only explained
        /// there (`push --porcelain` rejections, `commit` with nothing to commit).
        stdout: String,
    },

    /// The output of git could not be understood.
    #[error("unexpected git output: {0}")]
    Parse(String),
}

/// Why a git command failed, in terms the UI can explain in plain language. See
/// [`GitError::kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The remote has commits the branch lacks (non fast forward, fetch first).
    Rejected,
    /// The remote refused the credentials, or there are none and git cannot ask.
    Auth,
    /// There is no `user.name` or `user.email` to commit with.
    NoAuthor,
    /// A commit was asked for but nothing is staged.
    NothingToCommit,
    /// A merge or rebase stopped with conflicts to resolve.
    Conflicts,
    /// The remote could not be reached.
    Network,
    /// A hook (pre-commit, commit-msg, pre-push...) refused the operation. Best effort.
    Hook,
    /// A branch cannot be deleted with `-d` because it has commits no other branch has.
    NotMerged,
    /// A checkout was refused because uncommitted changes (or untracked files) would be lost.
    LocalChanges,
    /// Anything else.
    Other,
}

impl GitError {
    /// Classifies a failed command by what git printed. Anything that is not a
    /// [`GitError::Failed`] is [`FailureKind::Other`].
    pub fn kind(&self) -> FailureKind {
        match self {
            GitError::Failed { stderr, stdout, .. } => classify(&format!("{stderr}\n{stdout}")),
            _ => FailureKind::Other,
        }
    }

    /// When git refused the repository because another user owns it ("detected dubious
    /// ownership"), the value git suggests adding to `safe.directory` to trust it, as it printed
    /// it (`'%(prefix)///wsl.localhost/Ubuntu/home/me/repo'` without the quotes). This happens
    /// for every repository of WSL opened from Windows, whose owner Windows cannot read. Pass it
    /// to [`crate::Git::trust_directory`] once the user agrees.
    pub fn unsafe_directory(&self) -> Option<String> {
        let GitError::Failed { stderr, .. } = self else {
            return None;
        };
        if !stderr.contains("dubious ownership") {
            return None;
        }
        let line = stderr
            .lines()
            .find(|line| line.contains("--add safe.directory"))?;
        let (_, value) = line.split_once("safe.directory")?;
        let value = value.trim();
        let value = value
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .unwrap_or(value);
        (!value.is_empty()).then(|| value.to_string())
    }

    /// The raw output of the failed command (standard error, then standard output), for a
    /// "show git output" dialog. Empty for errors that are not [`GitError::Failed`].
    pub fn output(&self) -> String {
        match self {
            GitError::Failed { stderr, stdout, .. } => [stderr.as_str(), stdout.as_str()]
                .iter()
                .filter(|part| !part.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("\n"),
            other => other.to_string(),
        }
    }
}

/// Whether `text` contains any of `needles`.
fn has(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| text.contains(n))
}

fn classify(output: &str) -> FailureKind {
    let text = output.to_lowercase();
    if has(
        &text,
        &[
            "would be overwritten by checkout",
            "please commit your changes or stash them",
        ],
    ) {
        FailureKind::LocalChanges
    } else if text.contains("not fully merged") {
        FailureKind::NotMerged
    } else if has(
        &text,
        &["conflict", "automatic merge failed", "could not apply"],
    ) {
        FailureKind::Conflicts
    } else if has(&text, &["hook declined", "hook exited", "hook failed"])
        || (text.contains("hook") && text.contains("failed"))
    {
        FailureKind::Hook
    } else if has(
        &text,
        &[
            "non-fast-forward",
            "fetch first",
            "[rejected]",
            "updates were rejected",
        ],
    ) {
        FailureKind::Rejected
    } else if has(
        &text,
        &[
            "authentication failed",
            "permission denied",
            "could not read username",
            "could not read password",
            "host key verification failed",
            "terminal prompts disabled",
            "returned error: 403",
            "returned error: 401",
            "access denied",
        ],
    ) {
        FailureKind::Auth
    } else if has(&text, &["please tell me who you are", "empty ident"]) {
        FailureKind::NoAuthor
    } else if has(
        &text,
        &[
            "nothing to commit",
            "nothing added to commit",
            "no changes added to commit",
        ],
    ) {
        FailureKind::NothingToCommit
    } else if has(
        &text,
        &[
            "could not resolve host",
            "connection refused",
            "timed out",
            "network is unreachable",
            "unable to access",
        ],
    ) {
        FailureKind::Network
    } else if text.contains("hook") {
        FailureKind::Hook
    } else {
        FailureKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(stderr: &str, stdout: &str) -> GitError {
        GitError::Failed {
            command: "git x".into(),
            code: Some(1),
            stderr: stderr.into(),
            stdout: stdout.into(),
        }
    }

    #[test]
    fn classifies_real_git_messages() {
        let cases = [
            (
                "To /tmp/origin\n!\trefs/heads/main:refs/heads/main\t[rejected] (fetch first)\nDone",
                FailureKind::Rejected,
            ),
            (
                "hint: Updates were rejected because the tip of your current branch is behind",
                FailureKind::Rejected,
            ),
            (
                "remote: Invalid username or password.\nfatal: Authentication failed for 'https://github.com/a/b.git/'",
                FailureKind::Auth,
            ),
            (
                "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
                FailureKind::Auth,
            ),
            (
                "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.",
                FailureKind::Auth,
            ),
            ("Host key verification failed.", FailureKind::Auth),
            (
                "Author identity unknown\n\n*** Please tell me who you are.",
                FailureKind::NoAuthor,
            ),
            (
                "fatal: empty ident name (for <a@b>) not allowed",
                FailureKind::NoAuthor,
            ),
            (
                "On branch main\nnothing to commit, working tree clean",
                FailureKind::NothingToCommit,
            ),
            (
                "no changes added to commit (use \"git add\")",
                FailureKind::NothingToCommit,
            ),
            (
                "CONFLICT (content): Merge conflict in a.txt\nAutomatic merge failed; fix conflicts",
                FailureKind::Conflicts,
            ),
            (
                "error: could not apply 1a2b3c4... change",
                FailureKind::Conflicts,
            ),
            (
                "fatal: unable to access 'https://x/y.git/': Could not resolve host: x",
                FailureKind::Network,
            ),
            (
                "ssh: connect to host x port 22: Connection refused",
                FailureKind::Network,
            ),
            (
                "error: cannot run .git/hooks/pre-commit: No such file",
                FailureKind::Hook,
            ),
            (
                "! [remote rejected] main -> main (pre-receive hook declined)",
                FailureKind::Hook,
            ),
            (
                "error: Your local changes to the following files would be overwritten by checkout:\n\ta.txt\nPlease commit your changes or stash them before you switch branches.\nAborting",
                FailureKind::LocalChanges,
            ),
            (
                "error: The following untracked working tree files would be overwritten by checkout:\n\tconflict.txt",
                FailureKind::LocalChanges,
            ),
            (
                "error: the branch 'topic' is not fully merged\nhint: If you are sure you want to delete it, run 'git branch -D topic'",
                FailureKind::NotMerged,
            ),
            ("fatal: not a git repository", FailureKind::Other),
        ];
        for (text, expected) in cases {
            assert_eq!(failed(text, "").kind(), expected, "{text}");
            assert_eq!(failed("", text).kind(), expected, "stdout: {text}");
        }
    }

    #[test]
    fn output_joins_stderr_and_stdout() {
        assert_eq!(failed("a", "b").output(), "a\nb");
        assert_eq!(failed("", "b").output(), "b");
        assert_eq!(GitError::Parse("x".into()).kind(), FailureKind::Other);
    }

    #[test]
    fn display_falls_back_to_stdout() {
        assert!(failed("", "only out").to_string().ends_with(": only out"));
    }

    #[test]
    fn dubious_ownership_gives_the_directory_to_trust() {
        let windows = "fatal: detected dubious ownership in repository at '//wsl.localhost/Ubuntu-24.04/home/me/repo'\n'//wsl.localhost/Ubuntu-24.04/home/me/repo' may refer to a non-local directory\nTo add an exception for this directory, call:\n\n\tgit config --global --add safe.directory '%(prefix)///wsl.localhost/Ubuntu-24.04/home/me/repo'";
        assert_eq!(
            failed(windows, "").unsafe_directory().as_deref(),
            Some("%(prefix)///wsl.localhost/Ubuntu-24.04/home/me/repo")
        );
        let linux = "fatal: detected dubious ownership in repository at '/srv/repo'\nTo add an exception for this directory, call:\n\n\tgit config --global --add safe.directory /srv/repo";
        assert_eq!(
            failed(linux, "").unsafe_directory().as_deref(),
            Some("/srv/repo")
        );
        assert_eq!(
            failed("fatal: not a git repository", "").unsafe_directory(),
            None
        );
    }
}
