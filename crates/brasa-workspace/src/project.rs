//! Reading and writing the workspace configuration file, `.brasa/project.toml` (#215). The file
//! is optional: a workspace without it works, and it is only written when the user changes
//! something it holds. Every update reads the file again first, so a change made by hand or by a
//! pull since the last read is kept.

use std::fs;
use std::path::{Path, PathBuf};

use brasa_format::project::{PROJECT_FILE, ProjectConfig};

use crate::error::WorkspaceError;
use crate::workspace::{Workspace, atomic_write};

/// Whether [`Workspace::update_project`] may create the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectWrite {
    /// Write it, creating `.brasa/project.toml` when it does not exist: the user changed
    /// something only the file can hold (an environment's order or color).
    Create,
    /// Only update a file that exists: following a rename or a delete never creates one.
    IfExists,
}

/// The path of the file inside the workspace `root`.
pub(crate) fn project_path(root: &Path) -> PathBuf {
    PROJECT_FILE
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// Writes a new file with only the format version, for [`crate::create_workspace`].
pub(crate) fn write_new_project(root: &Path) -> Result<(), WorkspaceError> {
    write(&project_path(root), &ProjectConfig::default())
}

fn write(path: &Path, config: &ProjectConfig) -> Result<(), WorkspaceError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|source| WorkspaceError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    atomic_write(path, &config.to_toml())
}

/// Reorders `names` (already in natural order): the ones in `order` first, in its order, then
/// the rest in their current order. Names in `order` that are not in `names` are ignored, and so
/// is a name repeated in `order`.
pub(crate) fn apply_order(names: Vec<String>, order: &[String]) -> Vec<String> {
    let mut sorted: Vec<String> = Vec::with_capacity(names.len());
    for name in order {
        if names.contains(name) && !sorted.contains(name) {
            sorted.push(name.clone());
        }
    }
    sorted.extend(names.into_iter().filter(|name| !order.contains(name)));
    sorted
}

impl Workspace {
    /// Saves `names` as the order of the environments in `.brasa/project.toml`, creating the
    /// file when it does not exist. [`Workspace::list_environments`] returns them in this order
    /// from now on, with any environment not in `names` after them.
    pub fn set_environment_order(&self, names: &[String]) -> Result<(), WorkspaceError> {
        self.update_project(ProjectWrite::Create, |config| {
            config.set_environment_order(names);
        })
    }

    /// The workspace configuration. A missing file is the default configuration, never an error;
    /// a file that is not valid TOML is [`WorkspaceError::Project`].
    pub fn load_project(&self) -> Result<ProjectConfig, WorkspaceError> {
        Ok(self.read_project()?.unwrap_or_default())
    }

    /// Applies `edit` to the configuration on disk and saves it. With [`ProjectWrite::IfExists`]
    /// a missing file stays missing. A file that is not valid TOML is never overwritten: the
    /// error is returned and nothing is written.
    pub fn update_project(
        &self,
        write_mode: ProjectWrite,
        edit: impl FnOnce(&mut ProjectConfig),
    ) -> Result<(), WorkspaceError> {
        let mut config = match (self.read_project()?, write_mode) {
            (Some(config), _) => config,
            (None, ProjectWrite::Create) => ProjectConfig::default(),
            (None, ProjectWrite::IfExists) => return Ok(()),
        };
        let before = config.clone();
        edit(&mut config);
        if config == before && self.project_path().is_file() {
            return Ok(());
        }
        log::debug!("saving {PROJECT_FILE}");
        write(&self.project_path(), &config)
    }

    /// The configuration file's path.
    pub fn project_path(&self) -> PathBuf {
        project_path(self.root())
    }

