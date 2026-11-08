//! Tests against a real `git` in temporary folders. They skip themselves when git is missing.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

use telaio_git::{
    ApplyTarget, Change, DiffSource, Executable, FileVersion, Git, Head, Repo, block_patch, detect,
    hunk_patch, parse_file_diff,
};
use tempfile::TempDir;

/// A git, or `None` (and a note) when this machine has none.
fn git() -> Option<Git> {
    match detect(&Executable::Auto) {
        Ok(git) => Some(git),
        Err(err) => {
            eprintln!("skipping, no usable git: {err}");
            None
        }
    }
}

/// Runs git for test setup, with an identity so commits never depend on the global config.
fn raw(git: &Git, dir: &Path, args: &[&str]) {
    let output = Command::new(git.path())
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A fresh repository on `main` with a local identity. Returns the folder and the repo.
fn new_repo(git: &Git) -> (TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    git.init(dir.path()).expect("init");
    raw(
        git,
        dir.path(),
        &["symbolic-ref", "HEAD", "refs/heads/main"],
    );
    raw(
        git,
        dir.path(),
        &["config", "--local", "user.name", "Test User"],
    );
    raw(
        git,
        dir.path(),
        &["config", "--local", "user.email", "test@example.com"],
    );
    raw(
        git,
        dir.path(),
        &["config", "--local", "commit.gpgsign", "false"],
    );
    let repo = git.discover(dir.path()).expect("discover").expect("a repo");
    (dir, repo)
}

fn write(dir: &TempDir, path: &str, content: &str) {
    let full = dir.path().join(path);
    fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
    fs::write(full, content).expect("write");
}

fn commit_all(git: &Git, dir: &TempDir) {
    raw(git, dir.path(), &["add", "-A"]);
    raw(git, dir.path(), &["commit", "-q", "-m", "c"]);
}

fn entry<'a>(status: &'a telaio_git::Status, path: &str) -> &'a telaio_git::Entry {
    status
        .entries
        .iter()
        .find(|e| e.path == path)
        .unwrap_or_else(|| panic!("no entry {path}"))
}

#[test]
fn detect_reports_a_modern_version() {
    let Some(git) = git() else { return };
    assert!(git.version() >= telaio_git::MIN_VERSION);
    assert!(git.path().is_file());
}

#[test]
fn discovers_at_the_root_and_in_a_subfolder() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    let canonical = dir.path().canonicalize().expect("canonical");
    assert_eq!(repo.root().canonicalize().expect("canonical"), canonical);
    assert_eq!(repo.prefix(), "");

    fs::create_dir_all(dir.path().join("a/b")).expect("mkdir");
    let sub = git
        .discover(&dir.path().join("a/b"))
        .expect("discover")
        .expect("a repo");
    assert_eq!(sub.root().canonicalize().expect("canonical"), canonical);
    assert_eq!(sub.prefix(), "a/b");
}

#[test]
fn a_plain_folder_is_not_a_repository() {
    let Some(git) = git() else { return };
    let dir = tempfile::tempdir().expect("tempdir");
    // Skip if the temp dir happens to live inside a repository.
    if git.discover(dir.path()).expect("discover").is_some() {
        return;
    }
    assert!(git.discover(dir.path()).expect("discover").is_none());
}

