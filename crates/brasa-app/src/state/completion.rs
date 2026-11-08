//! The autocompletion menu of the request editor's single-line fields: its rows, each with the
//! text it inserts and the range of the field it replaces, and the highlighted one. What fills it
//! comes from `state::variable_completion` (`{{ }}` variables) and `state::header_completion`
//! (common header names and values). Plain Rust, unit tested without a window;
//! `views/completion.rs` draws it and edits the input.

use std::ops::Range;

use super::variable_hint::Hint;

/// One row of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    /// What the row shows.
    pub label: String,
    /// The text that replaces `replace` when the row is chosen.
    pub insert: String,
    /// The byte range of the field's text that `insert` replaces.
    pub replace: Range<usize>,
    /// For a variable, what its tooltip shows (the value on the right of the row).
    pub variable: Option<Hint>,
}

/// The rows of an open menu and the highlighted one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionMenu {
    /// The rows, in the order shown.
    pub items: Vec<CompletionItem>,
    /// The index of the highlighted row in `items`.
    pub selected: usize,
}

impl CompletionMenu {
    /// A menu showing `items`, the first one highlighted.
    pub fn new(items: Vec<CompletionItem>) -> Self {
        Self { items, selected: 0 }
    }

    /// Highlights the next row, from the last back to the first.
    pub fn select_next(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1) % self.items.len();
        }
    }

    /// Highlights the previous row, from the first round to the last.
    pub fn select_previous(&mut self) {
        if !self.items.is_empty() {
            self.selected = self.selected.checked_sub(1).unwrap_or(self.items.len() - 1);
        }
    }

    /// The highlighted row, `None` when there is none.
    pub fn selected(&self) -> Option<&CompletionItem> {
        self.items.get(self.selected)
    }
}

/// The entries of `all` whose key matches `query`, case-insensitively: keys that start with it
/// first, then keys that contain it, each group in the order of `all`. Every one for an empty
/// query.
pub fn rank<'a, T>(all: &'a [T], query: &str, key: impl Fn(&T) -> &str) -> Vec<&'a T> {
    let query = query.to_lowercase();
    let mut starts = Vec::new();
    let mut contains = Vec::new();
    for entry in all {
        let text = key(entry).to_lowercase();
        if text.starts_with(&query) {
            starts.push(entry);
        } else if text.contains(&query) {
            contains.push(entry);
        }
    }
    starts.extend(contains);
    starts
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn items(labels: &[&str]) -> Vec<CompletionItem> {
        labels
            .iter()
            .map(|label| CompletionItem {
                label: label.to_string(),
                insert: label.to_string(),
                replace: 0..0,
                variable: None,
            })
            .collect()
    }

    fn selected(menu: &CompletionMenu) -> Option<&str> {
        menu.selected().map(|item| item.label.as_str())
    }

    #[test]
    fn rank_puts_prefix_matches_before_substring_matches() {
        let all = ["apiKey", "baseUrl", "authBase", "token"];
        assert_eq!(
            rank(&all, "BAS", |entry| entry),
            vec![&"baseUrl", &"authBase"]
        );
        assert_eq!(rank(&all, "", |entry| entry).len(), 4);
        assert!(rank(&all, "zzz", |entry| entry).is_empty());
    }

    #[test]
    fn the_highlighted_row_wraps_around() {
        let mut menu = CompletionMenu::new(items(&["a", "b", "c"]));
        assert_eq!(selected(&menu), Some("a"));
        menu.select_previous();
        assert_eq!(selected(&menu), Some("c"));
        menu.select_next();
        menu.select_next();
        assert_eq!(selected(&menu), Some("b"));

        let mut empty = CompletionMenu::new(Vec::new());
        empty.select_next();
        empty.select_previous();
        assert_eq!(empty.selected(), None);
    }
}
