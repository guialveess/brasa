//! The stash, used by "Stash and check out": the uncommitted changes are stashed tagged with the
//! branch they came from and restored when coming back to it.

use crate::{GitError, Repo};

/// The message of the automatic stash of `branch`: the app's `prefix` (such as `postino-auto:`,
/// it tells the stashes the app created itself from the user's) followed by the branch name.
pub fn auto_stash_message(prefix: &str, branch: &str) -> String {
    format!("{prefix}{branch}")
}

/// One entry of the stash list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    /// The position in the list, 0 is the newest (`stash@{0}`).
    pub index: usize,
    /// The message given when it was stashed, without git's `On <branch>:` prefix.
    pub message: String,
}

impl Repo {
    /// Stashes the staged, unstaged and untracked changes with `message`. Returns `false` when
    /// there was nothing to stash.
    pub fn stash_push(&self, message: &str) -> Result<bool, GitError> {
        // `--no-literal-pathspecs` undoes the global option of `run`: with it, git stashes the
        // untracked files but forgets to remove them from the work tree.
        let args = [
            "--no-literal-pathspecs",
            "stash",
            "push",
            "--include-untracked",
            "-m",
            message,
        ];
        let output = self.run(&args)?;
        // Git exits with 0 and says so on the standard output.
        Ok(!String::from_utf8_lossy(&output).contains("No local changes to save"))
    }

    /// The stash entries, newest first.
    pub fn stash_list(&self) -> Result<Vec<StashEntry>, GitError> {
        let output = self.run(&["stash", "list", "--format=%gs"])?;
        Ok(String::from_utf8_lossy(&output)
            .lines()
            .enumerate()
            .map(|(index, line)| StashEntry {
                index,
                message: strip_branch_prefix(line).to_string(),
            })
            .collect())
    }

    /// Applies the stash entry `index` and drops it. When it conflicts git keeps the entry and
    /// the error is [`FailureKind::Conflicts`](crate::FailureKind::Conflicts).
    pub fn stash_pop(&self, index: usize) -> Result<(), GitError> {
        let entry = format!("stash@{{{index}}}");
        self.run(&["stash", "pop", &entry]).map(|_| ())
    }

    /// The index of the newest automatic stash made for `branch` with the app's `prefix`, if
    /// any.
    pub fn find_auto_stash(&self, prefix: &str, branch: &str) -> Result<Option<usize>, GitError> {
        let wanted = auto_stash_message(prefix, branch);
        Ok(self
            .stash_list()?
            .into_iter()
            .find(|entry| entry.message == wanted)
            .map(|entry| entry.index))
    }
}

/// Removes the `On <branch>: ` or `WIP on <branch>: ` that git puts before a stash message.
/// Branch names cannot contain `: `, so the first one ends the prefix.
fn strip_branch_prefix(subject: &str) -> &str {
    if (subject.starts_with("On ") || subject.starts_with("WIP on "))
        && let Some((_, message)) = subject.split_once(": ")
    {
        return message;
    }
    subject
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_branch_prefix() {
        assert_eq!(
            strip_branch_prefix("On main: app-auto:main"),
            "app-auto:main"
        );
        assert_eq!(
            strip_branch_prefix("On feature/x: my note: more"),
            "my note: more"
        );
        assert_eq!(
            strip_branch_prefix("WIP on main: abc123 subject"),
            "abc123 subject"
        );
        assert_eq!(strip_branch_prefix("something else"), "something else");
    }

    #[test]
    fn auto_message_is_prefix_and_branch() {
        assert_eq!(
            auto_stash_message("app-auto:", "feature/x"),
            "app-auto:feature/x"
        );
    }
}
