//! Integration tests for [`brasa_workspace::Workspace`] against real temporary folders.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use brasa_core::Method;
use brasa_workspace::{EnvChange, EnvLayer, Node, Workspace, WorkspaceError};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

/// Creates an empty file at `path`, creating its parent directories first.
fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, contents).expect("write fixture file");
}

#[test]
fn open_fails_on_a_missing_root() {
    let temp = TempDir::new().expect("temp dir");
    let missing = temp.path().join("does-not-exist");
    let result = Workspace::open(&missing);
    assert!(matches!(result, Err(WorkspaceError::RootNotFound(path)) if path == missing));
}

#[test]
fn open_fails_when_root_is_a_file() {
    let temp = TempDir::new().expect("temp dir");
    let file = temp.path().join("not-a-folder");
    fs::write(&file, "hello").expect("write file");
    assert!(Workspace::open(&file).is_err());
}

#[test]
fn scans_nested_folders_and_requests() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("auth/login.brasa"),
        "GET https://example.com\n",
    );
    write_file(
        &temp.path().join("users/list.brasa"),
        "GET https://example.com/users\n",
    );
    write_file(
        &temp.path().join("ping.brasa"),
        "GET https://example.com/ping\n",
    );

    let workspace = Workspace::open(temp.path()).expect("open workspace");
    let tree = workspace.tree();

    // Folders first, then requests, natural sort within each group: "auth", "users", "ping".
    assert_eq!(tree.len(), 3);
    let names: Vec<&str> = tree.iter().map(Node::name).collect();
    assert_eq!(names, vec!["auth", "users", "ping"]);

    let Node::Folder(auth) = &tree[0] else {
        panic!("expected a folder")
    };
    assert_eq!(auth.id, "auth");
    assert_eq!(auth.children.len(), 1);
    let Node::Request(login) = &auth.children[0] else {
        panic!("expected a request")
    };
    assert_eq!(login.id, "auth/login.brasa");
    assert_eq!(login.name, "login");
    assert_eq!(login.broken, None);
    assert_eq!(login.method, Some(Method::Get));
}

#[test]
fn method_is_read_from_the_parsed_request_without_a_second_read() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("create.brasa"),
        "POST https://example.com/users\n",
    );

    let workspace = Workspace::open(temp.path()).expect("open workspace");
    let Node::Request(create) = &workspace.tree()[0] else {
        panic!("expected a request node")
    };
    assert_eq!(create.method, Some(Method::Post));
}

#[test]
fn ids_use_forward_slashes_regardless_of_nesting_depth() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("a/b/c/deep.brasa"),
        "GET https://example.com\n",
    );
    let workspace = Workspace::open(temp.path()).expect("open workspace");

    let Node::Folder(a) = &workspace.tree()[0] else {
        panic!("expected folder a")
    };
    assert_eq!(a.id, "a");
    let Node::Folder(b) = &a.children[0] else {
        panic!("expected folder b")
    };
    assert_eq!(b.id, "a/b");
    let Node::Folder(c) = &b.children[0] else {
        panic!("expected folder c")
    };
    assert_eq!(c.id, "a/b/c");
    let Node::Request(deep) = &c.children[0] else {
        panic!("expected request")
    };
    assert_eq!(deep.id, "a/b/c/deep.brasa");
}

#[test]
fn hidden_folders_and_target_are_ignored() {
    let temp = TempDir::new().expect("temp dir");
    write_file(&temp.path().join(".git/config"), "");
    write_file(&temp.path().join("target/debug/build"), "");
    write_file(
        &temp.path().join("visible/request.brasa"),
        "GET https://example.com\n",
    );

    let workspace = Workspace::open(temp.path()).expect("open workspace");
    let names: Vec<&str> = workspace.tree().iter().map(Node::name).collect();
    assert_eq!(names, vec!["visible"]);
}

#[test]
fn node_modules_folders_are_ignored_at_every_level() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("node_modules/pkg/hidden.brasa"),
        "GET https://example.com\n",
    );
    write_file(
        &temp.path().join("api/node_modules/pkg/hidden.brasa"),
        "GET https://example.com\n",
    );
    write_file(
        &temp.path().join("api/ping.brasa"),
        "GET https://example.com\n",
    );

    let workspace = Workspace::open(temp.path()).expect("open workspace");
    let names: Vec<&str> = workspace.tree().iter().map(Node::name).collect();
    assert_eq!(names, vec!["api"]);
    let Node::Folder(api) = &workspace.tree()[0] else {
        panic!("expected folder api")
    };
    let children: Vec<&str> = api.children.iter().map(Node::name).collect();
    assert_eq!(children, vec!["ping"]);
}

