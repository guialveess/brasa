//! Editing an environment as two separate layers (`<name>.env` and `<name>.local.env`): loading
//! them apart, saving a list of changes that preserves comments, blank lines and the order of
//! every untouched line, renaming and deleting. [`Workspace::load_environment`] stays the way to
//! read the merged result for resolving variables.

use std::fs;
use std::path::PathBuf;

use brasa_core::KeyValue;
use brasa_format::env::{self, EnvLine};
use brasa_format::project::ProjectConfig;

use crate::ENVIRONMENTS_FOLDER;
use crate::error::WorkspaceError;
use crate::ids::validate_relative_path;
use crate::project::ProjectWrite;
use crate::sanitize::sanitize_file_name;
use crate::workspace::{Workspace, atomic_write};

/// Which file of an environment a variable is stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvLayer {
    /// `environments/<name>.env`, versioned.
    Base,
    /// `environments/<name>.local.env`, gitignored, overrides the base file.
    Local,
}

/// The two files of an environment, each parsed on its own. `None` when the file does not exist.
/// Variables are in file order and duplicates are kept as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvLayers {
    /// The variables of `<name>.env`.
    pub base: Option<Vec<KeyValue>>,
    /// The variables of `<name>.local.env`.
    pub local: Option<Vec<KeyValue>>,
}

/// One edit to apply to an environment's files with [`Workspace::save_environment`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvChange {
    /// Sets `key` to `value` in `layer`: replaces the first line for `key` in place, or appends
    /// a new line (creating the file if needed).
    Set {
        /// The file to write to.
        layer: EnvLayer,
        /// The variable name.
        key: String,
        /// The new value.
        value: String,
    },
    /// Removes the first line for `key` from `layer`.
    Remove {
        /// The file to edit.
        layer: EnvLayer,
        /// The variable name.
        key: String,
    },
    /// Renames the first line for `from` to `to` in `layer`, keeping its position and value.
    Rename {
        /// The file to edit.
        layer: EnvLayer,
        /// The current name.
        from: String,
        /// The new name.
        to: String,
    },
}

impl EnvChange {
    fn layer(&self) -> EnvLayer {
        match self {
            EnvChange::Set { layer, .. }
            | EnvChange::Remove { layer, .. }
            | EnvChange::Rename { layer, .. } => *layer,
        }
    }
}

impl Workspace {
    fn env_path(&self, name: &str, layer: EnvLayer) -> PathBuf {
        let file_name = match layer {
            EnvLayer::Base => format!("{name}.env"),
            EnvLayer::Local => format!("{name}.local.env"),
        };
        self.root().join(ENVIRONMENTS_FOLDER).join(file_name)
    }

    /// The path of the file of `name` holding `layer`, whether or not it exists yet.
    pub fn environment_path(&self, name: &str, layer: EnvLayer) -> PathBuf {
        self.env_path(name, layer)
    }

    /// Loads both files of the environment `name` apart. Fails with
    /// [`WorkspaceError::EnvironmentNotFound`] when neither exists.
    pub fn load_environment_layers(&self, name: &str) -> Result<EnvLayers, WorkspaceError> {
        validate_relative_path(name)
            .map_err(|_| WorkspaceError::EnvironmentNotFound(name.to_string()))?;
        let read = |layer| -> Result<Option<Vec<KeyValue>>, WorkspaceError> {
            let path = self.env_path(name, layer);
            if !path.is_file() {
                return Ok(None);
            }
            let text = fs::read_to_string(&path).map_err(|source| WorkspaceError::Io {
                path: path.clone(),
                source,
            })?;
            let variables = env::parse(&text).map_err(|source| WorkspaceError::EnvParse {
                name: name.to_string(),
                source,
            })?;
            Ok(Some(variables))
        };
        let layers = EnvLayers {
            base: read(EnvLayer::Base)?,
            local: read(EnvLayer::Local)?,
        };
        if layers.base.is_none() && layers.local.is_none() {
            return Err(WorkspaceError::EnvironmentNotFound(name.to_string()));
        }
        Ok(layers)
    }

