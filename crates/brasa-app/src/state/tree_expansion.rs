//! Which sidebar folders are expanded. Pure logic over a `folder id -> expanded` map, so it is
//! unit-tested without `gpui`; `views/root.rs` snapshots the live tree into such a map and
//! `views/sidebar.rs` turns it back into `TreeItem`s. A folder missing from the map is collapsed,
//! which is what a freshly opened workspace shows.

use std::collections::HashMap;

/// The ids of every folder above `id`, outermost first. `a/b/c.brasa` gives `a` and `a/b`.
pub fn ancestors(id: &str) -> Vec<&str> {
    id.match_indices('/').map(|(ix, _)| &id[..ix]).collect()
}

/// Expands every folder above `id`, so the entry is visible in the tree.
pub fn reveal(expansion: &mut HashMap<String, bool>, id: &str) {
    for folder in ancestors(id) {
        expansion.insert(folder.to_string(), true);
    }
}

/// Whether some folder above `id` is collapsed (or unknown), hiding the entry.
pub fn is_hidden(expansion: &HashMap<String, bool>, id: &str) -> bool {
    ancestors(id)
        .iter()
        .any(|folder| !expansion.get(*folder).copied().unwrap_or(false))
}

/// Moves the state of the folder `old` and of everything below it to `new`, after a rename, so
/// the renamed folder keeps its expand flag and so do its subfolders.
pub fn rename_prefix(expansion: &mut HashMap<String, bool>, old: &str, new: &str) {
    let below = format!("{old}/");
    let moved: Vec<(String, bool)> = expansion
        .iter()
        .filter(|(id, _)| id.as_str() == old || id.starts_with(&below))
        .map(|(id, expanded)| (id.clone(), *expanded))
        .collect();
    for (id, expanded) in moved {
        expansion.remove(&id);
        expansion.insert(format!("{new}{}", &id[old.len()..]), expanded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn map(entries: &[(&str, bool)]) -> HashMap<String, bool> {
        entries
            .iter()
            .map(|(id, expanded)| (id.to_string(), *expanded))
            .collect()
    }

    #[test]
    fn ancestors_lists_every_folder_above_an_id() {
        assert_eq!(ancestors("a/b/c.brasa"), vec!["a", "a/b"]);
        assert_eq!(ancestors("a/b"), vec!["a"]);
        assert!(ancestors("c.brasa").is_empty());
    }

    #[test]
    fn reveal_expands_only_the_ancestors() {
        let mut expansion = map(&[("a", false), ("a/b", false), ("a/b/d", false)]);
        reveal(&mut expansion, "a/b/c.brasa");
        assert_eq!(
            expansion,
            map(&[("a", true), ("a/b", true), ("a/b/d", false)])
        );
    }

    #[test]
    fn a_folder_missing_from_the_map_counts_as_collapsed() {
        let expansion = map(&[("a", true)]);
        assert!(!is_hidden(&expansion, "a/x.brasa"));
        assert!(is_hidden(&expansion, "a/b/x.brasa"));
        assert!(!is_hidden(&expansion, "root.brasa"));
        assert!(is_hidden(&HashMap::new(), "a/x.brasa"));
    }

    #[test]
    fn rename_prefix_moves_the_folder_and_its_subfolders() {
        let mut expansion = map(&[("a", true), ("a/b", false), ("ab", true), ("z", true)]);
        rename_prefix(&mut expansion, "a", "c");
        assert_eq!(
            expansion,
            map(&[("c", true), ("c/b", false), ("ab", true), ("z", true)])
        );
    }
}