#[test]
fn folders_without_requests_are_left_out_unless_empty() {
    let temp = TempDir::new().expect("temp dir");
    // A code repository around the requests: none of these folders holds a request.
    write_file(&temp.path().join("src/main.rs"), "fn main() {}\n");
    write_file(&temp.path().join("src/nested/lib.rs"), "");
    write_file(&temp.path().join("docs/README.md"), "# Docs\n");
    // Kept: a request below it, even next to other files or only deep down.
    write_file(&temp.path().join("api/README.md"), "# API\n");
    write_file(
        &temp.path().join("api/users/list.brasa"),
        "GET https://example.com\n",
    );
    // Kept: empty, or empty but for hidden files (`.gitkeep`) or empty subfolders.
    fs::create_dir_all(temp.path().join("empty")).expect("create empty folder");
    write_file(&temp.path().join("only-hidden/.gitkeep"), "");
    fs::create_dir_all(temp.path().join("parent/child")).expect("create nested empty folder");

    let workspace = Workspace::open(temp.path()).expect("open workspace");
    let names: Vec<&str> = workspace.tree().iter().map(Node::name).collect();
    assert_eq!(names, vec!["api", "empty", "only-hidden", "parent"]);
    let Node::Folder(api) = &workspace.tree()[0] else {
        panic!("expected folder api")
    };
    let children: Vec<&str> = api.children.iter().map(Node::name).collect();
    assert_eq!(children, vec!["users"]);
}

#[cfg(unix)]
#[test]
fn an_unreadable_subfolder_is_skipped_instead_of_failing_the_open() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("locked/secret.brasa"),
        "GET https://example.com\n",
    );
    write_file(
        &temp.path().join("open/ping.brasa"),
        "GET https://example.com\n",
    );
    let locked = temp.path().join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("lock folder");
    // Root ignores permissions: nothing to check then.
    let readable_anyway = fs::read_dir(&locked).is_ok();

    let result = Workspace::open(temp.path());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock folder");
    if readable_anyway {
        return;
    }
    let workspace = result.expect("open workspace");
    let names: Vec<&str> = workspace.tree().iter().map(Node::name).collect();
    assert_eq!(names, vec!["open"]);
}

#[test]
fn top_level_environments_folder_is_skipped_from_the_tree() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("environments/dev.env"),
        "BASE_URL=https://dev.test\n",
    );
    write_file(
        &temp.path().join("ping.brasa"),
        "GET https://example.com\n",
    );

    let workspace = Workspace::open(temp.path()).expect("open workspace");
    let names: Vec<&str> = workspace.tree().iter().map(Node::name).collect();
    assert_eq!(names, vec!["ping"]);
}

#[test]
fn a_broken_request_file_is_listed_but_does_not_fail_the_scan() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("good.brasa"),
        "GET https://example.com\n",
    );
    write_file(
        &temp.path().join("broken.brasa"),
        "this is not a valid request file",
    );

    let workspace = Workspace::open(temp.path()).expect("a broken file must not fail the scan");
    let tree = workspace.tree();
    assert_eq!(tree.len(), 2);

    let Node::Request(broken) = tree.iter().find(|node| node.name() == "broken").unwrap() else {
        panic!("expected a request node")
    };
    assert!(
        broken.broken.is_some(),
        "the broken file should be marked broken"
    );
    assert_eq!(broken.method, None);

    let Node::Request(good) = tree.iter().find(|node| node.name() == "good").unwrap() else {
        panic!("expected a request node")
    };
    assert_eq!(good.broken, None);
    assert_eq!(good.method, Some(Method::Get));
}

#[test]
fn load_request_fails_for_a_broken_file_with_a_parse_error() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("broken.brasa"),
        "not a valid request file",
    );
    let workspace = Workspace::open(temp.path()).expect("open workspace");

    let result = workspace.load_request("broken.brasa");
    assert!(matches!(result, Err(WorkspaceError::Parse(_))));
}

#[test]
fn load_request_fails_when_the_id_does_not_exist() {
    let temp = TempDir::new().expect("temp dir");
    let workspace = Workspace::open(temp.path()).expect("open workspace");
    assert!(matches!(
        workspace.load_request("missing.brasa"),
        Err(WorkspaceError::NotFound(_))
    ));
}

