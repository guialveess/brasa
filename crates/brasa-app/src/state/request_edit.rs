//! Pure editing operations on a [`Request`], used by the request editor view
//! (`views/request_editor.rs`).
//!
//! Kept free of `gpui` types so every rule here (what a button does to the data) is unit-tested
//! directly, without a window. The view's job is only to turn a click or a keystroke into a call
//! into this module, then mark the tab dirty and re-render.

use std::cmp::Ordering;

use brasa_core::{Body, KeyValue, Request};
use brasa_workspace::natural_cmp;
use rust_i18n::t;

/// The kind of body a request has, mirroring [`Body`] without its content. Used by the body type
/// selector, which needs to list and compare kinds without carrying the (possibly large) body
/// text around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BodyKind {
    /// No body.
    #[default]
    None,
    /// A JSON body.
    Json,
    /// A plain text body.
    Text,
    /// An XML body.
    Xml,
    /// A `application/x-www-form-urlencoded` body.
    Form,
    /// A `multipart/form-data` body.
    Multipart,
}

impl BodyKind {
    /// The kind of an existing [`Body`].
    pub fn of(body: &Body) -> Self {
        match body {
            Body::None => BodyKind::None,
            Body::Json(_) => BodyKind::Json,
            Body::Text(_) => BodyKind::Text,
            Body::Xml(_) => BodyKind::Xml,
            Body::Form(_) => BodyKind::Form,
            Body::Multipart(_) => BodyKind::Multipart,
        }
    }

    /// Every kind, in the order the body type selector lists them.
    pub const ALL: [BodyKind; 6] = [
        BodyKind::None,
        BodyKind::Json,
        BodyKind::Text,
        BodyKind::Xml,
        BodyKind::Form,
        BodyKind::Multipart,
    ];

    /// The label shown in the body type selector.
    pub fn label(self) -> String {
        match self {
            BodyKind::None => t!("request.body.none").into_owned(),
            BodyKind::Json => "JSON".to_string(),
            BodyKind::Text => t!("request.body.text").into_owned(),
            BodyKind::Xml => "XML".to_string(),
            BodyKind::Form => t!("request.body.form_urlencoded").into_owned(),
            BodyKind::Multipart => "Multipart".to_string(),
        }
    }
}

/// What the body kinds a request is not showing held, while its tab is open, so switching the
/// body type and back restores them instead of starting empty. Never saved: only the body kind
/// selected when saving is written to the file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BodyStash {
    /// The raw text of the last JSON, Text or XML body.
    text: Option<String>,
    /// The fields of the last Form or Multipart body.
    form: Option<Vec<KeyValue>>,
}

impl BodyStash {
    /// About how many bytes the stash holds, for the undo history's budget
    /// (`state::edit_history`).
    pub fn approx_size(&self) -> usize {
        let text = self.text.as_ref().map_or(0, String::len);
        let form: usize = self
            .form
            .iter()
            .flatten()
            .map(|row| row.key.len() + row.value.len() + 16)
            .sum();
        text + form
    }
}

/// Switches `request.body` to `kind`, putting what the current body holds in `stash` and
/// restoring what `stash` holds for `kind`.
///
/// JSON, Text and XML share one raw text, since they are all plain strings under the hood:
/// switching from JSON to Text (for example, while writing a body that is not valid JSON yet)
/// keeps what was typed. Form and Multipart share their fields the same way. Fields cannot be
/// recovered from raw text nor the other way round,
/// so each keeps its own content in `stash`: switching from Form to JSON and back brings the
/// fields back, and from JSON to None and back brings the text back.
pub fn set_body_kind(request: &mut Request, kind: BodyKind, stash: &mut BodyStash) {
    match std::mem::take(&mut request.body) {
        Body::Json(text) | Body::Text(text) | Body::Xml(text) => stash.text = Some(text),
        Body::Form(rows) | Body::Multipart(rows) => stash.form = Some(rows),
        Body::None => {}
    }
    request.body = match kind {
        BodyKind::None => Body::None,
        BodyKind::Json => Body::Json(stash.text.take().unwrap_or_default()),
        BodyKind::Text => Body::Text(stash.text.take().unwrap_or_default()),
        BodyKind::Xml => Body::Xml(stash.text.take().unwrap_or_default()),
        BodyKind::Form => Body::Form(stash.form.take().unwrap_or_default()),
        BodyKind::Multipart => Body::Multipart(stash.form.take().unwrap_or_default()),
    };
}

