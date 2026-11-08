//! Commit, push, pull and fetch against a real `git` and a local bare repository as `origin`.
//! They skip themselves when git is missing, and never touch the network.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

use telaio_git::{DiffSource, Executable, FailureKind, Git, PullMode, PushTarget, Repo, detect};
use tempfile::TempDir;

fn git() -> Option<Git> {
    match detect(&Executable::Auto) {
        Ok(git) => Some(git),
        Err(err) => {
            eprintln!("skipping, no usable git: {err}");
            None
        }
    }
}

fn raw(git: &Git, dir: &Path, args: &[&str]) {
    let output = Command::new(git.path())
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

fn identity(git: &Git, dir: &Path) {
    for (key, value) in [
        ("user.name", "Test User"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
    ] {
        raw(git, dir, &["config", "--local", key, value]);
    }
}

fn new_repo(git: &Git) -> (TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    git.init(dir.path()).expect("init");
    raw(
        git,
        dir.path(),
        &["symbolic-ref", "HEAD", "refs/heads/main"],
    );
    identity(git, dir.path());
    let repo = git.discover(dir.path()).expect("discover").expect("a repo");
    (dir, repo)
}

fn write(dir: &TempDir, path: &str, content: &str) {
    let full = dir.path().join(path);
    fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
    fs::write(full, content).expect("write");
}

/// Writes `path`, stages it and commits it through the crate under test.
fn commit_file(repo: &Repo, dir: &TempDir, path: &str, content: &str, message: &str) {
    write(dir, path, content);
    repo.stage(&[path.to_string()]).expect("stage");
    repo.commit(message, false).expect("commit");
}

/// A bare repository on `main` to be used as a remote.
fn bare(git: &Git) -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    raw(
        git,
        dir.path(),
        &["init", "--quiet", "--bare", "--initial-branch=main"],
    );
    dir
}

/// A repo with one commit whose `origin` is a fresh bare repository.
fn with_origin(git: &Git) -> (TempDir, Repo, TempDir) {
    let origin = bare(git);
    let (dir, repo) = new_repo(git);
    raw(
        git,
        dir.path(),
        &[
            "remote",
            "add",
            "origin",
            origin.path().to_str().expect("utf8"),
        ],
    );
    commit_file(&repo, &dir, "a.txt", "one\n", "first");
    (dir, repo, origin)
}

/// Clones `origin` into a new folder with an identity.
fn clone_of(git: &Git, origin: &TempDir) -> (TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().to_str().expect("utf8");
    raw(
        git,
        origin.path(),
        &[
            "clone",
            "--quiet",
            origin.path().to_str().expect("utf8"),
            target,
        ],
    );
    identity(git, dir.path());
    let repo = git.discover(dir.path()).expect("discover").expect("a repo");
    (dir, repo)
}

#[test]
fn first_commit_amend_and_message() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    assert_eq!(repo.last_commit_message().expect("msg"), None);

    write(&dir, "a.txt", "one\n");
    repo.stage(&["a.txt".to_string()]).expect("stage");
    let staged = repo.numstat(&DiffSource::HeadToIndex).expect("numstat");
    assert_eq!(staged.len(), 1);
    assert_eq!((staged[0].path.as_str(), staged[0].additions), ("a.txt", 1));

    repo.commit("First\n\nBody line", false).expect("commit");
    assert_eq!(
        repo.last_commit_message().expect("msg").as_deref(),
        Some("First\n\nBody line")
    );
    assert!(
        repo.numstat(&DiffSource::HeadToIndex)
            .expect("n")
            .is_empty()
    );

    // Rewording with an empty index.
    repo.commit("Reworded", true).expect("amend");
    assert_eq!(
        repo.last_commit_message().expect("msg").as_deref(),
        Some("Reworded")
    );
    let count = Command::new(git.path())
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git");
    assert_eq!(String::from_utf8_lossy(&count.stdout).trim(), "1");
}