#[test]
fn save_request_round_trips_and_is_reflected_in_a_rescan() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("ping.brasa"),
        "GET https://example.com\n",
    );
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let mut request = workspace
        .load_request("ping.brasa")
        .expect("load request");
    request.url = "https://example.com/v2".to_string();
    workspace
        .save_request("ping.brasa", &request)
        .expect("save request");

    let reloaded = workspace
        .load_request("ping.brasa")
        .expect("reload request");
    assert_eq!(reloaded.url, "https://example.com/v2");
}

#[test]
fn save_request_does_not_leave_a_temporary_file_behind() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("ping.brasa"),
        "GET https://example.com\n",
    );
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let request = workspace
        .load_request("ping.brasa")
        .expect("load request");
    workspace
        .save_request("ping.brasa", &request)
        .expect("save request");

    let entries: Vec<_> = fs::read_dir(temp.path())
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(entries, vec!["ping.brasa".to_string()]);
}

#[test]
fn save_request_fails_when_the_parent_folder_does_not_exist() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    let request = brasa_core::Request::default();
    assert!(
        workspace
            .save_request("missing/ping.brasa", &request)
            .is_err()
    );
}

#[test]
fn create_request_creates_a_parseable_file_and_appears_in_the_tree() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let id = workspace
        .create_request(None, "New request")
        .expect("create request");
    assert_eq!(id, "New request.brasa");

    let loaded = workspace
        .load_request(&id)
        .expect("a freshly created request must parse");
    assert!(!loaded.url.is_empty());

    let names: Vec<&str> = workspace.tree().iter().map(Node::name).collect();
    assert_eq!(names, vec!["New request"]);
}

#[test]
fn create_request_sanitizes_the_name() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    let id = workspace
        .create_request(None, "weird/name*here")
        .expect("create request");
    assert_eq!(id, "weird_name_here.brasa");
}

#[test]
fn create_request_inside_a_folder_uses_its_id_as_parent() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    let folder_id = workspace
        .create_folder(None, "auth")
        .expect("create folder");
    let request_id = workspace
        .create_request(Some(&folder_id), "login")
        .expect("create request");
    assert_eq!(request_id, "auth/login.brasa");
}

#[test]
fn create_request_fails_if_it_already_exists() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    workspace
        .create_request(None, "ping")
        .expect("create request");
    assert!(matches!(
        workspace.create_request(None, "ping"),
        Err(WorkspaceError::AlreadyExists(_))
    ));
}

#[test]
fn duplicate_request_copies_the_file_next_to_it() {
    let temp = TempDir::new().expect("temp dir");
    let text = "# a comment kept as is\nGET https://example.com/login\n";
    write_file(&temp.path().join("auth/Login.brasa"), text);
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let id = workspace
        .duplicate_request("auth/Login.brasa", "copy")
        .expect("duplicate");
    assert_eq!(id, "auth/Login copy.brasa");
    let copied = fs::read_to_string(temp.path().join("auth/Login copy.brasa")).expect("copy");
    assert_eq!(copied, text);
    assert!(workspace.load_request(&id).is_ok());
}

#[test]
fn duplicate_request_numbers_the_name_when_it_is_taken() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    workspace.create_request(None, "ping").expect("create");

    let first = workspace.duplicate_request("ping.brasa", "copy").unwrap();
    let second = workspace.duplicate_request("ping.brasa", "copy").unwrap();
    let third = workspace.duplicate_request("ping.brasa", "copy").unwrap();
    assert_eq!(
        [first, second, third],
        [
            "ping copy.brasa",
            "ping copy 2.brasa",
            "ping copy 3.brasa"
        ]
    );
}

#[test]
fn duplicate_request_refuses_a_folder_or_a_missing_request() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    let folder = workspace
        .create_folder(None, "auth")
        .expect("create folder");
    assert!(matches!(
        workspace.duplicate_request(&folder, "copy"),
        Err(WorkspaceError::NotFound(_))
    ));
    assert!(matches!(
        workspace.duplicate_request("missing.brasa", "copy"),
        Err(WorkspaceError::NotFound(_))
    ));
}

#[test]
fn create_folder_fails_for_a_missing_parent() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    assert!(matches!(
        workspace.create_folder(Some("missing"), "child"),
        Err(WorkspaceError::NotFound(_))
    ));
}

