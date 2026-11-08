//! Branches, checkout, stash, log and graph against a real `git`. They skip themselves when git
//! is missing, and never touch the network.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

use telaio_git::{
    Executable, FailureKind, Git, GraphLayout, LogQuery, RefKind, RefLabelKind, Repo,
    auto_stash_message, detect,
};
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

fn new_repo(git: &Git) -> (TempDir, Repo) {
    let dir = tempfile::tempdir().expect("tempdir");
    git.init(dir.path()).expect("init");
    raw(
        git,
        dir.path(),
        &["symbolic-ref", "HEAD", "refs/heads/main"],
    );
    for (key, value) in [
        ("user.name", "Test User"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
    ] {
        raw(git, dir.path(), &["config", "--local", key, value]);
    }
    let repo = git.discover(dir.path()).expect("discover").expect("a repo");
    (dir, repo)
}

fn write(dir: &TempDir, path: &str, content: &str) {
    let full = dir.path().join(path);
    fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
    fs::write(full, content).expect("write");
}

fn commit_file(repo: &Repo, dir: &TempDir, path: &str, content: &str, message: &str) {
    write(dir, path, content);
    repo.stage(&[path.to_string()]).expect("stage");
    repo.commit(message, false).expect("commit");
}

fn current(repo: &Repo) -> String {
    repo.branches()
        .expect("branches")
        .into_iter()
        .find(|b| b.is_head)
        .map(|b| b.name)
        .expect("a current branch")
}

/// A repository with `main` (a, b), a branch `feature` (c) and a merge of it into main.
fn merged_repo(git: &Git) -> (TempDir, Repo) {
    let (dir, repo) = new_repo(git);
    commit_file(&repo, &dir, "a.txt", "a\n", "first");
    repo.create_branch("feature", None, true).expect("branch");
    commit_file(&repo, &dir, "c.txt", "c\n", "side");
    repo.checkout("main").expect("back");
    commit_file(&repo, &dir, "b.txt", "b\n", "second");
    raw(
        git,
        dir.path(),
        &["merge", "--no-ff", "-m", "merge feature", "feature"],
    );
    (dir, repo)
}

#[test]
fn lists_branches_tags_and_remote_branches() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    // The remote.
    let origin = tempfile::tempdir().expect("tempdir");
    raw(
        &git,
        origin.path(),
        &["init", "--quiet", "--bare", "--initial-branch=main"],
    );
    let url = origin.path().to_string_lossy().to_string();
    raw(&git, dir.path(), &["remote", "add", "origin", &url]);

    assert!(repo.branches().expect("empty repo").is_empty());
    commit_file(&repo, &dir, "a.txt", "a\n", "first");
    raw(
        &git,
        dir.path(),
        &["push", "--quiet", "-u", "origin", "main"],
    );
    raw(&git, dir.path(), &["tag", "light"]);
    raw(
        &git,
        dir.path(),
        &["tag", "-a", "v1", "-m", "release notes"],
    );
    commit_file(&repo, &dir, "b.txt", "b\n", "second");
    repo.create_branch("topic", None, false).expect("create");

    let all = repo.branches().expect("branches");
    let main = all.iter().find(|b| b.name == "main").expect("main");
    assert_eq!(main.kind, RefKind::Local);
    assert!(main.is_head);
    assert_eq!(main.subject, "second");
    assert_eq!(main.upstream.as_deref(), Some("origin/main"));
    assert_eq!((main.ahead, main.behind), (1, 0));
    assert!(main.date > 0);
    let topic = all.iter().find(|b| b.name == "topic").expect("topic");
    assert!(!topic.is_head);
    assert_eq!(topic.upstream, None);

    let remote = all
        .iter()
        .find(|b| b.name == "origin/main")
        .expect("remote");
    assert_eq!(
        remote.kind,
        RefKind::Remote {
            remote: "origin".to_string()
        }
    );
    assert!(all.iter().all(|b| b.name != "origin/HEAD"));
    let light = all.iter().find(|b| b.name == "light").expect("tag");
    assert_eq!(light.kind, RefKind::Tag);
    let annotated = all.iter().find(|b| b.name == "v1").expect("annotated tag");
    assert_eq!(annotated.subject, "first");
    assert!(annotated.date > 0);
}

