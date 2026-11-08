//! Finds the git branch (or short commit hash when detached) of a workspace, without a git
//! dependency: it only reads `.git/HEAD`, and for a worktree the `.git` file that points at the
//! real git directory, the same files the `git` binary itself would read for this.

use std::fs;
use std::path::{Path, PathBuf};

/// The number of hex characters shown for a detached `HEAD`, matching `git`'s own default
/// abbreviation length.
const SHORT_HASH_LENGTH: usize = 7;

/// Returns the current git branch of the repository containing `root`, or the short commit hash
/// when `HEAD` is detached, or `None` if `root` is not inside a git repository.
///
/// Walks up from `root` looking for a `.git` entry. A `.git` directory is a normal repository; a
/// `.git` file (used by `git worktree`) contains a `gitdir: <path>` line pointing at the actual
/// git directory to read `HEAD` from.
pub fn git_branch(root: &Path) -> Option<String> {
    let git_dir = find_git_dir(root)?;
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    parse_head(head.trim())
}

/// Walks up from `path` (inclusive) looking for a `.git` file or directory, resolving it to the
/// actual git directory to read `HEAD` from.
fn find_git_dir(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(dir) = current {
        let candidate = dir.join(".git");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if candidate.is_file() {
            return resolve_git_dir_file(&candidate, dir);
        }
        current = dir.parent();
    }
    None
}

/// Resolves a `.git` file (git worktree) to the git directory it points at, relative to `base`
/// (the folder the `.git` file is in) if the path it contains is relative.
fn resolve_git_dir_file(git_file: &Path, base: &Path) -> Option<PathBuf> {
    let content = fs::read_to_string(git_file).ok()?;
    let target = content.trim().strip_prefix("gitdir:")?.trim();
    let target_path = PathBuf::from(target);
    if target_path.is_absolute() {
        Some(target_path)
    } else {
        Some(base.join(target_path))
    }
}

/// Parses the content of a `HEAD` file into a branch name or a short commit hash.
fn parse_head(head: &str) -> Option<String> {
    if let Some(reference) = head.strip_prefix("ref:") {
        let reference = reference.trim();
        return Some(
            reference
                .strip_prefix("refs/heads/")
                .unwrap_or(reference)
                .to_string(),
        );
    }
    if head.len() >= SHORT_HASH_LENGTH
        && head.chars().all(|character| character.is_ascii_hexdigit())
    {
        return Some(head[..SHORT_HASH_LENGTH].to_string());
    }
    None
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Writes `contents` to `path`, creating its parent directories first.
    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories");
        }
        fs::write(path, contents).expect("write fixture file");
    }

    #[test]
    fn reads_the_branch_name_from_a_normal_git_directory() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(&temp.path().join(".git/HEAD"), "ref: refs/heads/main\n");

        assert_eq!(git_branch(temp.path()), Some("main".to_string()));
    }

    #[test]
    fn reads_a_short_commit_hash_for_a_detached_head() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(
            &temp.path().join(".git/HEAD"),
            "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2\n",
        );

        assert_eq!(git_branch(temp.path()), Some("a1b2c3d".to_string()));
    }

    #[test]
    fn walks_up_from_a_subdirectory_to_find_the_git_directory() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(&temp.path().join(".git/HEAD"), "ref: refs/heads/develop\n");
        let subdir = temp.path().join("crates/app");
        fs::create_dir_all(&subdir).expect("create subdir");

        assert_eq!(git_branch(&subdir), Some("develop".to_string()));
    }

    #[test]
    fn follows_a_relative_gitdir_file_used_by_worktrees() {
        let temp = tempfile::tempdir().expect("temp dir");
        write_file(
            &temp.path().join("main-repo/.git/worktrees/feature/HEAD"),
            "ref: refs/heads/feature-branch\n",
        );
        write_file(
            &temp.path().join("worktree/.git"),
            "gitdir: ../main-repo/.git/worktrees/feature\n",
        );

        assert_eq!(
            git_branch(&temp.path().join("worktree")),
            Some("feature-branch".to_string())
        );
    }

    #[test]
    fn follows_an_absolute_gitdir_file_used_by_worktrees() {
        let temp = tempfile::tempdir().expect("temp dir");
        let real_git_dir = temp.path().join("main-repo/.git/worktrees/feature");
        write_file(&real_git_dir.join("HEAD"), "ref: refs/heads/feature-two\n");
        write_file(
            &temp.path().join("worktree/.git"),
            &format!("gitdir: {}\n", real_git_dir.display()),
        );

        assert_eq!(
            git_branch(&temp.path().join("worktree")),
            Some("feature-two".to_string())
        );
    }

    #[test]
    fn returns_none_outside_a_git_repository() {
        let temp = tempfile::tempdir().expect("temp dir");
        assert_eq!(git_branch(temp.path()), None);
    }
}