#[test]
fn rename_a_request_keeps_its_extension_and_updates_the_id() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("old.brasa"),
        "GET https://example.com\n",
    );
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let new_id = workspace
        .rename("old.brasa", "new")
        .expect("rename request");
    assert_eq!(new_id, "new.brasa");
    assert!(workspace.load_request("new.brasa").is_ok());
    assert!(workspace.load_request("old.brasa").is_err());
}

#[test]
fn rename_a_folder_moves_its_contents() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("old/inside.brasa"),
        "GET https://example.com\n",
    );
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let new_id = workspace.rename("old", "renamed").expect("rename folder");
    assert_eq!(new_id, "renamed");
    assert!(workspace.load_request("renamed/inside.brasa").is_ok());
}

#[test]
fn rename_fails_when_the_target_already_exists() {
    let temp = TempDir::new().expect("temp dir");
    write_file(&temp.path().join("a.brasa"), "GET https://example.com\n");
    write_file(&temp.path().join("b.brasa"), "GET https://example.com\n");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    assert!(matches!(
        workspace.rename("a.brasa", "b"),
        Err(WorkspaceError::AlreadyExists(_))
    ));
}

#[test]
fn move_request_into_a_folder_keeps_its_file_name() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("auth/login.brasa"),
        "POST https://example.com/login\n",
    );
    write_file(&temp.path().join("users/.keep"), "");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let new_id = workspace
        .move_entry("auth/login.brasa", Some("users"))
        .expect("move request");

    assert_eq!(new_id, "users/login.brasa");
    assert_eq!(
        workspace
            .load_request("users/login.brasa")
            .expect("load")
            .url,
        "https://example.com/login"
    );
    assert!(!temp.path().join("auth/login.brasa").exists());
    let ids: Vec<&str> = workspace.tree().iter().map(Node::id).collect();
    assert_eq!(ids, vec!["auth", "users"]);
}

#[test]
fn move_request_to_the_root_and_into_a_subfolder() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("a/b/ping.brasa"),
        "GET https://example.com\n",
    );
    write_file(&temp.path().join("c/d/.keep"), "");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let at_root = workspace
        .move_entry("a/b/ping.brasa", None)
        .expect("move to root");
    assert_eq!(at_root, "ping.brasa");

    let nested = workspace
        .move_entry("ping.brasa", Some("c/d"))
        .expect("move into subfolder");
    assert_eq!(nested, "c/d/ping.brasa");
    assert!(workspace.load_request("c/d/ping.brasa").is_ok());
}

#[test]
fn move_request_to_its_own_folder_changes_nothing() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("auth/login.brasa"),
        "GET https://example.com\n",
    );
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let id = workspace
        .move_entry("auth/login.brasa", Some("auth"))
        .expect("move to the same folder");

    assert_eq!(id, "auth/login.brasa");
    assert!(workspace.load_request("auth/login.brasa").is_ok());
}

#[test]
fn move_request_never_overwrites_a_file_with_the_same_name() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("a/ping.brasa"),
        "GET https://a.example.com\n",
    );
    write_file(
        &temp.path().join("b/ping.brasa"),
        "GET https://b.example.com\n",
    );
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let result = workspace.move_entry("a/ping.brasa", Some("b"));

    assert!(matches!(result, Err(WorkspaceError::AlreadyExists(name)) if name == "ping.brasa"));
    assert_eq!(
        workspace
            .load_request("a/ping.brasa")
            .expect("load a")
            .url,
        "https://a.example.com"
    );
    assert_eq!(
        workspace
            .load_request("b/ping.brasa")
            .expect("load b")
            .url,
        "https://b.example.com"
    );
}

#[test]
fn move_request_fails_for_a_missing_request_folder_or_a_folder_id() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("a/ping.brasa"),
        "GET https://example.com\n",
    );
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    assert!(matches!(
        workspace.move_entry("a/missing.brasa", None),
        Err(WorkspaceError::NotFound(_))
    ));
    assert!(matches!(
        workspace.move_entry("a/ping.brasa", Some("missing")),
        Err(WorkspaceError::NotFound(_))
    ));
    assert!(matches!(
        workspace.move_entry("a/missing", None),
        Err(WorkspaceError::NotFound(_))
    ));
    assert!(matches!(
        workspace.move_entry("../outside.brasa", None),
        Err(WorkspaceError::InvalidId(_))
    ));
    assert!(matches!(
        workspace.move_entry("a/ping.brasa", Some("../..")),
        Err(WorkspaceError::InvalidId(_))
    ));
}