#[test]
fn status_follows_add_modify_delete_and_rename() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);

    let status = repo.status().expect("status");
    assert_eq!(status.head, Head::Unborn("main".into()));
    assert!(status.entries.is_empty());

    write(&dir, "keep.txt", "one\n");
    write(&dir, "gone.txt", "bye\n");
    write(
        &dir,
        "old name.txt",
        "some longer content to match the rename\nline two\n",
    );
    write(&dir, "dir/ñandú.txt", "x\n");
    let status = repo.status().expect("status");
    assert_eq!(status.entries.len(), 4);
    assert!(status.entries.iter().all(|e| e.untracked));
    assert!(status.entries.iter().any(|e| e.path == "dir/ñandú.txt"));

    commit_all(&git, &dir);
    let status = repo.status().expect("status");
    assert!(matches!(status.head, Head::Branch(ref b) if b == "main"));
    assert!(status.entries.is_empty());

    write(&dir, "keep.txt", "two\n");
    fs::remove_file(dir.path().join("gone.txt")).expect("rm");
    fs::rename(
        dir.path().join("old name.txt"),
        dir.path().join("new name.txt"),
    )
    .expect("mv");
    write(&dir, "fresh.txt", "new\n");
    raw(&git, dir.path(), &["add", "new name.txt", "old name.txt"]);

    let status = repo.status().expect("status");
    let keep = entry(&status, "keep.txt");
    assert_eq!(
        (keep.index, keep.worktree),
        (Change::Unmodified, Change::Modified)
    );
    assert_eq!(entry(&status, "gone.txt").worktree, Change::Deleted);
    let renamed = entry(&status, "new name.txt");
    assert_eq!(renamed.index, Change::Renamed);
    assert_eq!(renamed.orig_path.as_deref(), Some("old name.txt"));
    assert!(entry(&status, "fresh.txt").untracked);
}

#[test]
fn stage_and_unstage_after_a_commit() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "a.txt", "1\n");
    commit_all(&git, &dir);

    write(&dir, "a.txt", "2\n");
    write(&dir, "b.txt", "new\n");
    repo.stage(&["a.txt".into(), "b.txt".into()])
        .expect("stage");
    let status = repo.status().expect("status");
    assert_eq!(entry(&status, "a.txt").index, Change::Modified);
    assert_eq!(entry(&status, "b.txt").index, Change::Added);

    repo.unstage(&["a.txt".into(), "b.txt".into()])
        .expect("unstage");
    let status = repo.status().expect("status");
    assert_eq!(entry(&status, "a.txt").index, Change::Unmodified);
    assert_eq!(entry(&status, "a.txt").worktree, Change::Modified);
    assert!(entry(&status, "b.txt").untracked);
}

#[test]
fn stage_and_unstage_on_an_unborn_branch() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "sub/a.txt", "1\n");
    repo.stage(&["sub".into()]).expect("stage");
    assert_eq!(
        entry(&repo.status().expect("status"), "sub/a.txt").index,
        Change::Added
    );

    repo.unstage(&["sub".into()]).expect("unstage");
    assert!(entry(&repo.status().expect("status"), "sub/a.txt").untracked);
    assert!(dir.path().join("sub/a.txt").is_file());
}

#[test]
fn discard_restores_tracked_and_deletes_new_files() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "a.txt", "1\n");
    write(&dir, "b.txt", "1\n");
    commit_all(&git, &dir);

    write(&dir, "a.txt", "2\n");
    fs::remove_file(dir.path().join("b.txt")).expect("rm");
    write(&dir, "new/c.txt", "c\n");
    write(&dir, "staged.txt", "s\n");
    repo.stage(&["staged.txt".into()]).expect("stage");

    let all: Vec<String> = ["a.txt", "b.txt", "new", "staged.txt"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    repo.discard(&all).expect("discard");

    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).expect("read"),
        "1\n"
    );
    assert!(dir.path().join("b.txt").is_file());
    assert!(!dir.path().join("new/c.txt").exists());
    assert!(!dir.path().join("staged.txt").exists());
    assert!(repo.status().expect("status").entries.is_empty());
}

#[test]
fn discard_undoes_a_staged_rename() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(
        &dir,
        "old.txt",
        "some content that is long enough\nsecond line\n",
    );
    commit_all(&git, &dir);
    raw(&git, dir.path(), &["mv", "old.txt", "new.txt"]);

    repo.discard(&["new.txt".into()]).expect("discard");
    assert!(dir.path().join("old.txt").is_file());
    assert!(!dir.path().join("new.txt").exists());
    assert!(repo.status().expect("status").entries.is_empty());
}

