//! The [`Git`] handle and the [`Repo`] operations.

use std::fs;
use std::path::{Path, PathBuf};

use crate::run::{Runner, run, run_with};
use crate::status::{Change, Entry, Status, parse_status};
use crate::{GitError, Version};

/// A usable git executable, found by [`crate::detect`].
#[derive(Debug, Clone)]
pub struct Git {
    runner: Runner,
    path: PathBuf,
    version: Version,
}

/// The identity git would use for a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    /// `user.name`.
    pub name: String,
    /// `user.email`.
    pub email: String,
}

/// A git work tree, and the folder inside it that the app opened.
#[derive(Debug, Clone)]
pub struct Repo {
    git: Git,
    root: PathBuf,
    prefix: String,
}

impl Git {
    pub(crate) fn new(runner: Runner, path: PathBuf, version: Version) -> Git {
        Git {
            runner,
            path,
            version,
        }
    }

    /// The executable in use. For the git of WSL, the path the settings gave
    /// (`\\wsl.localhost\Ubuntu\usr\bin\git`), or just `git` when it was found
    /// automatically; see [`Git::wsl_distro`].
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The WSL distribution whose git runs every command (through `wsl.exe`), `None` for a git
    /// of this machine.
    pub fn wsl_distro(&self) -> Option<&str> {
        match &self.runner {
            Runner::Native(_) => None,
            Runner::Wsl { distro, .. } => Some(distro),
        }
    }

    /// Its version.
    pub fn version(&self) -> Version {
        self.version
    }

    /// Finds the work tree that contains `dir`, or `None` when `dir` is not inside one (which
    /// includes the inside of a `.git` folder and bare repositories).
    pub fn discover(&self, dir: &Path) -> Result<Option<Repo>, GitError> {
        let args = ["rev-parse", "--show-toplevel", "--show-prefix"];
        let output = match run(&self.runner, Some(dir), &args) {
            Ok(output) => output,
            Err(GitError::Failed { stderr, .. })
                if stderr.contains("not a git repository")
                    || stderr.contains("must be run in a work tree") =>
            {
                return Ok(None);
            }
            Err(err) => return Err(err),
        };
        let text = String::from_utf8_lossy(&output);
        let mut lines = text.lines();
        let root = lines.next().filter(|l| !l.is_empty());
        let root = root.ok_or_else(|| GitError::Parse("rev-parse printed no root".to_string()))?;
        let prefix = lines.next().unwrap_or("").trim_end_matches('/').to_string();
        let root = match &self.runner {
            Runner::Native(_) => dunce::canonicalize(root)?,
            // A Linux path: back to the Windows path the app reads files through, on the same
            // share as `dir`.
            Runner::Wsl { distro, .. } => {
                let host = crate::wsl_location(dir).map(|location| location.host);
                crate::wsl::to_windows(root, distro, host.as_deref().unwrap_or("wsl.localhost"))
            }
        };
        Ok(Some(Repo {
            git: self.clone(),
            root,
            prefix,
        }))
    }

    /// Trusts a repository owned by another user: adds `directory` (as
    /// [`GitError::unsafe_directory`] gives it) to `safe.directory` in the user's global git
    /// configuration, unless it is there already. Only after the user asked for it.
    pub fn trust_directory(&self, directory: &str) -> Result<(), GitError> {
        let listed = run_with(
            &self.runner,
            None,
            &["config", "--global", "--get-all", "safe.directory"],
            None,
            // 1: the key is not set at all.
            &[1],
        )?;
        if String::from_utf8_lossy(&listed)
            .lines()
            .any(|line| line.trim() == directory)
        {
            return Ok(());
        }
        run(
            &self.runner,
            None,
            &["config", "--global", "--add", "safe.directory", directory],
        )?;
        Ok(())
    }

    /// Runs `git init` in `dir`.
    pub fn init(&self, dir: &Path) -> Result<(), GitError> {
        run(&self.runner, Some(dir), &["init", "--quiet"]).map(|_| ())
    }

    /// Clones only the last commit of `url` into `dir`, a folder that does not exist yet or is
    /// empty: `git clone --depth 1 --single-branch`, of `branch` or, when `None`, of the remote's
    /// default branch. For reading files of a repository without its history (Postino's OpenAPI
    /// import). `url` is anything git clones: `https://`, `git@host:path`, a local path.
    pub fn clone_shallow(
        &self,
        url: &str,
        branch: Option<&str>,
        dir: &Path,
    ) -> Result<(), GitError> {
        // Run from the parent with a relative target, so the git of WSL gets no Windows path.
        let parent = dir
            .parent()
            .ok_or_else(|| GitError::Parse(format!("{} has no parent folder", dir.display())))?;
        let name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| GitError::Parse(format!("{} has no folder name", dir.display())))?;
        let mut args = vec!["clone", "--quiet", "--depth", "1", "--single-branch"];
        if let Some(branch) = branch {
            args.extend(["--branch", branch]);
        }
        args.extend(["--", url, name.as_str()]);
        run(&self.runner, Some(parent), &args).map(|_| ())
    }
}