#[test]
fn move_entry_moves_a_folder_with_everything_inside() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("auth/admin/reset.brasa"),
        "POST https://example.com/reset\n",
    );
    write_file(
        &temp.path().join("auth/login.brasa"),
        "POST https://example.com/login\n",
    );
    write_file(&temp.path().join("users/.keep"), "");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let new_id = workspace
        .move_entry("auth", Some("users"))
        .expect("move folder");

    assert_eq!(new_id, "users/auth");
    assert!(!temp.path().join("auth").exists());
    assert!(workspace.load_request("users/auth/login.brasa").is_ok());
    assert!(
        workspace
            .load_request("users/auth/admin/reset.brasa")
            .is_ok()
    );
    let ids: Vec<&str> = workspace.tree().iter().map(Node::id).collect();
    assert_eq!(ids, vec!["users"]);

    let back = workspace
        .move_entry("users/auth/admin", None)
        .expect("move subfolder to the root");
    assert_eq!(back, "admin");
    assert!(workspace.load_request("admin/reset.brasa").is_ok());
}

#[test]
fn move_entry_never_moves_a_folder_into_itself_or_below_it() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("a/b/ping.brasa"),
        "GET https://example.com\n",
    );
    write_file(&temp.path().join("ab/.keep"), "");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    assert!(matches!(
        workspace.move_entry("a", Some("a")),
        Err(WorkspaceError::MoveIntoItself(id)) if id == "a"
    ));
    assert!(matches!(
        workspace.move_entry("a", Some("a/b")),
        Err(WorkspaceError::MoveIntoItself(_))
    ));
    assert!(temp.path().join("a/b/ping.brasa").is_file());

    // A sibling whose name starts with the same letters is not a subfolder.
    assert_eq!(
        workspace.move_entry("a", Some("ab")).expect("move a"),
        "ab/a"
    );
}

#[test]
fn move_entry_never_overwrites_a_folder_with_the_same_name() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("a/auth/login.brasa"),
        "GET https://a.example.com\n",
    );
    write_file(
        &temp.path().join("auth/ping.brasa"),
        "GET https://b.example.com\n",
    );
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let result = workspace.move_entry("a/auth", None);

    assert!(matches!(result, Err(WorkspaceError::AlreadyExists(name)) if name == "auth"));
    assert!(temp.path().join("a/auth/login.brasa").is_file());
    assert!(temp.path().join("auth/ping.brasa").is_file());
}

#[test]
fn delete_removes_a_request_file() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("ping.brasa"),
        "GET https://example.com\n",
    );
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    workspace.delete("ping.brasa").expect("delete request");
    assert!(workspace.tree().is_empty());
    assert!(!temp.path().join("ping.brasa").exists());
}

#[test]
fn delete_removes_a_folder_recursively() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("auth/login.brasa"),
        "GET https://example.com\n",
    );
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    workspace.delete("auth").expect("delete folder");
    assert!(workspace.tree().is_empty());
    assert!(!temp.path().join("auth").exists());
}

#[test]
fn list_environments_ignores_local_env_files() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("environments/dev.env"),
        "BASE_URL=https://dev.test\n",
    );
    write_file(
        &temp.path().join("environments/dev.local.env"),
        "TOKEN=secret\n",
    );
    write_file(
        &temp.path().join("environments/prod.env"),
        "BASE_URL=https://prod.test\n",
    );

    let workspace = Workspace::open(temp.path()).expect("open workspace");
    let environments = workspace.list_environments().expect("list environments");
    assert_eq!(environments, vec!["dev".to_string(), "prod".to_string()]);
}

#[test]
fn list_environments_is_empty_without_an_environments_folder() {
    let temp = TempDir::new().expect("temp dir");
    let workspace = Workspace::open(temp.path()).expect("open workspace");
    assert_eq!(
        workspace.list_environments().expect("list environments"),
        Vec::<String>::new()
    );
}

