//! Committing, and adding paths to the `.gitignore`.

use std::fs;

use crate::{GitError, Repo};

impl Repo {
    /// Commits what is staged (never `-a`) with `message`. With `amend` the last commit is
    /// replaced instead, which also works with nothing staged, to reword it. Works on a branch
    /// with no commits yet. The message is cleaned up like an editor message (comment lines and
    /// extra blank lines stripped) and git never opens an editor.
    pub fn commit(&self, message: &str, amend: bool) -> Result<(), GitError> {
        let mut args = vec!["commit", "--quiet", "--cleanup=strip", "-F", "-"];
        if amend {
            args.push("--amend");
        }
        self.run_with(&args, Some(message.as_bytes()), &[])
            .map(|_| ())
    }

    /// The full message of `HEAD`, `None` on a branch with no commits.
    pub fn last_commit_message(&self) -> Result<Option<String>, GitError> {
        if !self.has_head()? {
            return Ok(None);
        }
        let output = self.run(&["log", "-1", "--format=%B"])?;
        Ok(Some(
            String::from_utf8_lossy(&output).trim_end().to_string(),
        ))
    }

    /// Adds `path` (relative to the repository root) to the root `.gitignore` as an anchored
    /// pattern (`/path`), creating the file when needed. Does nothing when the same line is
    /// already there. The file is not unstaged, that is up to the caller.
    pub fn add_to_gitignore(&self, path: &str) -> Result<(), GitError> {
        let pattern = format!("/{}", path.trim_start_matches('/'));
        let file = self.root().join(".gitignore");
        let mut content = match fs::read_to_string(&file) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err.into()),
        };
        if content.lines().any(|line| line.trim_end() == pattern) {
            return Ok(());
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&pattern);
        content.push('\n');
        fs::write(file, content)?;
        Ok(())
    }
}