#[test]
fn commit_only_takes_what_is_staged_and_fails_when_empty() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    commit_file(&repo, &dir, "a.txt", "one\n", "first");
    write(&dir, "a.txt", "two\n");
    let err = repo.commit("nothing staged", false).expect_err("empty");
    assert_eq!(err.kind(), FailureKind::NothingToCommit);
    assert_eq!(repo.status().expect("status").entries.len(), 1);
}

#[test]
fn push_target_and_first_push_sets_upstream() {
    let Some(git) = git() else { return };
    let (_dir, repo, _origin) = with_origin(&git);
    let target = repo.push_target().expect("target").expect("some");
    assert_eq!(
        target,
        PushTarget {
            remote: "origin".into(),
            branch: "main".into(),
            set_upstream: true
        }
    );
    let pending = repo.unpushed_commits(&target).expect("log");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].subject, "first");

    repo.push(&target).expect("push");
    let status = repo.status().expect("status");
    assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    let target = repo.push_target().expect("target").expect("some");
    assert!(!target.set_upstream);
    assert!(repo.unpushed_commits(&target).expect("log").is_empty());
}

#[test]
fn unpushed_commits_with_upstream() {
    let Some(git) = git() else { return };
    let (dir, repo, _origin) = with_origin(&git);
    let target = repo.push_target().expect("t").expect("some");
    repo.push(&target).expect("push");
    commit_file(&repo, &dir, "b.txt", "b\n", "second");
    commit_file(&repo, &dir, "c.txt", "c\n", "third");
    let target = repo.push_target().expect("t").expect("some");
    let subjects: Vec<String> = repo
        .unpushed_commits(&target)
        .expect("log")
        .into_iter()
        .map(|c| c.subject)
        .collect();
    assert_eq!(subjects, ["third", "second"]);
}

#[test]
fn push_target_is_none_without_remote_or_commits() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    assert_eq!(repo.push_target().expect("t"), None);
    commit_file(&repo, &dir, "a.txt", "one\n", "first");
    assert_eq!(repo.push_target().expect("t"), None);
}

#[test]
fn push_target_needs_a_choice_with_several_remotes() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    commit_file(&repo, &dir, "a.txt", "one\n", "first");
    for name in ["one", "two"] {
        raw(&git, dir.path(), &["remote", "add", name, "/nonexistent"]);
    }
    assert_eq!(repo.push_target().expect("t"), None);
    raw(&git, dir.path(), &["config", "remote.pushDefault", "two"]);
    let target = repo.push_target().expect("t").expect("some");
    assert_eq!(target.remote, "two");
}

#[test]
fn fetch_updates_behind_count() {
    let Some(git) = git() else { return };
    let (_dir, repo, origin) = with_origin(&git);
    let target = repo.push_target().expect("t").expect("some");
    repo.push(&target).expect("push");

    let (other_dir, other) = clone_of(&git, &origin);
    commit_file(&other, &other_dir, "o.txt", "o\n", "from other");
    let other_target = other.push_target().expect("t").expect("some");
    other.push(&other_target).expect("push");

    assert_eq!(repo.status().expect("status").behind, 0);
    repo.fetch().expect("fetch");
    assert_eq!(repo.status().expect("status").behind, 1);
}

/// Two clones with diverging commits on different files; the second one is behind and ahead.
fn diverged(git: &Git) -> (TempDir, Repo, TempDir, Repo, TempDir) {
    let (dir, repo, origin) = with_origin(git);
    let target = repo.push_target().expect("t").expect("some");
    repo.push(&target).expect("push");
    let (other_dir, other) = clone_of(git, &origin);
    commit_file(&other, &other_dir, "o.txt", "o\n", "from other");
    other
        .push(&other.push_target().expect("t").expect("some"))
        .expect("push");
    (dir, repo, other_dir, other, origin)
}

#[test]
fn rejected_push_is_classified() {
    let Some(git) = git() else { return };
    let (dir, repo, _od, _o, _origin) = diverged(&git);
    commit_file(&repo, &dir, "b.txt", "b\n", "local");
    let target = repo.push_target().expect("t").expect("some");
    let err = repo.push(&target).expect_err("rejected");
    assert_eq!(err.kind(), FailureKind::Rejected, "{}", err.output());
    assert!(!err.output().is_empty());
}