#[test]
fn load_environment_merges_local_over_base() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("environments/dev.env"),
        "BASE_URL=https://dev.test\nTOKEN=placeholder\n",
    );
    write_file(
        &temp.path().join("environments/dev.local.env"),
        "TOKEN=real-secret\n",
    );

    let workspace = Workspace::open(temp.path()).expect("open workspace");
    let environment = workspace.load_environment("dev").expect("load environment");

    assert_eq!(environment.name, "dev");
    assert_eq!(
        environment.variables,
        vec![
            brasa_core::KeyValue::new("BASE_URL", "https://dev.test"),
            brasa_core::KeyValue::new("TOKEN", "real-secret"),
        ]
    );
}

#[test]
fn load_environment_works_with_only_a_local_file() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("environments/secrets.local.env"),
        "TOKEN=abc\n",
    );

    let workspace = Workspace::open(temp.path()).expect("open workspace");
    let environment = workspace
        .load_environment("secrets")
        .expect("load environment");
    assert_eq!(
        environment.variables,
        vec![brasa_core::KeyValue::new("TOKEN", "abc")]
    );
}

#[test]
fn load_environment_fails_for_an_unknown_name() {
    let temp = TempDir::new().expect("temp dir");
    let workspace = Workspace::open(temp.path()).expect("open workspace");
    assert!(matches!(
        workspace.load_environment("nope"),
        Err(WorkspaceError::EnvironmentNotFound(_))
    ));
}

// --- Environment writes ---------------------------------------------------------------------

#[test]
fn save_environment_set_creates_the_folder_and_file_when_missing() {
    let temp = TempDir::new().expect("temp dir");
    let workspace = Workspace::open(temp.path()).expect("open workspace");

    workspace
        .save_environment(
            "dev",
            &[EnvChange::Set {
                layer: EnvLayer::Base,
                key: "BASE_URL".to_string(),
                value: "https://dev.test".to_string(),
            }],
        )
        .expect("save environment");

    let content =
        fs::read_to_string(temp.path().join("environments/dev.env")).expect("read env file");
    assert_eq!(content, "BASE_URL=https://dev.test\n");
}

#[test]
fn save_environment_set_writes_the_local_variant() {
    let temp = TempDir::new().expect("temp dir");
    let workspace = Workspace::open(temp.path()).expect("open workspace");

    workspace
        .save_environment(
            "dev",
            &[EnvChange::Set {
                layer: EnvLayer::Local,
                key: "TOKEN".to_string(),
                value: "secret".to_string(),
            }],
        )
        .expect("save environment");

    let content = fs::read_to_string(temp.path().join("environments/dev.local.env"))
        .expect("read local env file");
    assert_eq!(content, "TOKEN=secret\n");
    assert!(!temp.path().join("environments/dev.env").exists());
}

#[test]
fn save_environment_set_replaces_a_key_preserving_comments_and_order() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &temp.path().join("environments/dev.env"),
        "# base config\nBASE_URL=https://old.test\nTIMEOUT=30\n",
    );
    let workspace = Workspace::open(temp.path()).expect("open workspace");

    workspace
        .save_environment(
            "dev",
            &[EnvChange::Set {
                layer: EnvLayer::Base,
                key: "BASE_URL".to_string(),
                value: "https://new.test".to_string(),
            }],
        )
        .expect("save environment");

    let content =
        fs::read_to_string(temp.path().join("environments/dev.env")).expect("read env file");
    assert_eq!(
        content,
        "# base config\nBASE_URL=https://new.test\nTIMEOUT=30\n"
    );
}

#[test]
fn save_environment_set_appends_a_missing_key() {
    let temp = TempDir::new().expect("temp dir");
    write_file(&temp.path().join("environments/dev.env"), "A=1\n");
    let workspace = Workspace::open(temp.path()).expect("open workspace");

    workspace
        .save_environment(
            "dev",
            &[EnvChange::Set {
                layer: EnvLayer::Base,
                key: "B".to_string(),
                value: "2".to_string(),
            }],
        )
        .expect("save environment");

    let content =
        fs::read_to_string(temp.path().join("environments/dev.env")).expect("read env file");
    assert_eq!(content, "A=1\nB=2\n");
}

#[test]
fn create_environment_creates_an_empty_env_file() {
    let temp = TempDir::new().expect("temp dir");
    let workspace = Workspace::open(temp.path()).expect("open workspace");

    workspace
        .create_environment("staging")
        .expect("create environment");

    assert_eq!(
        workspace.list_environments().expect("list environments"),
        vec!["staging".to_string()]
    );
}