#[test]
fn checkout_create_rename_and_delete() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    commit_file(&repo, &dir, "a.txt", "a\n", "first");

    assert!(repo.check_branch_name("feature/x").expect("check"));
    assert!(!repo.check_branch_name("bad name").expect("check"));
    assert!(!repo.check_branch_name("-x").expect("check"));
    assert!(!repo.check_branch_name("").expect("check"));

    repo.create_branch("feature/x", None, true).expect("create");
    assert_eq!(current(&repo), "feature/x");
    commit_file(&repo, &dir, "x.txt", "x\n", "on x");
    repo.checkout("main").expect("main");
    assert_eq!(current(&repo), "main");

    // Not merged: -d refuses with a classifiable error, -D deletes.
    let err = repo
        .delete_branch("feature/x", false)
        .expect_err("not merged");
    assert_eq!(err.kind(), FailureKind::NotMerged);
    repo.rename_branch("feature/x", "feature/y")
        .expect("rename");
    repo.delete_branch("feature/y", true).expect("force delete");
    repo.create_branch("tmp", Some("HEAD"), false)
        .expect("create");
    repo.delete_branch("tmp", false).expect("merged delete");

    let first = repo
        .log(&LogQuery {
            limit: 1,
            ..LogQuery::default()
        })
        .expect("log");
    repo.checkout_detached(&first[0].oid).expect("detach");
    assert!(
        repo.branches()
            .expect("branches")
            .iter()
            .all(|b| !b.is_head)
    );
    repo.checkout("main").expect("back");

    // Bad names never reach git as options.
    assert!(repo.checkout("--detach").is_err());
    assert!(repo.create_branch("-x", None, false).is_err());
}

#[test]
fn checking_out_a_remote_branch_creates_a_tracking_branch() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    let origin = tempfile::tempdir().expect("tempdir");
    raw(
        &git,
        origin.path(),
        &["init", "--quiet", "--bare", "--initial-branch=main"],
    );
    let url = origin.path().to_string_lossy().to_string();
    raw(&git, dir.path(), &["remote", "add", "origin", &url]);
    commit_file(&repo, &dir, "a.txt", "a\n", "first");
    repo.create_branch("feature/x", None, true).expect("create");
    commit_file(&repo, &dir, "x.txt", "x\n", "on x");
    raw(
        &git,
        dir.path(),
        &["push", "--quiet", "origin", "main", "feature/x"],
    );
    repo.checkout("main").expect("main");
    raw(&git, dir.path(), &["branch", "-D", "feature/x"]);
    raw(&git, dir.path(), &["fetch", "--quiet", "origin"]);

    repo.checkout("origin/feature/x").expect("track");
    assert_eq!(current(&repo), "feature/x");
    let info = repo
        .branches()
        .expect("branches")
        .into_iter()
        .find(|b| b.name == "feature/x")
        .expect("local branch");
    assert_eq!(info.upstream.as_deref(), Some("origin/feature/x"));

    // With the local branch there, the remote name switches to it.
    repo.checkout("main").expect("main");
    repo.checkout("origin/feature/x").expect("existing local");
    assert_eq!(current(&repo), "feature/x");
}

#[test]
fn refuses_checkout_with_local_changes() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    commit_file(&repo, &dir, "a.txt", "a\n", "first");
    repo.create_branch("other", None, true).expect("create");
    commit_file(&repo, &dir, "a.txt", "other\n", "change a");
    repo.checkout("main").expect("main");
    write(&dir, "a.txt", "dirty\n");
    let err = repo.checkout("other").expect_err("refused");
    assert_eq!(err.kind(), FailureKind::LocalChanges, "{err}");
}

#[test]
fn recent_branches_follow_the_reflog() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    assert!(repo.recent_branches(5).expect("no commits").is_empty());
    commit_file(&repo, &dir, "a.txt", "a\n", "first");
    for name in ["one", "two", "three"] {
        repo.create_branch(name, None, false).expect("create");
    }
    repo.checkout("one").expect("one");
    repo.checkout("two").expect("two");
    repo.checkout("one").expect("one");
    repo.delete_branch("three", false).expect("delete");
    // The current branch first, then the most recent ones, no duplicates, only existing ones.
    assert_eq!(
        repo.recent_branches(10).expect("recent"),
        ["one", "two", "main"]
    );
    assert_eq!(repo.recent_branches(2).expect("recent"), ["one", "two"]);
}

