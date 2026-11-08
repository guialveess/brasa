//! Sidebar filter: which tree entries a filter query matches. Pure logic over
//! `brasa_workspace::Node`, so it is unit-tested without `gpui`; `views/sidebar.rs` turns the
//! result into the `TreeItem`s the tree actually renders.

use std::collections::HashSet;

use brasa_workspace::Node;

/// The ids that must stay visible when filtering the tree by `query`: every request whose name
/// or id (its workspace-relative path) contains `query` as a case-insensitive substring, plus
/// every ancestor folder of a match, so a match is never hidden behind a filtered-out parent.
///
/// Callers only call this while the filter is non-empty; an empty query would trivially match
/// everything, which `views/sidebar.rs` already handles by skipping filtering entirely.
pub fn visible_ids(nodes: &[Node], query: &str) -> HashSet<String> {
    let query = query.trim().to_ascii_lowercase();
    let mut visible = HashSet::new();
    mark_visible(nodes, &query, &mut visible);
    visible
}

/// Recurses into `nodes`, inserting the id of every match and every ancestor of a match into
/// `visible`. Returns whether anything in `nodes` (at any depth) matched, so a caller folder can
/// decide whether to include itself.
fn mark_visible(nodes: &[Node], query: &str, visible: &mut HashSet<String>) -> bool {
    let mut any = false;
    for node in nodes {
        let self_matches = node_matches(node, query);
        let child_matches = match node {
            Node::Folder(folder) => mark_visible(&folder.children, query, visible),
            Node::Request(_) => false,
        };
        if self_matches || child_matches {
            visible.insert(node.id().to_string());
            any = true;
        }
    }
    any
}

/// Whether `node`'s name or id contains `query` (already trimmed and lowercased),
/// case-insensitive.
fn node_matches(node: &Node, query: &str) -> bool {
    node.name().to_ascii_lowercase().contains(query)
        || node.id().to_ascii_lowercase().contains(query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brasa_workspace::{Folder, RequestEntry};
    use pretty_assertions::assert_eq;

    fn request(id: &str, name: &str) -> Node {
        Node::Request(RequestEntry {
            id: id.to_string(),
            name: name.to_string(),
            broken: None,
            method: Some(brasa_core::Method::Get),
        })
    }

    fn folder(id: &str, name: &str, children: Vec<Node>) -> Node {
        Node::Folder(Folder {
            id: id.to_string(),
            name: name.to_string(),
            children,
        })
    }

    fn sample_tree() -> Vec<Node> {
        vec![
            folder(
                "auth",
                "auth",
                vec![
                    request("auth/login.brasa", "login"),
                    request("auth/refresh.brasa", "refresh"),
                ],
            ),
            folder(
                "users",
                "users",
                vec![request("users/list.brasa", "list")],
            ),
            request("health.brasa", "health"),
        ]
    }

    #[test]
    fn matches_by_request_name_case_insensitive() {
        let visible = visible_ids(&sample_tree(), "LOGIN");
        assert!(visible.contains("auth/login.brasa"));
        assert!(!visible.contains("auth/refresh.brasa"));
    }

    #[test]
    fn matches_by_path_substring() {
        let visible = visible_ids(&sample_tree(), "auth/");
        assert!(visible.contains("auth/login.brasa"));
        assert!(visible.contains("auth/refresh.brasa"));
        assert!(!visible.contains("users/list.brasa"));
    }

    #[test]
    fn a_match_keeps_its_ancestor_folders_visible() {
        let visible = visible_ids(&sample_tree(), "login");
        assert!(visible.contains("auth"));
        assert!(visible.contains("auth/login.brasa"));
        assert!(!visible.contains("users"));
    }

    #[test]
    fn non_matching_siblings_are_excluded() {
        let visible = visible_ids(&sample_tree(), "health");
        assert_eq!(visible.len(), 1);
        assert!(visible.contains("health.brasa"));
    }

    #[test]
    fn no_match_yields_an_empty_set() {
        let visible = visible_ids(&sample_tree(), "does-not-exist");
        assert!(visible.is_empty());
    }

    #[test]
    fn whitespace_is_trimmed_before_matching() {
        let visible = visible_ids(&sample_tree(), "  login  ");
        assert!(visible.contains("auth/login.brasa"));
    }
}