#[test]
fn create_environment_fails_if_it_already_exists() {
    let temp = TempDir::new().expect("temp dir");
    let workspace = Workspace::open(temp.path()).expect("open workspace");
    workspace
        .create_environment("staging")
        .expect("create environment");

    assert!(matches!(
        workspace.create_environment("staging"),
        Err(WorkspaceError::AlreadyExists(_))
    ));
}

// --- Path traversal rejection --------------------------------------------------------------

#[test]
fn load_request_rejects_a_traversal_id() {
    let temp = TempDir::new().expect("temp dir");
    let workspace = Workspace::open(temp.path()).expect("open workspace");
    assert!(matches!(
        workspace.load_request("../secret.brasa"),
        Err(WorkspaceError::InvalidId(_))
    ));
}

#[test]
fn save_request_rejects_a_traversal_id() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    let request = brasa_core::Request::default();
    assert!(matches!(
        workspace.save_request("../escape.brasa", &request),
        Err(WorkspaceError::InvalidId(_))
    ));
}

#[test]
fn save_request_rejects_an_absolute_id() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    let request = brasa_core::Request::default();
    assert!(matches!(
        workspace.save_request("/etc/passwd", &request),
        Err(WorkspaceError::InvalidId(_))
    ));
}

#[test]
fn create_request_rejects_a_traversal_parent_id() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    assert!(matches!(
        workspace.create_request(Some(".."), "evil"),
        Err(WorkspaceError::InvalidId(_))
    ));
}

#[test]
fn create_folder_rejects_a_traversal_parent_id() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    assert!(matches!(
        workspace.create_folder(Some("../../"), "evil"),
        Err(WorkspaceError::InvalidId(_))
    ));
}

#[test]
fn rename_rejects_a_traversal_id() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    assert!(matches!(
        workspace.rename("../outside.brasa", "new-name"),
        Err(WorkspaceError::InvalidId(_))
    ));
}

#[test]
fn delete_rejects_a_traversal_id() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    assert!(matches!(
        workspace.delete("../outside"),
        Err(WorkspaceError::InvalidId(_))
    ));
}

#[test]
fn a_dot_dot_new_name_cannot_be_used_to_escape_on_rename() {
    let temp = TempDir::new().expect("temp dir");
    write_file(&temp.path().join("a.brasa"), "GET https://example.com\n");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    // Sanitization strips the trailing dots, so this becomes a harmless "untitled.brasa"
    // instead of escaping the workspace root.
    let new_id = workspace
        .rename("a.brasa", "..")
        .expect("rename should sanitize, not fail");
    assert_eq!(new_id, "untitled.brasa");
    assert!(temp.path().join("untitled.brasa").exists());
}

fn env_dir(temp: &TempDir) -> std::path::PathBuf {
    temp.path().join("environments")
}

#[test]
fn load_environment_layers_returns_each_file_apart() {
    let temp = TempDir::new().expect("temp dir");
    write_file(&env_dir(&temp).join("dev.env"), "A=1\nB=2\n");
    write_file(&env_dir(&temp).join("dev.local.env"), "B=3\n");
    let workspace = Workspace::open(temp.path()).expect("open");

    let layers = workspace.load_environment_layers("dev").expect("layers");

    assert_eq!(layers.base.expect("base").len(), 2);
    assert_eq!(layers.local.expect("local").len(), 1);
    assert!(matches!(
        workspace.load_environment_layers("nope"),
        Err(WorkspaceError::EnvironmentNotFound(_))
    ));
}

#[test]
fn save_environment_preserves_comments_blank_lines_and_order() {
    let temp = TempDir::new().expect("temp dir");
    write_file(
        &env_dir(&temp).join("dev.env"),
        "# api\nURL=http://x\n\n# creds\nUSER=me\nKEEP=1\n",
    );
    write_file(
        &env_dir(&temp).join("dev.local.env"),
        "# secrets\nPASS=old\n",
    );
    let workspace = Workspace::open(temp.path()).expect("open");

    workspace
        .save_environment(
            "dev",
            &[
                EnvChange::Set {
                    layer: EnvLayer::Base,
                    key: "URL".into(),
                    value: "http://y".into(),
                },
                EnvChange::Remove {
                    layer: EnvLayer::Base,
                    key: "USER".into(),
                },
                EnvChange::Set {
                    layer: EnvLayer::Base,
                    key: "NEW".into(),
                    value: "n".into(),
                },
                EnvChange::Rename {
                    layer: EnvLayer::Local,
                    from: "PASS".into(),
                    to: "PASSWORD".into(),
                },
                EnvChange::Set {
                    layer: EnvLayer::Local,
                    key: "PASSWORD".into(),
                    value: "new".into(),
                },
            ],
        )
        .expect("save");

    assert_eq!(
        fs::read_to_string(env_dir(&temp).join("dev.env")).unwrap(),
        "# api\nURL=http://y\n\n# creds\nKEEP=1\nNEW=n\n"
    );
    assert_eq!(
        fs::read_to_string(env_dir(&temp).join("dev.local.env")).unwrap(),
        "# secrets\nPASSWORD=new\n"
    );
}

