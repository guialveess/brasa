//! The variables of the autocompletion menu (`state::completion`) in the URL and the values of
//! Params, Headers and Form: where the typed `{{` marker is, what text a chosen variable replaces
//! and which variables match what was typed. Plain Rust over the field's text and cursor, unit
//! tested without a window.

use std::ops::Range;

use super::completion::{CompletionItem, rank};
use super::variable_hint::Hint;

/// The text a chosen variable replaces, and what was typed after `{{` to filter the list with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    /// The byte range of the field's text that `{{name}}` replaces.
    pub replace: Range<usize>,
    /// The part of the name typed so far, `""` right after `{{` or for `Ctrl+Space` outside a
    /// marker.
    pub query: String,
}

/// Whether `c` can be part of what is typed as a variable name after `{{`. Anything else (a
/// space after the name, the `(` of a template function, a quote) means the user is not typing a
/// variable name, so the menu closes.
fn is_name_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '{' | '}' | '(' | ')' | '"' | '\'' | ',')
}

/// Where a completion applies in `text` with the cursor at byte `cursor`.
///
/// Inside a marker still being typed (`{{ba|`, or `{{ba|seUrl}}` when editing an existing one)
/// it replaces the whole marker: from its `{{` to the end of the name after the cursor and a
/// closing `}}` when there is one, so the braces are never doubled. Elsewhere, only an explicit
/// `Ctrl+Space` (`explicit`) opens the menu, inserting at `selection` (the cursor when empty).
pub fn trigger_at(
    text: &str,
    cursor: usize,
    selection: Range<usize>,
    explicit: bool,
) -> Option<Trigger> {
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    if let Some(open) = before.rfind("{{") {
        let typed = &before[open + 2..];
        let query = typed.trim_start();
        if !typed.contains("}}") && query.chars().all(is_name_char) {
            let after = &text[cursor..];
            let name_len: usize = after
                .chars()
                .take_while(|c| is_name_char(*c))
                .map(char::len_utf8)
                .sum();
            let rest = &after[name_len..];
            let close_len = if rest.trim_start().starts_with("}}") {
                rest.len() - rest.trim_start().len() + 2
            } else {
                0
            };
            return Some(Trigger {
                replace: open..cursor + name_len + close_len,
                query: query.to_string(),
            });
        }
    }
    explicit.then(|| Trigger {
        replace: selection.start.min(text.len())..selection.end.min(text.len()),
        query: String::new(),
    })
}

/// The marker inserted for the variable `name`.
pub fn marker(name: &str) -> String {
    format!("{{{{{name}}}}}")
}

/// The variables of `all` matching `query`, ranked by [`rank`].
pub fn filter(all: &[Hint], query: &str) -> Vec<Hint> {
    rank(all, query, |hint| hint.name.as_str())
        .into_iter()
        .cloned()
        .collect()
}

/// The menu rows for `hints`, each inserting its `{{name}}` marker in place of `replace`.
pub fn items(hints: Vec<Hint>, replace: &Range<usize>) -> Vec<CompletionItem> {
    hints
        .into_iter()
        .map(|hint| CompletionItem {
            label: hint.name.clone(),
            insert: marker(&hint.name),
            replace: replace.clone(),
            variable: Some(hint),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn trigger(replace: Range<usize>, query: &str) -> Option<Trigger> {
        Some(Trigger {
            replace,
            query: query.to_string(),
        })
    }

    fn hints(names: &[&str]) -> Vec<Hint> {
        names
            .iter()
            .map(|name| Hint {
                name: name.to_string(),
                value: Some(String::new()),
                masked: false,
                origin: None,
            })
            .collect()
    }

    fn names(hints: &[Hint]) -> Vec<&str> {
        hints.iter().map(|hint| hint.name.as_str()).collect()
    }

    #[test]
    fn typing_braces_opens_a_marker_with_an_empty_query() {
        let text = "http://{{";
        assert_eq!(trigger_at(text, 9, 9..9, false), trigger(7..9, ""));
    }

    #[test]
    fn the_typed_name_filters_and_the_whole_marker_is_replaced() {
        assert_eq!(trigger_at("{{ba", 4, 4..4, false), trigger(0..4, "ba"));
        assert_eq!(trigger_at("{{ ba", 5, 5..5, false), trigger(0..5, "ba"));
        // Editing an existing marker: the rest of the name and the closing braces go too.
        let text = "{{baseUrl}}/users";
        assert_eq!(trigger_at(text, 4, 4..4, false), trigger(0..11, "ba"));
        assert_eq!(trigger_at("{{ba }}", 4, 4..4, false), trigger(0..7, "ba"));
        assert_eq!(trigger_at("{{}}", 2, 2..2, false), trigger(0..4, ""));
    }

    #[test]
    fn a_closed_marker_or_something_else_than_a_name_closes_the_menu() {
        assert_eq!(trigger_at("{{a}}/x", 7, 7..7, false), None);
        assert_eq!(trigger_at("{{a}}", 5, 5..5, false), None);
        assert_eq!(trigger_at("{{ uuid(", 8, 8..8, false), None);
        assert_eq!(trigger_at("{{base url", 10, 10..10, false), None);
        assert_eq!(trigger_at("http://x", 8, 8..8, false), None);
    }

    #[test]
    fn ctrl_space_outside_a_marker_inserts_at_the_cursor_or_the_selection() {
        assert_eq!(trigger_at("http://", 7, 7..7, true), trigger(7..7, ""));
        assert_eq!(
            trigger_at("http://host/x", 11, 7..11, true),
            trigger(7..11, "")
        );
        // Inside a marker it still completes the marker.
        assert_eq!(trigger_at("x {{to", 6, 6..6, true), trigger(2..6, "to"));
    }

    #[test]
    fn non_ascii_names_keep_char_boundaries() {
        let text = "{{añ}}";
        assert_eq!(
            trigger_at(text, 3, 3..3, false),
            trigger(0..text.len(), "a")
        );
    }

    #[test]
    fn marker_wraps_the_name_in_braces() {
        assert_eq!(marker("baseUrl"), "{{baseUrl}}");
    }

    #[test]
    fn items_insert_markers_in_place_of_the_trigger() {
        let rows = items(hints(&["baseUrl"]), &(3..5));
        assert_eq!(rows[0].label, "baseUrl");
        assert_eq!(rows[0].insert, "{{baseUrl}}");
        assert_eq!(rows[0].replace, 3..5);
        assert!(rows[0].variable.is_some());
    }

    #[test]
    fn filter_puts_prefix_matches_before_substring_matches() {
        let all = hints(&["apiKey", "baseUrl", "authBase", "token"]);
        assert_eq!(names(&filter(&all, "BAS")), vec!["baseUrl", "authBase"]);
        assert_eq!(
            names(&filter(&all, "")),
            vec!["apiKey", "baseUrl", "authBase", "token"]
        );
        assert!(filter(&all, "zzz").is_empty());
    }
}