/// Replaces the raw text of a JSON, Text or XML body, keeping its current kind. Does nothing if
/// the body is currently `None` or `Form` (the body type selector must be used to switch kind
/// first).
pub fn set_body_text(request: &mut Request, text: String) {
    match &mut request.body {
        Body::Json(existing) | Body::Text(existing) | Body::Xml(existing) => *existing = text,
        Body::None | Body::Form(_) | Body::Multipart(_) => {}
    }
}

/// Whether `token` is a valid custom HTTP method for the `.brasa` file format: non-empty and
/// without a lowercase ASCII letter, the same rule `brasa_format`'s parser enforces on a request
/// line's method token (`crates/brasa-format/src/format.rs`'s `InvalidMethod` check). Not
/// imported from there directly: that check is private to the parser, and duplicating one boolean
/// condition here is simpler than exposing it as a new public API only this inline method editor
/// needs. Used by the request editor's "Custom..." method input to decide whether to commit an
/// edit.
pub fn is_valid_custom_method_token(token: &str) -> bool {
    !token.is_empty() && !token.chars().any(|c| c.is_ascii_lowercase())
}

/// Pretty-prints `text` as JSON with `indent` spaces per level (the `json_indent` setting), for
/// the request editor's "Format" button. The same formatter as the response's Pretty view, so
/// every token stays as it was typed (key order, number spellings, escapes). `None` if `text` is
/// not valid JSON (a body with `{{ }}` markers outside strings, for example), in which case the
/// caller leaves the body unchanged.
pub fn format_json_body(text: &str, indent: usize) -> Option<String> {
    super::json_pretty::pretty_print_json(text.as_bytes(), indent)
}

/// Appends a new, empty, enabled row (used for the Params, Headers and Form key-value tables).
pub fn add_row(rows: &mut Vec<KeyValue>) {
    rows.push(KeyValue::new("", ""));
}

/// Removes the row at `index`. Does nothing if `index` is out of range.
pub fn remove_row(rows: &mut Vec<KeyValue>, index: usize) {
    if index < rows.len() {
        rows.remove(index);
    }
}

// The key-value table's sort column and direction, owned by telaio's `KeyValueTable`.
pub use telaio_ui::components::{SortColumn, SortOrder};

/// Compares two rows by `column`, in the sidebar's natural order (`item2` before `item10`, case
/// ignored), with a case-sensitive tie break so the order is total.
fn compare_rows(a: &KeyValue, b: &KeyValue, column: SortColumn) -> Ordering {
    let (a, b) = match column {
        SortColumn::Key => (&a.key, &b.key),
        SortColumn::Value => (&a.value, &b.value),
    };
    natural_cmp(a, b).then_with(|| a.cmp(b))
}

/// Whether `rows` are in `order` by `column`. Equal neighbours count as sorted both ways.
pub fn is_sorted(rows: &[KeyValue], column: SortColumn, order: SortOrder) -> bool {
    let wrong = match order {
        SortOrder::Ascending => Ordering::Greater,
        SortOrder::Descending => Ordering::Less,
    };
    rows.windows(2)
        .all(|pair| compare_rows(&pair[0], &pair[1], column) != wrong)
}

/// The order a click on the header of `column` sorts by: A to Z, or Z to A when the last sort
/// (`last`) was A to Z on that same column and the rows are still in that order.
pub fn next_sort_order(
    rows: &[KeyValue],
    last: Option<(SortColumn, SortOrder)>,
    column: SortColumn,
) -> SortOrder {
    match last {
        Some((last_column, SortOrder::Ascending))
            if last_column == column && is_sorted(rows, column, SortOrder::Ascending) =>
        {
            SortOrder::Descending
        }
        _ => SortOrder::Ascending,
    }
}

/// Sorts `rows` by `column` in `order`. Stable: rows that compare equal keep their order.
pub fn sort_rows(rows: &mut [KeyValue], column: SortColumn, order: SortOrder) {
    rows.sort_by(|a, b| {
        let ordering = compare_rows(a, b, column);
        match order {
            SortOrder::Ascending => ordering,
            SortOrder::Descending => ordering.reverse(),
        }
    });
}

