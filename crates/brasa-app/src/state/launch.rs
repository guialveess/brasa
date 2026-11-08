//! Decides which workspace and which request to open when Brasa is started with a path (a
//! command line argument, or a file the OS asks it to open, such as a double click on a
//! `.brasa` file). Pure logic over paths, so it is tested without a window.

use std::path::{Path, PathBuf};

/// What to open: a workspace folder and, when a file was given, the request inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTarget {
    /// The workspace folder to open.
    pub workspace: PathBuf,
    /// The workspace-relative id (always `/` separated) of the request to open in a tab.
    pub request_id: Option<String>,
}

/// Resolves the startup target from the first command line argument and the remembered
/// workspace.
///
/// - A directory opens as a workspace, as before.
/// - An existing file opens the remembered workspace if it contains the file, otherwise the
///   file's parent folder, plus the file as a request.
/// - Anything else (a missing path) is kept as a workspace path, so opening it reports the
///   error the same way it always did.
/// - No argument opens the remembered workspace, if any.
pub fn resolve_launch_target(
    arg: Option<&Path>,
    remembered: Option<&Path>,
) -> Option<LaunchTarget> {
    match arg {
        Some(path) if path.is_file() => Some(target_for_file(path, remembered)),
        Some(path) => Some(LaunchTarget {
            workspace: path.to_path_buf(),
            request_id: None,
        }),
        None => remembered.map(|workspace| LaunchTarget {
            workspace: workspace.to_path_buf(),
            request_id: None,
        }),
    }
}

/// The target for an existing `file`: `candidate` when it contains the file, else the file's
/// parent folder. `candidate` is the remembered workspace at startup, or the open one when the
/// OS hands the app a file while it runs.
pub fn target_for_file(file: &Path, candidate: Option<&Path>) -> LaunchTarget {
    let canonical_file = canonical(file);
    if let Some(root) = candidate {
        let canonical_root = canonical(root);
        if let Some(id) = relative_id(&canonical_root, &canonical_file) {
            return LaunchTarget {
                workspace: root.to_path_buf(),
                request_id: Some(id),
            };
        }
    }
    let parent = canonical_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let request_id = canonical_file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    LaunchTarget {
        workspace: parent,
        request_id,
    }
}

/// Converts the `file://` URLs macOS hands over into paths, dropping anything else.
pub fn paths_from_urls(urls: &[String]) -> Vec<PathBuf> {
    urls.iter()
        .filter_map(|url| file_url_to_path(url))
        .collect()
}

/// Converts one `file://` URL (percent-encoded, optional `localhost` host) into a path.
pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let decoded = percent_decode(rest)?;
    // `file:///C:/dir/file` carries a leading slash that is not part of a Windows path.
    let bytes = decoded.as_bytes();
    let path = if bytes.len() > 2 && bytes[2] == b':' && bytes[1].is_ascii_alphabetic() {
        &decoded[1..]
    } else {
        decoded.as_str()
    };
    Some(PathBuf::from(path))
}

/// Decodes `%XX` escapes as UTF-8. `None` when an escape is malformed or the result is not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `dunce` avoids the verbatim `\\?\C:\...` form on Windows. A path that cannot be resolved is
/// compared as given.
fn canonical(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The `/` separated path of `file` relative to `root`, when `file` is inside it.
fn relative_id(root: &Path, file: &Path) -> Option<String> {
    let relative = file.strip_prefix(root).ok()?;
    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::fs;

    use super::*;

    fn make_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, "GET https://example.com\n").expect("write");
        path
    }

    #[test]
    fn a_directory_argument_is_the_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let other = tempfile::tempdir().expect("tempdir");
        let target = resolve_launch_target(Some(dir.path()), Some(other.path())).expect("target");
        assert_eq!(target.workspace, dir.path());
        assert_eq!(target.request_id, None);
    }

    #[test]
    fn no_argument_uses_the_remembered_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = resolve_launch_target(None, Some(dir.path())).expect("target");
        assert_eq!(target.workspace, dir.path());
        assert_eq!(resolve_launch_target(None, None), None);
    }

    #[test]
    fn a_missing_path_stays_a_workspace_path() {
        let target = resolve_launch_target(Some(Path::new("/does/not/exist")), None).expect("t");
        assert_eq!(target.workspace, Path::new("/does/not/exist"));
        assert_eq!(target.request_id, None);
    }

    #[test]
    fn a_file_inside_the_remembered_workspace_opens_that_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = make_file(dir.path(), "users/list.brasa");
        let target = resolve_launch_target(Some(&file), Some(dir.path())).expect("target");
        assert_eq!(target.workspace, dir.path());
        assert_eq!(target.request_id.as_deref(), Some("users/list.brasa"));
    }

    #[test]
    fn a_file_outside_the_remembered_workspace_opens_its_parent_folder() {
        let remembered = tempfile::tempdir().expect("tempdir");
        let dir = tempfile::tempdir().expect("tempdir");
        let file = make_file(dir.path(), "health.brasa");
        let target = resolve_launch_target(Some(&file), Some(remembered.path())).expect("target");
        assert_eq!(
            target.workspace,
            dunce::canonicalize(dir.path()).expect("canonical")
        );
        assert_eq!(target.request_id.as_deref(), Some("health.brasa"));
    }

    #[test]
    fn a_file_without_a_remembered_workspace_opens_its_parent_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = make_file(dir.path(), "health.brasa");
        let target = resolve_launch_target(Some(&file), None).expect("target");
        assert_eq!(
            target.workspace,
            dunce::canonicalize(dir.path()).expect("canonical")
        );
        assert_eq!(target.request_id.as_deref(), Some("health.brasa"));
    }

    #[test]
    fn a_relative_remembered_workspace_is_compared_canonicalized() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = make_file(dir.path(), "a/b.brasa");
        let indirect = dir.path().join("a").join("..");
        let target = target_for_file(&file, Some(&indirect));
        assert_eq!(target.workspace, indirect);
        assert_eq!(target.request_id.as_deref(), Some("a/b.brasa"));
    }

    #[test]
    fn file_urls_are_percent_decoded() {
        assert_eq!(
            file_url_to_path("file:///Users/ana/My%20API/get%20all.brasa"),
            Some(PathBuf::from("/Users/ana/My API/get all.brasa"))
        );
        assert_eq!(
            file_url_to_path("file://localhost/tmp/a.brasa"),
            Some(PathBuf::from("/tmp/a.brasa"))
        );
        assert_eq!(
            file_url_to_path("file:///tmp/%C3%B1.brasa"),
            Some(PathBuf::from("/tmp/\u{f1}.brasa"))
        );
    }

    #[test]
    fn non_file_or_malformed_urls_are_dropped() {
        assert_eq!(file_url_to_path("https://example.com/a.brasa"), None);
        assert_eq!(file_url_to_path("file:///tmp/%zz"), None);
        assert_eq!(file_url_to_path("file://remote/tmp/a"), None);
        let urls = vec![
            "brasa://x".to_string(),
            "file:///tmp/a.brasa".to_string(),
        ];
        assert_eq!(
            paths_from_urls(&urls),
            vec![PathBuf::from("/tmp/a.brasa")]
        );
    }
}