#[test]
fn stash_round_trip() {
    const PREFIX: &str = "app-auto:";
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    commit_file(&repo, &dir, "a.txt", "a\n", "first");
    assert!(!repo.stash_push("nothing").expect("nothing to stash"));
    assert_eq!(repo.find_auto_stash(PREFIX, "main").expect("find"), None);

    write(&dir, "a.txt", "changed\n");
    write(&dir, "new.txt", "untracked\n");
    let message = auto_stash_message(PREFIX, "main");
    assert!(message.starts_with(PREFIX));
    assert!(repo.stash_push(&message).expect("stash"));
    assert!(!dir.path().join("new.txt").exists());

    let list = repo.stash_list().expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].message, message);
    assert_eq!(repo.find_auto_stash(PREFIX, "main").expect("find"), Some(0));
    assert_eq!(repo.find_auto_stash(PREFIX, "other").expect("find"), None);

    repo.stash_pop(0).expect("pop");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).expect("read"),
        "changed\n"
    );
    assert!(dir.path().join("new.txt").exists());
    assert!(repo.stash_list().expect("list").is_empty());
}

#[test]
fn log_pages_filters_and_decorations() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    assert!(
        repo.log(&LogQuery {
            limit: 10,
            ..LogQuery::default()
        })
        .expect("empty")
        .is_empty()
    );
    for i in 0..7 {
        let path = if i % 2 == 0 {
            "src/even.txt"
        } else {
            "odd.txt"
        };
        commit_file(
            &repo,
            &dir,
            path,
            &format!("{i}\n"),
            &format!("commit number {i}"),
        );
    }
    raw(&git, dir.path(), &["tag", "v1"]);
    raw(&git, dir.path(), &["tag", "-a", "v2", "-m", "annotated"]);

    let query = |skip, limit| LogQuery {
        skip,
        limit,
        ..LogQuery::default()
    };
    let all = repo.log(&query(0, 100)).expect("all");
    assert_eq!(all.len(), 7);
    assert_eq!(all[0].subject, "commit number 6");
    assert_eq!(all[0].parents, [all[1].oid.clone()]);
    assert!(all[6].parents.is_empty());
    assert_eq!(all[0].author_name, "Test User");
    assert_eq!(all[0].author_email, "test@example.com");
    assert!(all[0].date > 0);
    assert!(all[0].oid.starts_with(&all[0].short));
    let kinds: Vec<_> = all[0]
        .refs
        .iter()
        .map(|r| (r.kind, r.name.as_str()))
        .collect();
    assert!(kinds.contains(&(RefLabelKind::Head, "main")), "{kinds:?}");
    assert!(kinds.contains(&(RefLabelKind::Tag, "v1")), "{kinds:?}");
    assert!(kinds.contains(&(RefLabelKind::Tag, "v2")), "{kinds:?}");

    // Paging continues exactly where the previous page ended.
    let first = repo.log(&query(0, 3)).expect("page 1");
    let second = repo.log(&query(3, 3)).expect("page 2");
    let third = repo.log(&query(6, 3)).expect("page 3");
    let paged: Vec<_> = first.iter().chain(&second).chain(&third).collect();
    assert_eq!(paged.len(), 7);
    assert!(paged.iter().zip(&all).all(|(a, b)| a.oid == b.oid));
    assert!(repo.log(&query(0, 0)).expect("zero").is_empty());

    // Filters.
    let by_path = LogQuery {
        path: Some("src".into()),
        limit: 100,
        ..LogQuery::default()
    };
    assert_eq!(repo.log(&by_path).expect("path").len(), 4);
    let by_author = LogQuery {
        author: Some("TEST user".into()),
        limit: 100,
        ..LogQuery::default()
    };
    assert_eq!(repo.log(&by_author).expect("author").len(), 7);
    let nobody = LogQuery {
        author: Some("nobody".into()),
        limit: 100,
        ..LogQuery::default()
    };
    assert!(repo.log(&nobody).expect("author").is_empty());
    let by_text = LogQuery {
        text: Some("NUMBER 3".into()),
        limit: 100,
        ..LogQuery::default()
    };
    let found = repo.log(&by_text).expect("text");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].subject, "commit number 3");
    // Regex characters are plain text.
    let regex = LogQuery {
        text: Some("(".into()),
        limit: 100,
        ..LogQuery::default()
    };
    assert!(repo.log(&regex).expect("text").is_empty());

    // A hash prefix finds the commit, once, at the top.
    let target = &all[4];
    let by_hash = LogQuery {
        text: Some(target.short.clone()),
        limit: 100,
        ..LogQuery::default()
    };
    let found = repo.log(&by_hash).expect("hash");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].oid, target.oid);
    // ... unless another filter rules it out.
    let excluded = LogQuery {
        author: Some("nobody".into()),
        ..by_hash.clone()
    };
    assert!(repo.log(&excluded).expect("hash").is_empty());
    // Paging the hash result: skip 1 skips the hash match, nothing else matches.
    let paged = LogQuery { skip: 1, ..by_hash };
    assert!(repo.log(&paged).expect("hash page").is_empty());

    // A branch limits the history to it.
    repo.create_branch("old", Some(&all[5].oid), false)
        .expect("old");
    let on_old = LogQuery {
        branch: Some("old".into()),
        limit: 100,
        ..LogQuery::default()
    };
    assert_eq!(repo.log(&on_old).expect("branch").len(), 2);
}

