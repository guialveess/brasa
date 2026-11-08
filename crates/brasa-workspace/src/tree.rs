//! The collection tree returned by [`crate::Workspace::tree`]: the folders and requests found
//! under the workspace root.

use std::cmp::Ordering;

use brasa_core::Method;

/// One entry of the collection tree: a folder or a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A folder, mapped from a directory under the workspace root.
    Folder(Folder),
    /// A request, mapped from a `.brasa` file.
    Request(RequestEntry),
}

impl Node {
    /// The stable id of this entry. See [`Folder::id`] and [`RequestEntry::id`].
    pub fn id(&self) -> &str {
        match self {
            Node::Folder(folder) => &folder.id,
            Node::Request(request) => &request.id,
        }
    }

    /// The display name of this entry: the directory name for a folder, the file name without
    /// the `.brasa` extension for a request.
    pub fn name(&self) -> &str {
        match self {
            Node::Folder(folder) => &folder.name,
            Node::Request(request) => &request.name,
        }
    }
}

/// A folder in the collection tree, mapped from a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// The stable id: the folder's path relative to the workspace root, using `/` as the
    /// separator regardless of the host OS, so the id is identical on every platform.
    pub id: String,
    /// The folder name (the directory name).
    pub name: String,
    /// The folder's direct children, already sorted: folders first, then requests, natural sort by
    /// name within each group.
    pub children: Vec<Node>,
}

/// A request in the collection tree, mapped from a `.brasa` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestEntry {
    /// The stable id: the file's path relative to the workspace root (including the `.brasa`
    /// extension), using `/` as the separator regardless of the host OS.
    pub id: String,
    /// The request name (the file name without the `.brasa` extension).
    pub name: String,
    /// `Some(message)` when the file failed to read or parse. The request is still listed, so a
    /// single broken file never breaks the whole scan, but [`crate::Workspace::load_request`]
    /// will fail for it until the file is fixed.
    pub broken: Option<String>,
    /// The request's HTTP method, read from the same parse that fills [`Self::broken`] (no
    /// second read of the file). `None` when the file is broken.
    pub method: Option<Method>,
}

/// Sorts a list of nodes in place: folders before requests, then natural sort by name within
/// each group.
pub(crate) fn sort_nodes(nodes: &mut [Node]) {
    nodes.sort_by(|a, b| {
        is_request(a)
            .cmp(&is_request(b))
            .then_with(|| natural_cmp(a.name(), b.name()))
            .then_with(|| a.name().cmp(b.name()))
    });
}

/// Whether `node` is a request, used to sort folders before requests.
fn is_request(node: &Node) -> bool {
    matches!(node, Node::Request(_))
}

/// Compares two names with "natural sort": runs of ASCII digits are compared by numeric value,
/// so `"item2"` sorts before `"item10"`. Everything else is compared case-insensitively,
/// character by character. Ties (for example `"a"` versus `"A"`) are broken by the caller with a
/// plain, case-sensitive comparison, so the overall order stays a total, stable order.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut a_chars = a.chars().peekable();
    let mut b_chars = b.chars().peekable();
    loop {
        match (a_chars.peek().copied(), b_chars.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(a_char), Some(b_char)) => {
                if a_char.is_ascii_digit() && b_char.is_ascii_digit() {
                    let a_number = take_digits(&mut a_chars);
                    let b_number = take_digits(&mut b_chars);
                    match compare_numeric(&a_number, &b_number) {
                        Ordering::Equal => continue,
                        other => return other,
                    }
                } else {
                    a_chars.next();
                    b_chars.next();
                    match a_char
                        .to_ascii_lowercase()
                        .cmp(&b_char.to_ascii_lowercase())
                    {
                        Ordering::Equal => continue,
                        other => return other,
                    }
                }
            }
        }
    }
}

/// Consumes and returns the run of consecutive ASCII digits at the front of `chars`.
fn take_digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut digits = String::new();
    while let Some(character) = chars.peek() {
        if character.is_ascii_digit() {
            digits.push(*character);
            chars.next();
        } else {
            break;
        }
    }
    digits
}

/// Compares two runs of digits by numeric value, without parsing them into a number (so a run of
/// any length is handled). Leading zeros do not affect the result: `"007"` and `"7"` compare
/// equal.
fn compare_numeric(a: &str, b: &str) -> Ordering {
    let a_trimmed = a.trim_start_matches('0');
    let b_trimmed = b.trim_start_matches('0');
    a_trimmed
        .len()
        .cmp(&b_trimmed.len())
        .then_with(|| a_trimmed.cmp(b_trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn request(name: &str) -> Node {
        Node::Request(RequestEntry {
            id: format!("{name}.brasa"),
            name: name.to_string(),
            broken: None,
            method: Some(Method::Get),
        })
    }

    fn folder(name: &str) -> Node {
        Node::Folder(Folder {
            id: name.to_string(),
            name: name.to_string(),
            children: Vec::new(),
        })
    }

    #[test]
    fn natural_cmp_orders_digit_runs_numerically() {
        assert_eq!(natural_cmp("item2", "item10"), Ordering::Less);
        assert_eq!(natural_cmp("item10", "item2"), Ordering::Greater);
        assert_eq!(natural_cmp("item2", "item2"), Ordering::Equal);
    }

    #[test]
    fn natural_cmp_ignores_leading_zeros() {
        assert_eq!(natural_cmp("item007", "item7"), Ordering::Equal);
    }

    #[test]
    fn natural_cmp_is_case_insensitive() {
        assert_eq!(natural_cmp("Banana", "apple"), Ordering::Greater);
        assert_eq!(natural_cmp("apple", "Apple"), Ordering::Equal);
    }

    #[test]
    fn sort_nodes_puts_folders_before_requests() {
        let mut nodes = vec![request("z"), folder("a")];
        sort_nodes(&mut nodes);
        assert_eq!(nodes, vec![folder("a"), request("z")]);
    }

    #[test]
    fn sort_nodes_uses_natural_sort_within_each_group() {
        let mut nodes = vec![request("item10"), request("item2"), request("item1")];
        sort_nodes(&mut nodes);
        assert_eq!(
            nodes,
            vec![request("item1"), request("item2"), request("item10")]
        );
    }

    #[test]
    fn sort_nodes_breaks_case_insensitive_ties_deterministically() {
        let mut nodes = vec![request("b"), request("A"), request("a"), request("B")];
        sort_nodes(&mut nodes);
        assert_eq!(
            nodes,
            vec![request("A"), request("a"), request("B"), request("b")]
        );
    }
}
