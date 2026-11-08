//! Creating a new workspace folder, the counterpart of [`crate::Workspace::open`] for a folder
//! that does not exist yet.

use std::fs;
use std::path::Path;

use brasa_core::{KeyValue, Method, Request};

use crate::ENVIRONMENTS_FOLDER;
use crate::error::WorkspaceError;
use crate::workspace::atomic_write;

/// The id of the example request [`NewWorkspaceContent::Example`] writes.
pub const EXAMPLE_REQUEST_ID: &str = "hello.brasa";

/// The name of the environment [`NewWorkspaceContent::Example`] writes.
pub const EXAMPLE_ENVIRONMENT: &str = "local";

/// The URL the example environment points `baseUrl` to. The example request asks it for
/// `/hello.json`, a static file of the project website (`site/hello.json`) whose shape must not
/// change, since the example's test reads its `app` field.
const EXAMPLE_BASE_URL: &str = "https://brasa.tanis.codes";

/// The post script of the example request.
const EXAMPLE_POST_SCRIPT: &str = r#"test("status is 200", () => {
  expect(res.status).toBe(200);
});

test("the server says hello", () => {
  expect(res.json().app).toBe("Brasa");
});"#;

/// The `.gitignore` of every new workspace: the files that are personal or generated on each
/// machine.
pub const GITIGNORE: &str = "\
# Secrets and personal overrides of environments
*.local.env

# Load test run history
.brasa/runs/

# Report of the last import
.brasa/import-report.md
";

/// What [`create_workspace`] puts in a new workspace folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewWorkspaceContent {
    /// The example: `environments/local.env` (see [`EXAMPLE_ENVIRONMENT`]) and `hello.brasa`
    /// (see [`EXAMPLE_REQUEST_ID`]). The first Send works without editing anything.
    Example,
    /// No requests or environments, for a folder that is about to receive an import.
    Empty,
}