#[test]
fn save_environment_swaps_two_names_and_creates_a_missing_local_file() {
    let temp = TempDir::new().expect("temp dir");
    write_file(&env_dir(&temp).join("dev.env"), "A=1\nB=2\n");
    let workspace = Workspace::open(temp.path()).expect("open");

    workspace
        .save_environment(
            "dev",
            &[
                EnvChange::Rename {
                    layer: EnvLayer::Base,
                    from: "A".into(),
                    to: "B".into(),
                },
                EnvChange::Rename {
                    layer: EnvLayer::Base,
                    from: "B".into(),
                    to: "A".into(),
                },
                EnvChange::Set {
                    layer: EnvLayer::Local,
                    key: "S".into(),
                    value: "x".into(),
                },
            ],
        )
        .expect("save");

    assert_eq!(
        fs::read_to_string(env_dir(&temp).join("dev.env")).unwrap(),
        "B=1\nA=2\n"
    );
    assert_eq!(
        fs::read_to_string(env_dir(&temp).join("dev.local.env")).unwrap(),
        "S=x\n"
    );
}

#[test]
fn save_environment_without_changes_for_a_file_does_not_create_it() {
    let temp = TempDir::new().expect("temp dir");
    write_file(&env_dir(&temp).join("dev.env"), "A=1\n");
    let workspace = Workspace::open(temp.path()).expect("open");

    workspace
        .save_environment(
            "dev",
            &[EnvChange::Remove {
                layer: EnvLayer::Base,
                key: "A".into(),
            }],
        )
        .expect("save");

    assert_eq!(
        fs::read_to_string(env_dir(&temp).join("dev.env")).unwrap(),
        ""
    );
    assert!(!env_dir(&temp).join("dev.local.env").exists());
}

#[test]
fn rename_environment_moves_both_files() {
    let temp = TempDir::new().expect("temp dir");
    write_file(&env_dir(&temp).join("dev.env"), "A=1\n");
    write_file(&env_dir(&temp).join("dev.local.env"), "B=2\n");
    write_file(&env_dir(&temp).join("taken.env"), "");
    let workspace = Workspace::open(temp.path()).expect("open");

    assert!(matches!(
        workspace.rename_environment("dev", "taken"),
        Err(WorkspaceError::AlreadyExists(_))
    ));
    assert!(matches!(
        workspace.rename_environment("dev", "a/b"),
        Err(WorkspaceError::InvalidId(_))
    ));
    assert!(matches!(
        workspace.rename_environment("dev", "qa.LOCAL"),
        Err(WorkspaceError::InvalidId(_))
    ));
    workspace.rename_environment("dev", "qa").expect("rename");

    assert!(!env_dir(&temp).join("dev.env").exists());
    assert!(!env_dir(&temp).join("dev.local.env").exists());
    assert_eq!(
        fs::read_to_string(env_dir(&temp).join("qa.env")).unwrap(),
        "A=1\n"
    );
    assert_eq!(
        fs::read_to_string(env_dir(&temp).join("qa.local.env")).unwrap(),
        "B=2\n"
    );
}

#[test]
fn delete_environment_removes_both_files() {
    let temp = TempDir::new().expect("temp dir");
    write_file(&env_dir(&temp).join("dev.env"), "A=1\n");
    write_file(&env_dir(&temp).join("dev.local.env"), "B=2\n");
    let workspace = Workspace::open(temp.path()).expect("open");

    workspace.delete_environment("dev").expect("delete");

    assert!(!env_dir(&temp).join("dev.env").exists());
    assert!(!env_dir(&temp).join("dev.local.env").exists());
    assert!(matches!(
        workspace.delete_environment("dev"),
        Err(WorkspaceError::EnvironmentNotFound(_))
    ));
}