impl Repo {
    /// The top level folder of the work tree.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The workspace folder relative to [`Repo::root`], `/` separated and without a trailing
    /// slash, `""` when the workspace is the root.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub(crate) fn run(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        run(&self.git.runner, Some(&self.root), args)
    }

    /// Runs git with `stdin` as its input and `ok_codes` accepted as success.
    pub(crate) fn run_with(
        &self,
        args: &[&str],
        stdin: Option<&[u8]>,
        ok_codes: &[i32],
    ) -> Result<Vec<u8>, GitError> {
        run_with(&self.git.runner, Some(&self.root), args, stdin, ok_codes)
    }

    /// Runs `git <command...> -- <paths>`.
    fn run_with_paths(&self, command: &[&str], paths: &[String]) -> Result<(), GitError> {
        let mut args: Vec<&str> = command.to_vec();
        args.push("--");
        args.extend(paths.iter().map(String::as_str));
        self.run(&args).map(|_| ())
    }

    /// Whether `HEAD` points at a commit (false on a repository without commits).
    pub(crate) fn has_head(&self) -> Result<bool, GitError> {
        match self.run(&["rev-parse", "--verify", "--quiet", "HEAD"]) {
            Ok(_) => Ok(true),
            Err(GitError::Failed { .. }) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// The branch, upstream and changed files.
    pub fn status(&self) -> Result<Status, GitError> {
        let args = [
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
        ];
        parse_status(&self.run(&args)?)
    }

    /// Stages every change (additions, edits and deletions) under `paths`, relative to the root.
    pub fn stage(&self, paths: &[String]) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        self.run_with_paths(&["add", "-A"], paths)
    }

    /// Removes `paths` from the index, keeping the work tree as it is. Also works before the
    /// first commit.
    pub fn unstage(&self, paths: &[String]) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        if self.has_head()? {
            self.run_with_paths(&["restore", "--staged"], paths)
        } else {
            self.run_with_paths(&["rm", "--cached", "-r", "--quiet"], paths)
        }
    }

    /// Throws away the changes under `paths`, relative to the root. Tracked files go back to
    /// `HEAD` (index and work tree), new files are deleted, whether staged or untracked.
    pub fn discard(&self, paths: &[String]) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        let status = self.status()?;
        let selected: Vec<&Entry> = status
            .entries
            .iter()
            .filter(|e| paths.iter().any(|p| covers(p, &e.path)))
            .collect();

        let mut restore = Vec::new();
        let mut remove = Vec::new();
        for entry in selected {
            if entry.untracked {
                delete_path(&self.root.join(&entry.path))?;
            } else if matches!(
                entry.index,
                Change::Added | Change::Copied | Change::Renamed
            ) {
                // Not in HEAD under this name: drop it from index and work tree.
                remove.push(entry.path.clone());
                if let (Change::Renamed, Some(orig)) = (entry.index, &entry.orig_path) {
                    restore.push(orig.clone());
                }
            } else {
                restore.push(entry.path.clone());
            }
        }
        if !remove.is_empty() {
            self.run_with_paths(&["rm", "-f", "--quiet"], &remove)?;
        }
        if !restore.is_empty() {
            self.run_with_paths(
                &["restore", "--source=HEAD", "--staged", "--worktree"],
                &restore,
            )?;
        }
        Ok(())
    }

    /// The `user.name` and `user.email` git would commit with, `None` when either is not set.
    pub fn author(&self) -> Result<Option<Author>, GitError> {
        let name = self.config("user.name")?;
        let email = self.config("user.email")?;
        Ok(name.zip(email).map(|(name, email)| Author { name, email }))
    }

    pub(crate) fn config(&self, key: &str) -> Result<Option<String>, GitError> {
        match self.run(&["config", "--get", key]) {
            Ok(output) => {
                let value = String::from_utf8_lossy(&output).trim().to_string();
                Ok((!value.is_empty()).then_some(value))
            }
            // Exit code 1 means the key is not set.
            Err(GitError::Failed { code: Some(1), .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

/// Whether `selector` (a file or a folder, repo relative) is `path` or contains it.
fn covers(selector: &str, path: &str) -> bool {
    let selector = selector.trim_end_matches('/');
    selector.is_empty()
        || path
            .strip_prefix(selector)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn delete_path(path: &Path) -> Result<(), GitError> {
    let result = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
        _ => Ok(()),
    }
}