/// Creates `root` (and its missing parents) as a new workspace folder holding `content`, a
/// `.gitignore` (see [`GITIGNORE`]) and the workspace configuration file,
/// `.brasa/project.toml`.
///
/// `root` may be missing or an empty directory (hidden files count as content). Fails without
/// writing anything with [`WorkspaceError::NotEmpty`] when `root` is a directory with anything
/// inside, and with [`WorkspaceError::AlreadyExists`] when it is a file. Any other failure is a
/// [`WorkspaceError::Io`]; a failure while writing the content can leave the folder partly
/// filled. Open the result with [`crate::Workspace::open`].
pub fn create_workspace(root: &Path, content: NewWorkspaceContent) -> Result<(), WorkspaceError> {
    if root.exists() {
        if !root.is_dir() {
            return Err(WorkspaceError::AlreadyExists(root.display().to_string()));
        }
        let mut entries = fs::read_dir(root).map_err(|source| WorkspaceError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        if entries.next().is_some() {
            return Err(WorkspaceError::NotEmpty(root.to_path_buf()));
        }
    } else {
        fs::create_dir_all(root).map_err(|source| WorkspaceError::Io {
            path: root.to_path_buf(),
            source,
        })?;
    }

    if content == NewWorkspaceContent::Example {
        write_example(root)?;
    }
    crate::project::write_new_project(root)?;
    atomic_write(&root.join(".gitignore"), GITIGNORE)
}

/// Writes the files of [`NewWorkspaceContent::Example`] into the existing, empty folder `root`.
fn write_example(root: &Path) -> Result<(), WorkspaceError> {
    let environments = root.join(ENVIRONMENTS_FOLDER);
    fs::create_dir(&environments).map_err(|source| WorkspaceError::Io {
        path: environments.clone(),
        source,
    })?;
    let variables = [KeyValue::new("baseUrl", EXAMPLE_BASE_URL)];
    atomic_write(
        &environments.join(format!("{EXAMPLE_ENVIRONMENT}.env")),
        &brasa_format::env::serialize(&variables),
    )?;

    let request = Request {
        method: Method::Get,
        url: "{{baseUrl}}/hello.json".to_string(),
        post_script: EXAMPLE_POST_SCRIPT.to_string(),
        ..Request::default()
    };
    atomic_write(
        &root.join(EXAMPLE_REQUEST_ID),
        &brasa_format::serialize(&request),
    )
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{Node, Workspace};
    use pretty_assertions::assert_eq;

    /// The names of the entries directly inside `dir`, sorted.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read dir")
            .map(|entry| {
                entry
                    .expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn creates_a_missing_folder_with_its_parents() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("a/b/my-api");

        create_workspace(&root, NewWorkspaceContent::Example).expect("create");

        assert!(root.join("hello.brasa").is_file());
    }

    #[test]
    fn fills_an_existing_empty_folder() {
        let temp = tempfile::tempdir().expect("temp dir");

        create_workspace(temp.path(), NewWorkspaceContent::Example).expect("create");

        assert_eq!(
            names_in(temp.path()),
            vec![".brasa", ".gitignore", "environments", "hello.brasa"]
        );
    }

    #[test]
    fn refuses_a_folder_with_content_and_writes_nothing() {
        let temp = tempfile::tempdir().expect("temp dir");
        fs::write(temp.path().join("notes.txt"), "keep me").expect("write file");

        let result = create_workspace(temp.path(), NewWorkspaceContent::Example);

        assert!(matches!(result, Err(WorkspaceError::NotEmpty(path)) if path == temp.path()));
        assert_eq!(names_in(temp.path()), vec!["notes.txt"]);
    }

    #[test]
    fn a_hidden_file_counts_as_content() {
        let temp = tempfile::tempdir().expect("temp dir");
        fs::write(temp.path().join(".DS_Store"), "").expect("write file");

        let result = create_workspace(temp.path(), NewWorkspaceContent::Empty);

        assert!(matches!(result, Err(WorkspaceError::NotEmpty(_))));
        assert_eq!(names_in(temp.path()), vec![".DS_Store"]);
    }

    #[test]
    fn refuses_a_path_that_is_a_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let file = temp.path().join("my-api");
        fs::write(&file, "not a folder").expect("write file");

        let result = create_workspace(&file, NewWorkspaceContent::Example);

        assert!(matches!(result, Err(WorkspaceError::AlreadyExists(_))));
        assert_eq!(
            fs::read_to_string(&file).expect("read file"),
            "not a folder"
        );
    }

    #[test]
    fn empty_content_creates_the_folder_with_only_the_gitignore_and_the_project_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("imported");

        create_workspace(&root, NewWorkspaceContent::Empty).expect("create");

        assert!(root.is_dir());
        assert_eq!(names_in(&root), vec![".brasa", ".gitignore"]);
        assert_eq!(
            Workspace::open(&root)
                .expect("open")
                .load_project()
                .expect("project"),
            brasa_format::project::ProjectConfig::default()
        );
        assert_eq!(
            fs::read_to_string(root.join(".gitignore")).expect("read .gitignore"),
            GITIGNORE
        );
        assert!(Workspace::open(&root).expect("open").tree().is_empty());
    }

    #[test]
    fn the_example_opens_as_a_working_workspace() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("my-api");
        create_workspace(&root, NewWorkspaceContent::Example).expect("create");

        let workspace = Workspace::open(&root).expect("open");

        let [Node::Request(entry)] = workspace.tree() else {
            panic!(
                "expected only hello.brasa in the tree: {:?}",
                workspace.tree()
            );
        };
        assert_eq!(entry.id, EXAMPLE_REQUEST_ID);
        assert_eq!(entry.broken, None);

        assert_eq!(
            workspace.list_environments().expect("list"),
            vec![EXAMPLE_ENVIRONMENT]
        );
        let environment = workspace
            .load_environment(EXAMPLE_ENVIRONMENT)
            .expect("load environment");
        assert_eq!(
            environment.variables,
            vec![KeyValue::new("baseUrl", "https://brasa.tanis.codes")]
        );

        let request = workspace
            .load_request(EXAMPLE_REQUEST_ID)
            .expect("load request");
        assert_eq!(request.method, Method::Get);
        assert_eq!(request.url, "{{baseUrl}}/hello.json");
        assert_eq!(request.post_script, EXAMPLE_POST_SCRIPT);
        assert_eq!(
            request,
            Request {
                method: Method::Get,
                url: "{{baseUrl}}/hello.json".to_string(),
                post_script: EXAMPLE_POST_SCRIPT.to_string(),
                ..Request::default()
            }
        );

        assert_eq!(
            fs::read_to_string(root.join(".gitignore")).expect("read .gitignore"),
            GITIGNORE
        );
    }
}