#[test]
fn split_by_prefix_on_a_real_repository() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "ws/a.txt", "1\n");
    write(&dir, "other.txt", "1\n");
    commit_all(&git, &dir);
    write(&dir, "ws/a.txt", "2\n");
    write(&dir, "other.txt", "2\n");
    repo.stage(&["other.txt".into()]).expect("stage");

    let status = repo.status().expect("status");
    let (inside, staged_outside) = status.split_by_prefix("ws");
    assert_eq!(inside.len(), 1);
    assert_eq!(inside[0].path, "ws/a.txt");
    assert_eq!(staged_outside, 1);
}

#[test]
fn author_reads_the_local_config() {
    let Some(git) = git() else { return };
    let (_dir, repo) = new_repo(&git);
    let author = repo.author().expect("author").expect("some author");
    assert_eq!(author.name, "Test User");
    assert_eq!(author.email, "test@example.com");
}

const BASE: &str =
    "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\nl13\nl14\nl15\nl16\nl17\nl18\nl19\nl20\n";

/// `BASE` with lines 2 and 19 changed, two hunks far apart.
fn two_hunks() -> String {
    BASE.replace("l2\n", "L2\n").replace("l19\n", "L19\n")
}

#[test]
fn diff_of_a_modified_file_per_source() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "a b/f.txt", BASE);
    commit_all(&git, &dir);
    write(&dir, "a b/f.txt", &BASE.replace("l2\n", "L2\n"));
    raw(&git, dir.path(), &["add", "a b/f.txt"]);
    write(&dir, "a b/f.txt", &two_hunks());

    let path = "a b/f.txt";
    let staged = repo.diff(path, &DiffSource::HeadToIndex).expect("diff");
    assert!(
        staged.contains("+L2\n") && !staged.contains("+L19\n"),
        "{staged}"
    );
    let unstaged = repo.diff(path, &DiffSource::IndexToWorkTree).expect("diff");
    assert!(
        unstaged.contains("+L19\n") && !unstaged.contains("+L2\n"),
        "{unstaged}"
    );
    let all = repo.diff(path, &DiffSource::HeadToWorkTree).expect("diff");
    assert!(all.contains("+L2\n") && all.contains("+L19\n"));
    assert!(all.contains("--- a/a b/f.txt"), "{all}");
    assert_eq!(
        repo.diff("missing.txt", &DiffSource::HeadToWorkTree)
            .expect("diff"),
        ""
    );
}

#[test]
fn diff_of_staged_new_untracked_and_deleted_files() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "gone.txt", "bye\n");
    write(&dir, "keep.txt", "k\n");
    commit_all(&git, &dir);
    write(&dir, "added.txt", "new\n");
    raw(&git, dir.path(), &["add", "added.txt"]);
    write(&dir, "sub/untracked.txt", "u1\nu2\n");
    fs::remove_file(dir.path().join("gone.txt")).expect("rm");

    let added = repo
        .diff("added.txt", &DiffSource::HeadToIndex)
        .expect("diff");
    assert!(
        added.contains("new file mode") && added.contains("+new\n"),
        "{added}"
    );
    let added = repo
        .diff("added.txt", &DiffSource::HeadToWorkTree)
        .expect("diff");
    assert!(added.contains("new file mode"), "{added}");

    for source in [DiffSource::HeadToWorkTree, DiffSource::IndexToWorkTree] {
        let patch = repo.diff("sub/untracked.txt", &source).expect("diff");
        assert!(
            patch.contains("diff --git a/sub/untracked.txt b/sub/untracked.txt"),
            "{patch}"
        );
        assert!(patch.contains("new file mode"), "{patch}");
        assert!(
            patch.contains("--- /dev/null\n+++ b/sub/untracked.txt\n"),
            "{patch}"
        );
        assert!(patch.contains("+u1\n+u2\n"), "{patch}");
    }

    let gone = repo
        .diff("gone.txt", &DiffSource::HeadToWorkTree)
        .expect("diff");
    assert!(
        gone.contains("deleted file mode") && gone.contains("-bye\n"),
        "{gone}"
    );
}

