//! Log text about the open workspace (GitHub #36): its size when opened, and failures of
//! workspace operations made safe to log.

use brasa_workspace::{Node, WorkspaceError};

/// How many folders and requests a workspace tree holds, and the ids of the requests that do not
/// parse.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TreeCounts<'a> {
    /// Folders, at any depth.
    pub folders: usize,
    /// Requests, broken ones included.
    pub requests: usize,
    /// Ids of the requests whose file does not parse.
    pub broken: Vec<&'a str>,
}

/// Counts `nodes` recursively.
pub fn tree_counts(nodes: &[Node]) -> TreeCounts<'_> {
    let mut counts = TreeCounts::default();
    add_counts(nodes, &mut counts);
    counts
}

fn add_counts<'a>(nodes: &'a [Node], counts: &mut TreeCounts<'a>) {
    for node in nodes {
        match node {
            Node::Folder(folder) => {
                counts.folders += 1;
                add_counts(&folder.children, counts);
            }
            Node::Request(request) => {
                counts.requests += 1;
                if request.broken.is_some() {
                    counts.broken.push(&request.id);
                }
            }
        }
    }
}

/// `error` as safe to log at Warn. A parse error quotes the offending line, which may hold a
/// header value or an environment secret, so it is reduced to its kind; its full text is only
/// logged at Debug (see [`is_detailed_only_at_debug`]).
pub fn workspace_error_for_log(error: &WorkspaceError) -> String {
    match error {
        WorkspaceError::Parse(_) => "the request file does not parse".to_string(),
        WorkspaceError::EnvParse { name, .. } => format!("environment {name:?} does not parse"),
        other => other.to_string(),
    }
}

/// Whether [`workspace_error_for_log`] hides part of `error`, so its full text is worth a Debug
/// line.
pub fn is_detailed_only_at_debug(error: &WorkspaceError) -> bool {
    matches!(
        error,
        WorkspaceError::Parse(_) | WorkspaceError::EnvParse { .. }
    )
}

/// Logs a failed workspace operation: the safe text at Warn, the full one at Debug when they
/// differ. `action` names the operation, such as "save posts/new.brasa".
pub fn log_workspace_error(action: &str, error: &WorkspaceError) {
    log::warn!("{action} failed: {}", workspace_error_for_log(error));
    if is_detailed_only_at_debug(error) {
        log::debug!("{action} failed: {error}");
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;

    #[test]
    fn counts_folders_requests_and_broken_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("users")).expect("mkdir");
        fs::write(dir.path().join("ok.brasa"), "GET https://example.com\n").expect("write");
        fs::write(
            dir.path().join("users/bad.brasa"),
            "Authorization Bearer x\n",
        )
        .expect("write");
        let workspace = brasa_workspace::Workspace::open(dir.path()).expect("open");

        assert_eq!(
            tree_counts(workspace.tree()),
            TreeCounts {
                folders: 1,
                requests: 2,
                broken: vec!["users/bad.brasa"],
            }
        );
    }

    #[test]
    fn parse_errors_do_not_quote_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("bad.brasa"),
            "GET https://example.com\nAuthorization Bearer secret\n",
        )
        .expect("write");
        let workspace = brasa_workspace::Workspace::open(dir.path()).expect("open");
        let error = workspace
            .load_request("bad.brasa")
            .expect_err("does not parse");

        assert!(error.to_string().contains("secret"), "{error}");
        assert_eq!(
            workspace_error_for_log(&error),
            "the request file does not parse"
        );
        assert!(is_detailed_only_at_debug(&error));
    }

    #[test]
    fn other_errors_are_logged_as_is() {
        let error = WorkspaceError::NotFound("a.brasa".to_string());
        assert_eq!(
            workspace_error_for_log(&error),
            "no entry found for \"a.brasa\""
        );
        assert!(!is_detailed_only_at_debug(&error));
    }
}