    /// Applies `changes` to the files of the environment `name`, rewriting only the lines they
    /// touch: comments, blank lines and the order of every other line are preserved. Within a
    /// file the order of application is: every `Remove`, then every `Rename` (two phases, so a
    /// swap of two names works), then every `Set` in the given order. A file with no change is
    /// not touched, and a missing file is only created by a `Set`. Both files are parsed before
    /// either is written, so a parse error leaves them as they were.
    pub fn save_environment(
        &self,
        name: &str,
        changes: &[EnvChange],
    ) -> Result<(), WorkspaceError> {
        validate_relative_path(name)?;
        let mut pending = Vec::new();
        for layer in [EnvLayer::Base, EnvLayer::Local] {
            let layer_changes: Vec<&EnvChange> = changes
                .iter()
                .filter(|change| change.layer() == layer)
                .collect();
            if layer_changes.is_empty() {
                continue;
            }
            let path = self.env_path(name, layer);
            let existing = if path.is_file() {
                fs::read_to_string(&path).map_err(|source| WorkspaceError::Io {
                    path: path.clone(),
                    source,
                })?
            } else {
                String::new()
            };
            let mut lines =
                env::parse_lines(&existing).map_err(|source| WorkspaceError::EnvParse {
                    name: name.to_string(),
                    source,
                })?;
            apply_changes(&mut lines, &layer_changes);
            pending.push((path, env::serialize_lines(&lines)));
        }
        if pending.is_empty() {
            return Ok(());
        }
        let dir = self.root().join(ENVIRONMENTS_FOLDER);
        fs::create_dir_all(&dir).map_err(|source| WorkspaceError::Io {
            path: dir.clone(),
            source,
        })?;
        for (path, text) in pending {
            atomic_write(&path, &text)?;
        }
        Ok(())
    }

    /// Renames the environment `old` to `new`: both its files. Fails with
    /// [`WorkspaceError::AlreadyExists`] if `new` already has either file, and with
    /// [`WorkspaceError::InvalidId`] if `new` is not a plain file name.
    pub fn rename_environment(&self, old: &str, new: &str) -> Result<(), WorkspaceError> {
        validate_relative_path(old)?;
        if crate::workspace::strip_local_suffix(new).is_some() || sanitize_file_name(new) != new {
            return Err(WorkspaceError::InvalidId(new.to_string()));
        }
        let moves: Vec<(PathBuf, PathBuf)> = [EnvLayer::Base, EnvLayer::Local]
            .into_iter()
            .map(|layer| (self.env_path(old, layer), self.env_path(new, layer)))
            .filter(|(from, _)| from.is_file())
            .collect();
        if moves.is_empty() {
            return Err(WorkspaceError::EnvironmentNotFound(old.to_string()));
        }
        for layer in [EnvLayer::Base, EnvLayer::Local] {
            if self.env_path(new, layer).exists() {
                return Err(WorkspaceError::AlreadyExists(new.to_string()));
            }
        }
        for (from, to) in moves {
            fs::rename(&from, &to).map_err(|source| WorkspaceError::Io { path: from, source })?;
        }
        self.follow_in_project(|config| config.rename_environment(old, new));
        Ok(())
    }

    /// Deletes both files of the environment `name`. Fails with
    /// [`WorkspaceError::EnvironmentNotFound`] when neither exists.
    pub fn delete_environment(&self, name: &str) -> Result<(), WorkspaceError> {
        validate_relative_path(name)?;
        let mut deleted = false;
        for layer in [EnvLayer::Base, EnvLayer::Local] {
            let path = self.env_path(name, layer);
            if path.is_file() {
                fs::remove_file(&path).map_err(|source| WorkspaceError::Io { path, source })?;
                deleted = true;
            }
        }
        if deleted {
            self.follow_in_project(|config| config.remove_environment(name));
            Ok(())
        } else {
            Err(WorkspaceError::EnvironmentNotFound(name.to_string()))
        }
    }
}

impl Workspace {
    /// Keeps `.brasa/project.toml`, when it exists, in step with a rename or a delete that
    /// already happened. A file that cannot be updated is only logged: the environment change
    /// itself succeeded.
    fn follow_in_project(&self, edit: impl FnOnce(&mut ProjectConfig)) {
        if let Err(error) = self.update_project(ProjectWrite::IfExists, edit) {
            log::warn!("cannot update the workspace configuration: {error}");
        }
    }
}

/// Applies one file's changes to its lines, see [`Workspace::save_environment`] for the order.
fn apply_changes(lines: &mut Vec<EnvLine>, changes: &[&EnvChange]) {
    for change in changes {
        if let EnvChange::Remove { key, .. } = change {
            env::remove_variable(lines, key);
        }
    }
    let renames: Vec<(&str, &str)> = changes
        .iter()
        .filter_map(|change| match change {
            EnvChange::Rename { from, to, .. } => Some((from.as_str(), to.as_str())),
            _ => None,
        })
        .collect();
    // Two phases through unique temporary names, so renaming `a` to `b` while `b` is renamed to
    // `a` (or to anything else) never collides with a line that still has the target name.
    for (index, (from, _)) in renames.iter().enumerate() {
        env::rename_variable(lines, from, &format!("\u{0}rename-{index}"));
    }
    for (index, (_, to)) in renames.iter().enumerate() {
        env::rename_variable(lines, &format!("\u{0}rename-{index}"), to);
    }
    for change in changes {
        if let EnvChange::Set { key, value, .. } = change {
            env::set_variable(lines, key, value);
        }
    }
}