#[test]
fn an_untracked_file_patch_applies_to_the_index() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "keep.txt", "k\n");
    commit_all(&git, &dir);
    write(&dir, "sub/new.txt", "x\ny\n");
    let patch = repo
        .diff("sub/new.txt", &DiffSource::HeadToWorkTree)
        .expect("diff");
    assert!(
        repo.can_apply(&patch, ApplyTarget::Index, false)
            .expect("check")
    );
    repo.apply(&patch, ApplyTarget::Index, false)
        .expect("apply");
    let staged = repo
        .diff("sub/new.txt", &DiffSource::HeadToIndex)
        .expect("diff");
    assert!(staged.contains("+x\n+y\n"), "{staged}");
}

#[test]
fn diff_on_an_unborn_branch_uses_the_empty_tree() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "first.txt", "one\n");
    raw(&git, dir.path(), &["add", "first.txt"]);
    write(&dir, "loose.txt", "two\n");
    for source in [DiffSource::HeadToIndex, DiffSource::HeadToWorkTree] {
        let patch = repo.diff("first.txt", &source).expect("diff");
        assert!(
            patch.contains("new file mode") && patch.contains("+one\n"),
            "{patch}"
        );
    }
    let loose = repo
        .diff("loose.txt", &DiffSource::HeadToWorkTree)
        .expect("diff");
    assert!(loose.contains("+two\n"), "{loose}");
    assert_eq!(
        repo.file_at("first.txt", &FileVersion::Head).expect("at"),
        None
    );
    let stats = repo.numstat(&DiffSource::HeadToIndex).expect("numstat");
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].path, "first.txt");
}

#[test]
fn diff_of_a_commit_and_of_a_root_commit() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "f.txt", "one\n");
    commit_all(&git, &dir);
    write(&dir, "f.txt", "one\ntwo\n");
    commit_all(&git, &dir);

    let second = repo
        .diff("f.txt", &DiffSource::Commit("HEAD".into()))
        .expect("diff");
    assert!(
        second.contains("+two\n") && !second.contains("new file"),
        "{second}"
    );
    let root = repo
        .diff("f.txt", &DiffSource::Commit("HEAD~1".into()))
        .expect("diff");
    assert!(
        root.contains("new file mode") && root.contains("+one\n"),
        "{root}"
    );
    let stats = repo
        .numstat(&DiffSource::Commit("HEAD~1".into()))
        .expect("numstat");
    assert_eq!((stats[0].additions, stats[0].deletions), (1, 0));
    assert!(
        repo.diff("f.txt", &DiffSource::Commit("--oops".into()))
            .is_err()
    );
}

#[test]
fn diff_of_a_staged_rename_and_numstat() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "old.txt", BASE);
    write(&dir, "bin.dat", "x");
    commit_all(&git, &dir);
    raw(&git, dir.path(), &["mv", "old.txt", "new.txt"]);
    write(&dir, "new.txt", &BASE.replace("l2\n", "L2\n"));
    raw(&git, dir.path(), &["add", "new.txt"]);
    fs::write(dir.path().join("bin.dat"), [0u8, 1, 2, 0, 255]).expect("write");

    let patch = repo
        .diff("new.txt", &DiffSource::HeadToIndex)
        .expect("diff");
    assert!(patch.contains("+++ b/new.txt"), "{patch}");
    let stats = repo.numstat(&DiffSource::HeadToWorkTree).expect("numstat");
    let new = stats
        .iter()
        .find(|s| s.path == "new.txt")
        .expect("renamed entry");
    assert_eq!((new.additions, new.deletions, new.binary), (1, 1, false));
    let bin = stats
        .iter()
        .find(|s| s.path == "bin.dat")
        .expect("binary entry");
    assert!(bin.binary);
}

#[test]
fn numstat_is_limited_to_the_workspace_folder() {
    let Some(git) = git() else { return };
    let (dir, _) = new_repo(&git);
    write(&dir, "ws/a.txt", "a\n");
    write(&dir, "other/b.txt", "b\n");
    commit_all(&git, &dir);
    write(&dir, "ws/a.txt", "a\nmore\n");
    write(&dir, "other/b.txt", "b\nmore\n");
    let ws = git
        .discover(&dir.path().join("ws"))
        .expect("discover")
        .expect("a repo");
    let stats = ws.numstat(&DiffSource::HeadToWorkTree).expect("numstat");
    assert_eq!(
        stats.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(),
        ["ws/a.txt"]
    );
}