#[test]
fn commit_detail_lists_files_with_counts() {
    let Some(git) = git() else { return };
    let (dir, repo) = new_repo(&git);
    commit_file(
        &repo,
        &dir,
        "a.txt",
        "one\ntwo\nthree\n",
        "root\n\nbody line",
    );
    let root = repo
        .log(&LogQuery {
            limit: 1,
            ..LogQuery::default()
        })
        .expect("log")
        .remove(0);
    let detail = repo.commit_detail(&root.oid).expect("root detail");
    assert_eq!(detail.oid, root.oid);
    assert!(detail.parents.is_empty());
    assert_eq!(detail.message, "root\n\nbody line");
    assert_eq!(detail.author_name, "Test User");
    assert_eq!(detail.files.len(), 1);
    assert_eq!(
        (detail.files[0].letter, detail.files[0].additions),
        ('A', 3)
    );

    // Modify one file, delete none, rename another, add a binary one, in one commit.
    write(&dir, "a.txt", "one\nTWO\nthree\nfour\n");
    write(&dir, "bin.dat", "x\0y\0z");
    repo.stage(&["a.txt".to_string(), "bin.dat".to_string()])
        .expect("stage");
    repo.commit("edit", false).expect("commit");
    commit_file(&repo, &dir, "keep.txt", "k\n", "keep");
    raw(&git, dir.path(), &["mv", "keep.txt", "moved.txt"]);
    repo.commit("rename", false).expect("commit");

    let head = repo
        .log(&LogQuery {
            limit: 1,
            ..LogQuery::default()
        })
        .expect("log")
        .remove(0);
    let renamed = repo.commit_detail(&head.oid).expect("rename detail");
    assert_eq!(renamed.files.len(), 1);
    assert_eq!(renamed.files[0].letter, 'R');
    assert_eq!(renamed.files[0].path, "moved.txt");
    assert_eq!(renamed.files[0].orig_path.as_deref(), Some("keep.txt"));

    let edit = repo.commit_detail("HEAD~2").expect("edit detail");
    let a = edit.files.iter().find(|f| f.path == "a.txt").expect("a");
    assert_eq!((a.letter, a.additions, a.deletions), ('M', 2, 1));
    let bin = edit
        .files
        .iter()
        .find(|f| f.path == "bin.dat")
        .expect("bin");
    assert!(bin.binary);
    assert_eq!(bin.letter, 'A');
    assert!(repo.commit_detail("--output=x").is_err());
}

#[test]
fn merge_detail_is_against_the_first_parent_and_feeds_the_graph() {
    let Some(git) = git() else { return };
    let (_dir, repo) = merged_repo(&git);
    let commits = repo
        .log(&LogQuery {
            limit: 100,
            ..LogQuery::default()
        })
        .expect("log");
    let subjects: Vec<_> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects.len(), 4);
    assert_eq!(subjects[0], "merge feature");

    // The merge brought in c.txt (relative to main's first parent).
    let merge = repo.commit_detail(&commits[0].oid).expect("merge detail");
    assert_eq!(merge.parents.len(), 2);
    let paths: Vec<_> = merge.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["c.txt"]);

    // Feed the graph in two pages, as the app does.
    let mut layout = GraphLayout::new();
    let mut rows = Vec::new();
    for page in [
        repo.log(&LogQuery {
            limit: 2,
            ..LogQuery::default()
        })
        .expect("p1"),
        repo.log(&LogQuery {
            skip: 2,
            limit: 2,
            ..LogQuery::default()
        })
        .expect("p2"),
    ] {
        for commit in &page {
            rows.push(layout.push(&commit.oid, &commit.parents));
        }
    }
    assert_eq!(rows.len(), 4);
    // The merge sits in lane 0 and opens lane 1 for the side branch.
    assert_eq!((rows[0].lane, rows[0].width), (0, 2));
    // The side commit is in lane 1, the root converges both lanes and nothing is left open.
    let side = commits
        .iter()
        .position(|c| c.subject == "side")
        .expect("side");
    assert_eq!(rows[side].lane, 1);
    let root = commits.len() - 1;
    assert_eq!(rows[root].lane, 0);
    assert_eq!(layout.width(), 0);
}