    /// The parsed file, `None` when it does not exist.
    fn read_project(&self) -> Result<Option<ProjectConfig>, WorkspaceError> {
        let path = self.project_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(WorkspaceError::Io { path, source }),
        };
        Ok(Some(ProjectConfig::parse(&text)?))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = Workspace::open(dir.path()).expect("open");
        (dir, workspace)
    }

    #[test]
    fn a_workspace_without_the_file_works_and_stays_without_it() {
        let (_dir, workspace) = workspace();
        assert_eq!(
            workspace.load_project().expect("load"),
            ProjectConfig::default()
        );
        workspace
            .update_project(ProjectWrite::IfExists, |config| {
                config.rename_environment("dev", "development");
            })
            .expect("update");
        assert!(!workspace.project_path().exists());
    }

    #[test]
    fn create_writes_the_file_and_its_folder() {
        let (_dir, workspace) = workspace();
        workspace
            .update_project(ProjectWrite::Create, |config| {
                config.set_environment_color("dev", Some("blue"));
            })
            .expect("update");
        let config = workspace.load_project().expect("load");
        assert_eq!(config.environment_color("dev"), Some("blue"));
    }

    #[test]
    fn an_update_keeps_what_changed_on_disk_meanwhile() {
        let (_dir, workspace) = workspace();
        workspace
            .update_project(ProjectWrite::Create, |config| {
                config.set_environment_color("dev", Some("blue"));
            })
            .expect("update");
        // Someone edits the file by hand.
        fs::write(
            workspace.project_path(),
            "version = 1\n[environments.colors]\ndev = \"blue\"\nprod = \"red\"\n",
        )
        .expect("write");
        workspace
            .update_project(ProjectWrite::IfExists, |config| {
                config.set_environment_color("qa", Some("amber"));
            })
            .expect("update");
        let config = workspace.load_project().expect("load");
        assert_eq!(config.environment_color("prod"), Some("red"));
        assert_eq!(config.environment_color("qa"), Some("amber"));
    }

    #[test]
    fn renaming_and_deleting_an_environment_follow_in_the_file() {
        let (_dir, workspace) = workspace();
        workspace.create_environment("dev").expect("create dev");
        workspace.create_environment("prod").expect("create prod");
        workspace
            .update_project(ProjectWrite::Create, |config| {
                config.set_environment_order(&["prod".to_string(), "dev".to_string()]);
                config.set_environment_color("dev", Some("blue"));
            })
            .expect("update");

        workspace
            .rename_environment("dev", "development")
            .expect("rename");
        let config = workspace.load_project().expect("load");
        assert_eq!(config.environment_order(), vec!["prod", "development"]);
        assert_eq!(config.environment_color("development"), Some("blue"));

        workspace.delete_environment("development").expect("delete");
        let config = workspace.load_project().expect("load");
        assert_eq!(config.environment_order(), vec!["prod"]);
        assert_eq!(config.environment_color("development"), None);
    }

    #[test]
    fn a_broken_file_does_not_stop_a_rename() {
        let (_dir, workspace) = workspace();
        workspace.create_environment("dev").expect("create");
        fs::create_dir_all(workspace.project_path().parent().expect("parent")).expect("mkdir");
        fs::write(workspace.project_path(), "[environments\n").expect("write");
        workspace.rename_environment("dev", "qa").expect("rename");
        assert_eq!(workspace.list_environments().expect("list"), vec!["qa"]);
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn apply_order_puts_listed_names_first_and_keeps_the_rest_in_order() {
        let names = strings(&["dev", "local", "prod", "staging"]);
        assert_eq!(
            apply_order(names, &strings(&["local", "gone", "prod", "local"])),
            strings(&["local", "prod", "dev", "staging"])
        );
        let names = strings(&["dev", "local"]);
        assert_eq!(apply_order(names.clone(), &[]), names);
    }

    #[test]
    fn the_environment_list_follows_the_saved_order() {
        let (_dir, workspace) = workspace();
        for name in ["dev", "local", "prod", "staging"] {
            workspace.create_environment(name).expect("create");
        }
        assert_eq!(
            workspace.list_environments().expect("list"),
            strings(&["dev", "local", "prod", "staging"])
        );
        assert!(!workspace.project_path().exists());

        workspace
            .set_environment_order(&strings(&["local", "dev", "staging", "prod"]))
            .expect("order");
        workspace.create_environment("qa").expect("create qa");
        assert_eq!(
            workspace.list_environments().expect("list"),
            strings(&["local", "dev", "staging", "prod", "qa"])
        );
    }

    #[test]
    fn a_broken_file_leaves_the_natural_order() {
        let (_dir, workspace) = workspace();
        workspace.create_environment("prod").expect("create");
        workspace.create_environment("dev").expect("create");
        fs::create_dir_all(workspace.project_path().parent().expect("parent")).expect("mkdir");
        fs::write(workspace.project_path(), "[environments\n").expect("write");
        assert_eq!(
            workspace.list_environments().expect("list"),
            strings(&["dev", "prod"])
        );
    }

    #[test]
    fn a_broken_file_is_reported_and_never_overwritten() {
        let (_dir, workspace) = workspace();
        fs::create_dir_all(workspace.project_path().parent().expect("parent")).expect("mkdir");
        fs::write(workspace.project_path(), "[environments\n").expect("write");

        assert!(matches!(
            workspace.load_project(),
            Err(WorkspaceError::Project(_))
        ));
        let result = workspace.update_project(ProjectWrite::Create, |config| {
            config.set_environment_color("dev", Some("blue"));
        });
        assert!(matches!(result, Err(WorkspaceError::Project(_))));
        assert_eq!(
            fs::read_to_string(workspace.project_path()).expect("read"),
            "[environments\n"
        );
    }
}