#[test]
fn file_at_reads_head_index_and_commits() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "d/f.txt", "v1\n");
    commit_all(&git, &dir);
    write(&dir, "d/f.txt", "v2\n");
    raw(&git, dir.path(), &["add", "d/f.txt"]);
    write(&dir, "d/f.txt", "v3\n");
    let at = |v: FileVersion| repo.file_at("d/f.txt", &v).expect("file_at");
    assert_eq!(at(FileVersion::Head).as_deref(), Some("v1\n"));
    assert_eq!(at(FileVersion::Index).as_deref(), Some("v2\n"));
    assert_eq!(
        at(FileVersion::Commit("HEAD".into())).as_deref(),
        Some("v1\n")
    );
    assert_eq!(
        repo.file_at("nope.txt", &FileVersion::Head).expect("at"),
        None
    );
    assert_eq!(
        repo.file_at("nope.txt", &FileVersion::Index).expect("at"),
        None
    );
}

#[test]
fn one_hunk_can_be_staged_alone() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "f.txt", BASE);
    commit_all(&git, &dir);
    write(&dir, "f.txt", &two_hunks());

    let patch = repo
        .diff("f.txt", &DiffSource::HeadToWorkTree)
        .expect("diff");
    let second = hunk_patch(&patch, 1).expect("second hunk");
    assert!(
        repo.can_apply(&second, ApplyTarget::Index, false)
            .expect("check")
    );
    repo.apply(&second, ApplyTarget::Index, false)
        .expect("apply");

    let staged = repo.diff("f.txt", &DiffSource::HeadToIndex).expect("diff");
    assert!(
        staged.contains("+L19\n") && !staged.contains("+L2\n"),
        "{staged}"
    );
    let unstaged = repo
        .diff("f.txt", &DiffSource::IndexToWorkTree)
        .expect("diff");
    assert!(
        unstaged.contains("+L2\n") && !unstaged.contains("+L19\n"),
        "{unstaged}"
    );

    // Staging it again is no longer possible, unstaging it is.
    assert!(
        !repo
            .can_apply(&second, ApplyTarget::Index, false)
            .expect("check")
    );
    assert!(
        repo.can_apply(&second, ApplyTarget::Index, true)
            .expect("check")
    );
    repo.apply(&second, ApplyTarget::Index, true)
        .expect("unstage");
    assert_eq!(
        repo.diff("f.txt", &DiffSource::HeadToIndex).expect("diff"),
        ""
    );
}

#[test]
fn one_hunk_can_be_reverted_in_the_work_tree() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "f.txt", BASE);
    commit_all(&git, &dir);
    write(&dir, "f.txt", &two_hunks());

    let patch = repo
        .diff("f.txt", &DiffSource::HeadToWorkTree)
        .expect("diff");
    let first = hunk_patch(&patch, 0).expect("first hunk");
    repo.apply(&first, ApplyTarget::WorkTree, true)
        .expect("revert");
    let content = fs::read_to_string(dir.path().join("f.txt")).expect("read");
    assert_eq!(content, BASE.replace("l19\n", "L19\n"));
    assert!(
        repo.apply("not a patch", ApplyTarget::WorkTree, false)
            .is_err()
    );
    assert!(
        !repo
            .can_apply("not a patch", ApplyTarget::Both, false)
            .expect("check")
    );
}

#[test]
fn apply_handles_a_large_patch_on_stdin() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    let big: String = (0..200_000).map(|i| format!("line {i}\n")).collect();
    write(&dir, "keep.txt", "k\n");
    commit_all(&git, &dir);
    write(&dir, "big.txt", &big);
    let patch = repo
        .diff("big.txt", &DiffSource::HeadToWorkTree)
        .expect("diff");
    assert!(patch.len() > 1_000_000);
    repo.apply(&patch, ApplyTarget::Index, false)
        .expect("apply");
    assert_eq!(
        repo.diff("big.txt", &DiffSource::IndexToWorkTree)
            .expect("diff"),
        ""
    );
}

