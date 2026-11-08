//! The file work every import shares (see `docs/postman-import.md`): picking free names, writing
//! environments and undoing a failed import. Nothing is ever overwritten.

use std::fs;
use std::path::{Path, PathBuf};

use brasa_core::KeyValue;

use crate::ENVIRONMENTS_FOLDER;
use crate::error::WorkspaceError;
use crate::ids::path_to_id;
use crate::sanitize::sanitize_file_name;
use crate::workspace::atomic_write;

/// The result of an import: every file that was written, and every warning collected while
/// mapping the source JSON.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportReport {
    /// The workspace ids of the files written by the import (requests and environment files).
    /// The new collection folder itself is not listed here, only the files inside it.
    pub created_files: Vec<String>,
    /// Everything that could not be mapped faithfully, carried over from the parser as is.
    pub warnings: Vec<String>,
}

/// Removes what a failed import wrote: its new collection folder `target` and the `created` files
/// outside of it, the environments. Best effort, the import's own error is the one to report.
pub(crate) fn undo_import(root: &Path, target: &Path, created: &[String]) {
    if let Err(error) = fs::remove_dir_all(target)
        && target.exists()
    {
        log::warn!("could not undo the import in {}: {error}", target.display());
    }
    for id in created {
        let path = root.join(id);
        if path.exists()
            && let Err(error) = fs::remove_file(&path)
        {
            log::warn!("could not undo the import of {id}: {error}");
        }
    }
}

/// Finds a filesystem path inside `dir` for `sanitized_name` that does not exist yet, appending
/// ` (2)`, ` (3)`, ... to the name until one is free. Used for folders, which have no extension
/// (including the new top-level collection folder itself).
pub(crate) fn unique_path(dir: &Path, sanitized_name: &str) -> PathBuf {
    let mut candidate = dir.join(sanitized_name);
    let mut counter = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{sanitized_name} ({counter})"));
        counter += 1;
    }
    candidate
}

/// Like [`unique_path`], but for a file with `extension`: the numeric suffix is inserted before
/// the extension (`"login (2).brasa"`), not after it.
pub(crate) fn unique_named_path(dir: &Path, sanitized_name: &str, extension: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{sanitized_name}.{extension}"));
    let mut counter = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{sanitized_name} ({counter}).{extension}"));
        counter += 1;
    }
    candidate
}

/// Writes `base` to `environments/<name>.env` and, if `secret` is non-empty, `secret` to
/// `environments/<name>.local.env`.
///
/// `name` is sanitized first. If a file with that name already exists (checking both
/// extensions, so the base and secret files of one environment always share the same name), a
/// numeric suffix is appended, the same way [`unique_path`] does for folders and requests: this
/// crate chooses to suffix rather than fail, consistent with how every other import name clash
/// is handled, so one import never aborts partway through because of a stray existing file.
/// Writing nothing (`base` and `secret` both empty) is a no-op, the `environments/` folder is
/// not even created in that case.
pub(crate) fn write_environment(
    root: &Path,
    name: &str,
    base: &[KeyValue],
    secret: &[KeyValue],
    created: &mut Vec<String>,
) -> Result<(), WorkspaceError> {
    if base.is_empty() && secret.is_empty() {
        return Ok(());
    }
    let dir = root.join(ENVIRONMENTS_FOLDER);
    fs::create_dir_all(&dir).map_err(|source| WorkspaceError::Io {
        path: dir.clone(),
        source,
    })?;

    let sanitized = sanitize_file_name(name);
    let unique_name = unique_environment_name(&dir, &sanitized);

    if !base.is_empty() {
        let path = dir.join(format!("{unique_name}.env"));
        atomic_write(&path, &brasa_format::env::serialize(base))?;
        created.push(path_to_id(root, &path));
    }
    if !secret.is_empty() {
        let path = dir.join(format!("{unique_name}.local.env"));
        atomic_write(&path, &brasa_format::env::serialize(secret))?;
        created.push(path_to_id(root, &path));
    }
    Ok(())
}

/// Finds an environment name inside `dir` for which neither `<name>.env` nor `<name>.local.env`
/// exists yet, appending ` (2)`, ` (3)`, ... until one is free.
fn unique_environment_name(dir: &Path, sanitized_name: &str) -> String {
    let mut candidate = sanitized_name.to_string();
    let mut counter = 2;
    while dir.join(format!("{candidate}.env")).exists()
        || dir.join(format!("{candidate}.local.env")).exists()
    {
        candidate = format!("{sanitized_name} ({counter})");
        counter += 1;
    }
    candidate
}
