//! Builds a `brasa_format::export::ExportNode` tree of a folder (or the whole workspace), loading
//! every request's body from disk, for `Workspace::export_tree`.

use brasa_format::export::ExportNode;

use crate::error::WorkspaceError;
use crate::tree::{Folder, Node};
use crate::workspace::Workspace;

impl Workspace {
    /// Builds the export tree of the folder `folder_id`, or the whole workspace when `folder_id`
    /// is `None`, loading and parsing every request under it from disk.
    ///
    /// Fails with [`WorkspaceError::NotFound`] when `folder_id` names no folder, and with
    /// whatever [`Workspace::load_request`] fails with for the first request that cannot be
    /// read or parsed.
    pub fn export_tree(&self, folder_id: Option<&str>) -> Result<Vec<ExportNode>, WorkspaceError> {
        let nodes = match folder_id {
            None => self.tree(),
            Some(id) => {
                &find_folder(self.tree(), id)
                    .ok_or_else(|| WorkspaceError::NotFound(id.to_string()))?
                    .children
            }
        };
        self.export_nodes(nodes)
    }

    fn export_nodes(&self, nodes: &[Node]) -> Result<Vec<ExportNode>, WorkspaceError> {
        nodes.iter().map(|node| self.export_node(node)).collect()
    }

    fn export_node(&self, node: &Node) -> Result<ExportNode, WorkspaceError> {
        match node {
            Node::Folder(folder) => Ok(ExportNode::Folder {
                name: folder.name.clone(),
                children: self.export_nodes(&folder.children)?,
            }),
            Node::Request(entry) => {
                let request = self.load_request(&entry.id)?;
                Ok(ExportNode::Request {
                    name: entry.name.clone(),
                    request,
                })
            }
        }
    }
}

/// Finds the folder with the given id, searching `nodes` depth first.
fn find_folder<'a>(nodes: &'a [Node], id: &str) -> Option<&'a Folder> {
    for node in nodes {
        match node {
            Node::Folder(folder) if folder.id == id => return Some(folder),
            Node::Folder(folder) => {
                if let Some(found) = find_folder(&folder.children, id) {
                    return Some(found);
                }
            }
            Node::Request(_) => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;

    fn request_text(url: &str) -> String {
        format!("GET {url}\n")
    }

    #[test]
    fn exports_the_whole_workspace_when_no_folder_is_given() {
        let dir = tempdir().expect("temp dir");
        std::fs::create_dir(dir.path().join("US001")).expect("create folder");
        std::fs::write(
            dir.path().join("US001/list.brasa"),
            request_text("https://example.com/users"),
        )
        .expect("write request");
        let workspace = Workspace::open(dir.path()).expect("open workspace");

        let tree = workspace.export_tree(None).expect("export tree");
        assert_eq!(tree.len(), 1);
        match &tree[0] {
            ExportNode::Folder { name, children } => {
                assert_eq!(name, "US001");
                assert_eq!(children.len(), 1);
            }
            ExportNode::Request { .. } => panic!("expected a folder"),
        }
    }

    #[test]
    fn exports_a_single_folder_by_id() {
        let dir = tempdir().expect("temp dir");
        std::fs::create_dir(dir.path().join("US001")).expect("create folder");
        std::fs::write(
            dir.path().join("US001/list.brasa"),
            request_text("https://example.com/users"),
        )
        .expect("write request");
        std::fs::write(
            dir.path().join("other.brasa"),
            request_text("https://example.com/other"),
        )
        .expect("write request");
        let workspace = Workspace::open(dir.path()).expect("open workspace");

        let tree = workspace.export_tree(Some("US001")).expect("export tree");
        assert_eq!(tree.len(), 1);
        match &tree[0] {
            ExportNode::Request { name, request } => {
                assert_eq!(name, "list");
                assert_eq!(request.url, "https://example.com/users");
            }
            ExportNode::Folder { .. } => panic!("expected a request"),
        }
    }

    #[test]
    fn an_unknown_folder_id_is_not_found() {
        let dir = tempdir().expect("temp dir");
        let workspace = Workspace::open(dir.path()).expect("open workspace");
        assert!(matches!(
            workspace.export_tree(Some("missing")),
            Err(WorkspaceError::NotFound(_))
        ));
    }
}