/// 30 lines `l1` to `l30`, with the lines in `changes` (1-based) replaced and, when `drop_12`,
/// line 12 turned into two lines.
fn thirty_lines(changed: &[usize], split_12: bool) -> String {
    (1..=30)
        .map(|n| {
            if split_12 && n == 12 {
                "x12a\nx12b\n".to_string()
            } else if changed.contains(&n) {
                format!("x{n}\n")
            } else {
                format!("l{n}\n")
            }
        })
        .collect()
}

#[test]
fn single_blocks_of_a_full_context_patch_stage_unstage_and_revert() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    let base = thirty_lines(&[], false);
    write(&dir, "f.txt", &base);
    commit_all(&git, &dir);
    write(&dir, "f.txt", &thirty_lines(&[2, 25], true));

    let source = DiffSource::HeadToWorkTree;
    let patch = repo.diff_full("f.txt", &source).expect("diff");
    let diff = parse_file_diff(&patch);
    assert_eq!(diff.blocks.len(), 3);
    assert_eq!(diff.lines.len(), 30 + 1 + 3);
    let blocks: Vec<String> = (0..3)
        .map(|n| block_patch(&patch, &diff, n).expect("block"))
        .collect();

    // Stage block 1 only.
    assert!(
        repo.can_apply(&blocks[0], ApplyTarget::Index, false)
            .expect("check")
    );
    repo.apply(&blocks[0], ApplyTarget::Index, false)
        .expect("stage");
    let staged = repo.diff("f.txt", &DiffSource::HeadToIndex).expect("diff");
    let changes: Vec<&str> = staged
        .lines()
        .filter(|l| (l.starts_with('+') || l.starts_with('-')) && !l.starts_with("+++"))
        .filter(|l| !l.starts_with("---"))
        .collect();
    assert_eq!(changes, vec!["-l2", "+x2"], "{staged}");

    // A staged block cannot be staged again, only unstaged; the others the other way round.
    assert!(
        !repo
            .can_apply(&blocks[0], ApplyTarget::Index, false)
            .expect("check")
    );
    assert!(
        repo.can_apply(&blocks[0], ApplyTarget::Index, true)
            .expect("check")
    );
    assert!(
        repo.can_apply(&blocks[1], ApplyTarget::Index, false)
            .expect("check")
    );
    assert!(
        !repo
            .can_apply(&blocks[1], ApplyTarget::Index, true)
            .expect("check")
    );

    // Unstage it again.
    repo.apply(&blocks[0], ApplyTarget::Index, true)
        .expect("unstage");
    assert_eq!(
        repo.diff("f.txt", &DiffSource::HeadToIndex).expect("diff"),
        ""
    );

    // Revert block 2 in the work tree: only that change is undone.
    repo.apply(&blocks[1], ApplyTarget::WorkTree, true)
        .expect("revert");
    let content = fs::read_to_string(dir.path().join("f.txt")).expect("read");
    let expected = base.replace("l2\n", "x2\n").replace("l25\n", "x25\n");
    assert_eq!(content, expected);
}

#[test]
fn a_block_at_the_end_without_newline_applies() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    write(&dir, "f.txt", "a\nb\nc\n");
    commit_all(&git, &dir);
    write(&dir, "f.txt", "a\nb\nz");
    let patch = repo
        .diff_full("f.txt", &DiffSource::IndexToWorkTree)
        .expect("diff");
    let diff = parse_file_diff(&patch);
    let block = block_patch(&patch, &diff, 0).expect("block");
    assert!(block.contains("\\ No newline at end of file\n"), "{block}");
    repo.apply(&block, ApplyTarget::Index, false)
        .expect("stage");
    assert_eq!(
        repo.diff("f.txt", &DiffSource::IndexToWorkTree)
            .expect("diff"),
        ""
    );
}