/// The sort arrow a table header shows: the last sort (`last`) while the rows are still in that
/// order, nothing once an edit broke it.
pub fn shown_sort(
    rows: &[KeyValue],
    last: Option<(SortColumn, SortOrder)>,
) -> Option<(SortColumn, SortOrder)> {
    last.filter(|(column, order)| is_sorted(rows, *column, *order))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn kv(key: &str, value: &str) -> KeyValue {
        KeyValue::new(key, value)
    }

    #[test]
    fn body_kind_of_matches_every_variant() {
        assert_eq!(BodyKind::of(&Body::None), BodyKind::None);
        assert_eq!(BodyKind::of(&Body::Json(String::new())), BodyKind::Json);
        assert_eq!(BodyKind::of(&Body::Text(String::new())), BodyKind::Text);
        assert_eq!(BodyKind::of(&Body::Xml(String::new())), BodyKind::Xml);
        assert_eq!(BodyKind::of(&Body::Form(Vec::new())), BodyKind::Form);
    }

    #[test]
    fn set_body_kind_preserves_text_across_raw_kinds() {
        let mut request = Request {
            body: Body::Json("{}".to_string()),
            ..Request::default()
        };
        set_body_kind(&mut request, BodyKind::Xml, &mut BodyStash::default());
        assert_eq!(request.body, Body::Xml("{}".to_string()));
    }

    #[test]
    fn set_body_kind_to_form_starts_empty_from_a_raw_body() {
        let mut request = Request {
            body: Body::Json("{}".to_string()),
            ..Request::default()
        };
        set_body_kind(&mut request, BodyKind::Form, &mut BodyStash::default());
        assert_eq!(request.body, Body::Form(Vec::new()));
    }

    #[test]
    fn set_body_kind_to_form_and_back_keeps_form_rows() {
        let mut request = Request {
            body: Body::Form(vec![kv("a", "1")]),
            ..Request::default()
        };
        set_body_kind(&mut request, BodyKind::Form, &mut BodyStash::default());
        assert_eq!(request.body, Body::Form(vec![kv("a", "1")]));
    }

    #[test]
    fn set_body_kind_to_none_drops_content() {
        let mut request = Request {
            body: Body::Text("hello".to_string()),
            ..Request::default()
        };
        set_body_kind(&mut request, BodyKind::None, &mut BodyStash::default());
        assert_eq!(request.body, Body::None);
    }

    #[test]
    fn switching_from_form_and_back_restores_the_fields() {
        let mut request = Request {
            body: Body::Form(vec![kv("a", "1"), kv("b", "2")]),
            ..Request::default()
        };
        let mut stash = BodyStash::default();
        set_body_kind(&mut request, BodyKind::Json, &mut stash);
        assert_eq!(request.body, Body::Json(String::new()));
        set_body_kind(&mut request, BodyKind::Form, &mut stash);
        assert_eq!(request.body, Body::Form(vec![kv("a", "1"), kv("b", "2")]));
    }

    #[test]
    fn switching_to_form_and_back_restores_the_text() {
        let mut request = Request {
            body: Body::Json("{\"a\": 1}".to_string()),
            ..Request::default()
        };
        let mut stash = BodyStash::default();
        set_body_kind(&mut request, BodyKind::Form, &mut stash);
        set_body_kind(&mut request, BodyKind::Text, &mut stash);
        assert_eq!(request.body, Body::Text("{\"a\": 1}".to_string()));
    }

    #[test]
    fn switching_to_none_and_back_restores_the_text() {
        let mut request = Request {
            body: Body::Xml("<a/>".to_string()),
            ..Request::default()
        };
        let mut stash = BodyStash::default();
        set_body_kind(&mut request, BodyKind::None, &mut stash);
        assert_eq!(request.body, Body::None);
        set_body_kind(&mut request, BodyKind::Xml, &mut stash);
        assert_eq!(request.body, Body::Xml("<a/>".to_string()));
    }

    #[test]
    fn the_stash_follows_the_latest_edit() {
        let mut request = Request {
            body: Body::Form(vec![kv("a", "1")]),
            ..Request::default()
        };
        let mut stash = BodyStash::default();
        set_body_kind(&mut request, BodyKind::Json, &mut stash);
        set_body_kind(&mut request, BodyKind::Form, &mut stash);
        request.body = Body::Form(vec![kv("b", "2")]);
        set_body_kind(&mut request, BodyKind::None, &mut stash);
        set_body_kind(&mut request, BodyKind::Form, &mut stash);
        assert_eq!(request.body, Body::Form(vec![kv("b", "2")]));
    }

    #[test]
    fn set_body_text_updates_a_raw_body() {
        let mut request = Request {
            body: Body::Json("{}".to_string()),
            ..Request::default()
        };
        set_body_text(&mut request, "{\"a\":1}".to_string());
        assert_eq!(request.body, Body::Json("{\"a\":1}".to_string()));
    }

    #[test]
    fn set_body_text_does_nothing_for_none_or_form() {
        let mut request = Request::default();
        set_body_text(&mut request, "ignored".to_string());
        assert_eq!(request.body, Body::None);
    }

    #[test]
    fn format_json_body_pretty_prints_valid_json() {
        assert_eq!(
            format_json_body(r#"{"a":1,"b":[1,2]}"#, 2),
            Some("{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ]\n}".to_string())
        );
    }

    #[test]
    fn format_json_body_uses_the_given_indentation_and_keeps_tokens() {
        assert_eq!(
            format_json_body(r#"{"z":1.0e3,"a":"\u00e9"}"#, 4),
            Some("{\n    \"z\": 1.0e3,\n    \"a\": \"\\u00e9\"\n}".to_string())
        );
    }

    #[test]
    fn format_json_body_rejects_invalid_json() {
        assert_eq!(format_json_body("not json", 2), None);
    }

    #[test]
    fn is_valid_custom_method_token_accepts_an_uppercase_token() {
        assert!(is_valid_custom_method_token("PURGE"));
        assert!(is_valid_custom_method_token("X-CUSTOM2"));
    }

    #[test]
    fn is_valid_custom_method_token_rejects_empty() {
        assert!(!is_valid_custom_method_token(""));
    }

    #[test]
    fn is_valid_custom_method_token_rejects_lowercase() {
        assert!(!is_valid_custom_method_token("purge"));
        assert!(!is_valid_custom_method_token("Purge"));
    }

    #[test]
    fn add_row_appends_an_empty_enabled_row() {
        let mut rows = vec![kv("a", "1")];
        add_row(&mut rows);
        assert_eq!(rows, vec![kv("a", "1"), KeyValue::new("", "")]);
        assert!(rows[1].enabled);
    }

    #[test]
    fn remove_row_removes_the_given_index() {
        let mut rows = vec![kv("a", "1"), kv("b", "2")];
        remove_row(&mut rows, 0);
        assert_eq!(rows, vec![kv("b", "2")]);
    }

    #[test]
    fn remove_row_out_of_range_does_nothing() {
        let mut rows = vec![kv("a", "1")];
        remove_row(&mut rows, 5);
        assert_eq!(rows, vec![kv("a", "1")]);
    }

    fn keys(rows: &[KeyValue]) -> Vec<&str> {
        rows.iter().map(|row| row.key.as_str()).collect()
    }

    #[test]
    fn sort_rows_uses_the_natural_order_and_ignores_case() {
        let mut rows = vec![
            kv("item10", ""),
            kv("Banana", ""),
            kv("item2", ""),
            kv("apple", ""),
        ];
        sort_rows(&mut rows, SortColumn::Key, SortOrder::Ascending);
        assert_eq!(keys(&rows), ["apple", "Banana", "item2", "item10"]);
        sort_rows(&mut rows, SortColumn::Key, SortOrder::Descending);
        assert_eq!(keys(&rows), ["item10", "item2", "Banana", "apple"]);
    }

    #[test]
    fn sort_rows_by_value_moves_whole_rows() {
        let mut rows = vec![kv("a", "3"), kv("b", "1"), kv("c", "2")];
        sort_rows(&mut rows, SortColumn::Value, SortOrder::Ascending);
        assert_eq!(rows, vec![kv("b", "1"), kv("c", "2"), kv("a", "3")]);
    }

    #[test]
    fn a_click_sorts_a_to_z_then_z_to_a() {
        let rows = vec![kv("b", ""), kv("a", "")];
        assert_eq!(
            next_sort_order(&rows, None, SortColumn::Key),
            SortOrder::Ascending
        );
        let sorted = vec![kv("a", ""), kv("b", "")];
        let last = Some((SortColumn::Key, SortOrder::Ascending));
        assert_eq!(
            next_sort_order(&sorted, last, SortColumn::Key),
            SortOrder::Descending
        );
        // Another column, or rows edited out of order, start again from A to Z.
        assert_eq!(
            next_sort_order(&sorted, last, SortColumn::Value),
            SortOrder::Ascending
        );
        assert_eq!(
            next_sort_order(&rows, last, SortColumn::Key),
            SortOrder::Ascending
        );
        let last = Some((SortColumn::Key, SortOrder::Descending));
        assert_eq!(
            next_sort_order(&rows, last, SortColumn::Key),
            SortOrder::Ascending
        );
    }

    #[test]
    fn the_arrow_shows_until_an_edit_breaks_the_order() {
        let last = Some((SortColumn::Key, SortOrder::Ascending));
        let sorted = vec![kv("a", ""), kv("b", "")];
        assert_eq!(shown_sort(&sorted, last), last);
        let edited = vec![kv("c", ""), kv("b", "")];
        assert_eq!(shown_sort(&edited, last), None);
        assert_eq!(shown_sort(&sorted, None), None);
    }
}