#[test]
fn pull_merges_divergent_branches_without_config() {
    let Some(git) = git() else { return };
    let (dir, repo, _od, _o, _origin) = diverged(&git);
    commit_file(&repo, &dir, "b.txt", "b\n", "local");
    repo.fetch().expect("fetch");
    repo.pull(PullMode::FollowConfig).expect("pull");
    let status = repo.status().expect("status");
    assert_eq!(status.behind, 0);
    assert_eq!(status.ahead, 2);
    assert!(dir.path().join("o.txt").exists());
}

#[test]
fn pull_rebase_replays_local_commits() {
    let Some(git) = git() else { return };
    let (dir, repo, _od, _o, _origin) = diverged(&git);
    commit_file(&repo, &dir, "b.txt", "b\n", "local");
    repo.pull(PullMode::Rebase).expect("pull");
    let status = repo.status().expect("status");
    assert_eq!((status.ahead, status.behind), (1, 0));
}

#[test]
fn pull_with_conflicts_is_classified() {
    let Some(git) = git() else { return };
    let (dir, repo, origin) = with_origin(&git);
    repo.push(&repo.push_target().expect("t").expect("some"))
        .expect("push");
    let (other_dir, other) = clone_of(&git, &origin);
    commit_file(&other, &other_dir, "a.txt", "theirs\n", "theirs");
    other
        .push(&other.push_target().expect("t").expect("some"))
        .expect("push");

    commit_file(&repo, &dir, "a.txt", "ours\n", "ours");
    for mode in [PullMode::Merge, PullMode::Rebase] {
        let err = repo.pull(mode).expect_err("conflict");
        assert_eq!(err.kind(), FailureKind::Conflicts, "{}", err.output());
        raw(
            &git,
            dir.path(),
            if mode == PullMode::Merge {
                &["merge", "--abort"]
            } else {
                &["rebase", "--abort"]
            },
        );
    }
}

#[test]
fn gitignore_helper_appends_once() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    repo.add_to_gitignore("environments/dev.local.env")
        .expect("add");
    repo.add_to_gitignore("environments/dev.local.env")
        .expect("again");
    let text = fs::read_to_string(dir.path().join(".gitignore")).expect("read");
    assert_eq!(text, "/environments/dev.local.env\n");

    fs::write(dir.path().join(".gitignore"), "target").expect("write");
    repo.add_to_gitignore("x.local.env").expect("add");
    let text = fs::read_to_string(dir.path().join(".gitignore")).expect("read");
    assert_eq!(text, "target\n/x.local.env\n");
}

#[test]
fn clone_shallow_copies_the_last_commit_of_a_branch() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    commit_file(&repo, &dir, "spec.yaml", "one", "first");
    commit_file(&repo, &dir, "spec.yaml", "two", "second");
    raw(&git, dir.path(), &["branch", "other"]);
    commit_file(&repo, &dir, "spec.yaml", "three", "third");
    // `--depth` is ignored for a plain path, a file:// URL is a real (local) remote.
    let url = format!("file://{}", dir.path().display());

    let target = tempfile::tempdir().expect("tempdir");
    let clone = target.path().join("clone");
    git.clone_shallow(&url, None, &clone)
        .expect("clones the default branch");
    assert_eq!(
        fs::read_to_string(clone.join("spec.yaml")).expect("read"),
        "three"
    );
    let count = Command::new(git.path())
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(&clone)
        .output()
        .expect("git runs");
    assert_eq!(String::from_utf8_lossy(&count.stdout).trim(), "1");

    let branch = target.path().join("branch");
    git.clone_shallow(&url, Some("other"), &branch)
        .expect("clones a branch");
    assert_eq!(
        fs::read_to_string(branch.join("spec.yaml")).expect("read"),
        "two"
    );

    let missing = target.path().join("missing");
    assert!(git.clone_shallow(&url, Some("nope"), &missing).is_err());
}
