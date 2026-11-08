//! Converting between a workspace-relative id and an actual filesystem path.
//!
//! An id is the path of a folder or a request file relative to the workspace root, always using
//! `/` as the separator regardless of the host OS, so the same id refers to the same entry on
//! Linux, Windows and macOS.

use std::path::{Component, Path, PathBuf};

use crate::error::WorkspaceError;

/// Builds the stable id of `path`, relative to `root`, using `/` as the separator.
///
/// `path` is expected to be `root` itself or a descendant of it, as produced by walking the
/// workspace folder. Any component of the relative path that is not a plain name (there should
/// be none, in practice) is silently dropped, which only makes the resulting id shorter, never
/// unsafe.
pub(crate) fn path_to_id(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    parts.join("/")
}

/// Resolves an id back into an actual filesystem path under `root`.
///
/// Rejects anything that is not a plain, non-empty relative path made of normal components: an
/// empty id, an absolute path, or a path containing a `.` or `..` component. This is what keeps
/// every [`crate::Workspace`] operation confined to the workspace root.
pub(crate) fn id_to_path(root: &Path, id: &str) -> Result<PathBuf, WorkspaceError> {
    validate_relative_path(id)?;
    Ok(root.join(id))
}

/// Checks that `id` (or a single path component such as an environment name) is a non-empty
/// relative path made only of normal components, rejecting anything that could escape the
/// workspace root.
pub(crate) fn validate_relative_path(id: &str) -> Result<(), WorkspaceError> {
    if id.is_empty() {
        return Err(WorkspaceError::InvalidId(id.to_string()));
    }
    for component in Path::new(id).components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err(WorkspaceError::InvalidId(id.to_string())),
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn path_to_id_uses_forward_slashes() {
        let root = Path::new("/workspace");
        let path = Path::new("/workspace/auth/login.brasa");
        assert_eq!(path_to_id(root, path), "auth/login.brasa");
    }

    #[test]
    fn path_to_id_of_root_itself_is_empty() {
        let root = Path::new("/workspace");
        assert_eq!(path_to_id(root, root), "");
    }

    #[test]
    fn id_to_path_joins_a_valid_relative_id() {
        let root = Path::new("/workspace");
        assert_eq!(
            id_to_path(root, "auth/login.brasa").unwrap(),
            Path::new("/workspace/auth/login.brasa")
        );
    }

    #[test]
    fn id_to_path_rejects_empty_id() {
        assert!(id_to_path(Path::new("/workspace"), "").is_err());
    }

    #[test]
    fn id_to_path_rejects_parent_component() {
        assert!(id_to_path(Path::new("/workspace"), "../secret.brasa").is_err());
        assert!(id_to_path(Path::new("/workspace"), "auth/../../secret.brasa").is_err());
    }

    #[test]
    fn id_to_path_rejects_absolute_id() {
        assert!(id_to_path(Path::new("/workspace"), "/etc/passwd").is_err());
    }
}
