//! A semantic diff of two versions of a `.brasa` file, grouped by part of the request.
//!
//! [`diff_requests`] parses both versions and compares them part by part (method and URL,
//! headers, query, auth, body, scripts, docs) instead of line by line, for the "Request" mode of
//! the diff tab. Pure, no IO. Every row's text reads exactly as the line would in the file, so the
//! caller can highlight it like the file. Either side that does not parse is an error, and the
//! caller falls back to the text modes.

use std::collections::HashMap;

use brasa_core::{Body, KeyValue, Request};
use serde_json::Value;

use crate::format::{ParseError, build_sections, format_query_or_form_line, parse};

/// Above this many cells, the line diff skips the LCS table and treats the differing middle of
/// both texts as fully removed and fully added, so a huge file cannot exhaust memory.
const MAX_LCS_CELLS: usize = 25_000_000;

/// The comparison of two versions of a request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RequestDiff {
    /// The parts with at least one difference, in file order.
    pub changed: Vec<PartDiff>,
    /// The parts present on either side that did not change, in file order. Parts absent on both
    /// sides are left out.
    pub unchanged: Vec<RequestPart>,
}

/// A part of a request, in the order a `.brasa` file writes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestPart {
    /// The method and the URL (the request line).
    Request,
    /// The header block.
    Headers,
    /// The `::: query` section.
    Query,
    /// The `::: auth` section.
    Auth,
    /// The `::: body` section.
    Body,
    /// The `::: pre` section.
    PreScript,
    /// The `::: post` section.
    PostScript,
    /// The `::: docs` section.
    Docs,
}

/// The differences of one part of the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartDiff {
    /// Which part this is.
    pub part: RequestPart,
    /// The rows to show, never empty.
    pub rows: Vec<DiffRow>,
    /// The counts shown next to the part's name.
    pub summary: PartSummary,
}

/// How many entries of a part were added, removed or changed. An entry whose value or enabled
/// state changed counts once in `changed`, not in `added` and `removed`. Parts compared line by
/// line only use `added` and `removed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PartSummary {
    /// Entries only in the new version.
    pub added: usize,
    /// Entries only in the old version.
    pub removed: usize,
    /// Entries in both versions with a different value or enabled state.
    pub changed: usize,
}

/// What a row means for the diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRowKind {
    /// The text is only in the new version.
    Added,
    /// The text is only in the old version.
    Removed,
    /// The text is in both versions. Reserved for context lines, [`diff_requests`] does not
    /// produce it today.
    Unchanged,
}

/// One row of a [`PartDiff`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    /// Whether the text was added or removed.
    pub kind: DiffRowKind,
    /// The text as the line reads in the file (`Name: value`, `key = value`, `"path": value`, a
    /// script line, ...).
    pub text: String,
    /// A short label for the row, translated by the app.
    pub note: Option<RowNote>,
}

/// A short machine value that explains a row, translated by the app. A changed entry is a
/// removed row followed by an added row, and only the added row carries the `Changed` note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowNote {
    /// A header only in the new version.
    NewHeader,
    /// A header only in the old version.
    RemovedHeader,
    /// A header whose value or enabled state changed.
    ChangedHeader,
    /// A query, form or JSON field only in the new version.
    NewField,
    /// A query, form or JSON field only in the old version.
    RemovedField,
    /// A query, form or JSON field whose value or enabled state changed.
    ChangedField,
}

/// Why two versions could not be compared: one side does not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestDiffError {
    /// The old version does not parse.
    #[error("the old version does not parse: {0}")]
    Old(ParseError),
    /// The new version does not parse.
    #[error("the new version does not parse: {0}")]
    New(ParseError),
}

/// Compares two versions of a `.brasa` file by part of the request.
///
/// `old` is `None` when the file is new (everything is added) and `new` is `None` when it was
/// deleted (everything is removed). A side that does not parse is an error.
///
/// Headers, query and form fields are compared as ordered key/value lists, including the
/// disabled state; the same key repeated is matched in order. JSON bodies that both parse are
/// compared by field path; any other body, the scripts, the docs and the auth are compared line by
/// line.
pub fn diff_requests(
    old: Option<&str>,
    new: Option<&str>,
) -> Result<RequestDiff, RequestDiffError> {
    let old = old
        .map(|text| parse(text).map_err(RequestDiffError::Old))
        .transpose()?;
    let new = new
        .map(|text| parse(text).map_err(RequestDiffError::New))
        .transpose()?;
    let empty = Request::default();
    let (old_request, new_request) = (
        old.as_ref().unwrap_or(&empty),
        new.as_ref().unwrap_or(&empty),
    );
    let old_sections = build_sections(old_request);
    let new_sections = build_sections(new_request);

    let mut diff = RequestDiff::default();
    let mut record = |part: RequestPart, present: bool, rows: Rows| {
        if !rows.rows.is_empty() {
            diff.changed.push(PartDiff {
                part,
                rows: rows.rows,
                summary: rows.summary,
            });
        } else if present {
            diff.unchanged.push(part);
        }
    };

    record(
        RequestPart::Request,
        true,
        request_line_rows(old.as_ref(), new.as_ref()),
    );
    record(
        RequestPart::Headers,
        !old_request.headers.is_empty() || !new_request.headers.is_empty(),
        key_value_rows(
            &old_request.headers,
            &new_request.headers,
            header_text,
            Kind::Header,
        ),
    );
    record(
        RequestPart::Query,
        !old_request.query.is_empty() || !new_request.query.is_empty(),
        key_value_rows(
            &old_request.query,
            &new_request.query,
            format_query_or_form_line,
            Kind::Field,
        ),
    );
    record(
        RequestPart::Auth,
        old_request.auth != brasa_core::Auth::None
            || new_request.auth != brasa_core::Auth::None,
        line_rows(
            &section_lines(&old_sections, "::: auth"),
            &section_lines(&new_sections, "::: auth"),
        ),
    );
    record(
        RequestPart::Body,
        old_request.body != Body::None || new_request.body != Body::None,
        body_rows(
            &old_request.body,
            &new_request.body,
            &old_sections,
            &new_sections,
        ),
    );
    for (part, old_text, new_text) in [
        (
            RequestPart::PreScript,
            &old_request.pre_script,
            &new_request.pre_script,
        ),
        (
            RequestPart::PostScript,
            &old_request.post_script,
            &new_request.post_script,
        ),
        (RequestPart::Docs, &old_request.docs, &new_request.docs),
    ] {
        record(
            part,
            !old_text.is_empty() || !new_text.is_empty(),
            line_rows(&text_lines(old_text), &text_lines(new_text)),
        );
    }
    Ok(diff)
}

/// The rows of a part under construction, with their summary kept in step.
#[derive(Default)]
struct Rows {
    rows: Vec<DiffRow>,
    summary: PartSummary,
}

/// Which notes the key/value rows carry.
#[derive(Clone, Copy)]
enum Kind {
    Header,
    Field,
}

impl Rows {
    fn push(&mut self, kind: DiffRowKind, text: String, note: Option<RowNote>) {
        self.rows.push(DiffRow { kind, text, note });
    }

    fn added(&mut self, text: String, note: Option<RowNote>) {
        self.summary.added += 1;
        self.push(DiffRowKind::Added, text, note);
    }

    fn removed(&mut self, text: String, note: Option<RowNote>) {
        self.summary.removed += 1;
        self.push(DiffRowKind::Removed, text, note);
    }

    /// A removed row followed by an added row, counted as one change.
    fn changed(&mut self, old: String, new: String, note: RowNote) {
        self.summary.changed += 1;
        self.push(DiffRowKind::Removed, old, None);
        self.push(DiffRowKind::Added, new, Some(note));
    }
}

/// The method and URL: the old `METHOD url` removed and the new one added when either differs.
fn request_line_rows(old: Option<&Request>, new: Option<&Request>) -> Rows {
    let mut rows = Rows::default();
    let line = |request: &Request| format!("{} {}", request.method, request.url);
    match (old, new) {
        (Some(old), Some(new)) if old.method != new.method || old.url != new.url => {
            rows.removed(line(old), None);
            rows.added(line(new), None);
        }
        (Some(old), None) => rows.removed(line(old), None),
        (None, Some(new)) => rows.added(line(new), None),
        _ => {}
    }
    rows
}

/// A header as its line in the file, with `# ` when disabled.
fn header_text(entry: &KeyValue) -> String {
    let prefix = if entry.enabled { "" } else { "# " };
    format!("{prefix}{}: {}", entry.key, entry.value)
}

/// Compares two ordered key/value lists. The n-th entry with a key matches the n-th entry with
/// that key on the other side. Rows follow the order of the files: removed entries are written
/// before the first matched entry that comes after them in the old list.
fn key_value_rows(
    old: &[KeyValue],
    new: &[KeyValue],
    text: fn(&KeyValue) -> String,
    kind: Kind,
) -> Rows {
    let (new_note, removed_note, changed_note) = match kind {
        Kind::Header => (
            RowNote::NewHeader,
            RowNote::RemovedHeader,
            RowNote::ChangedHeader,
        ),
        Kind::Field => (
            RowNote::NewField,
            RowNote::RemovedField,
            RowNote::ChangedField,
        ),
    };

    let mut old_index: HashMap<(&str, usize), usize> = HashMap::new();
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (position, entry) in old.iter().enumerate() {
        let nth = seen.entry(entry.key.as_str()).or_insert(0);
        old_index.insert((entry.key.as_str(), *nth), position);
        *nth += 1;
    }

    let mut rows = Rows::default();
    let mut matched = vec![false; old.len()];
    let mut flushed = 0;
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for entry in new {
        let nth = seen.entry(entry.key.as_str()).or_insert(0);
        let found = old_index.get(&(entry.key.as_str(), *nth)).copied();
        *nth += 1;
        let Some(position) = found else {
            rows.added(text(entry), Some(new_note));
            continue;
        };
        matched[position] = true;
        while flushed < position {
            if !matched[flushed] && !is_matched_later(old, flushed, new) {
                rows.removed(text(&old[flushed]), Some(removed_note));
            }
            flushed += 1;
        }
        let before = &old[position];
        if before.value != entry.value || before.enabled != entry.enabled {
            rows.changed(text(before), text(entry), changed_note);
        }
    }
    for (position, entry) in old.iter().enumerate().skip(flushed) {
        if !matched[position] && !is_matched_later(old, position, new) {
            rows.removed(text(entry), Some(removed_note));
        }
    }
    rows
}

/// Whether the old entry at `position` has a counterpart in `new`, which has not been reached yet
/// when it is asked about during the walk of `new`.
fn is_matched_later(old: &[KeyValue], position: usize, new: &[KeyValue]) -> bool {
    let key = old[position].key.as_str();
    let nth = old[..position].iter().filter(|e| e.key == key).count();
    new.iter().filter(|e| e.key == key).count() > nth
}

/// The lines of a raw text, none when it is empty.
fn text_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(str::to_string).collect()
    }
}

/// The heading and the content of the section whose heading starts with `prefix`, as lines.
fn section_lines(sections: &[(String, Vec<String>)], prefix: &str) -> Vec<String> {
    sections
        .iter()
        .find(|(heading, _)| heading.starts_with(prefix))
        .map(|(heading, content)| {
            std::iter::once(heading.clone())
                .chain(content.iter().cloned())
                .collect()
        })
        .unwrap_or_default()
}

/// The body: by field path when both sides are JSON that parses, as key/value fields when both
/// are forms, line by line (heading included, so a changed kind shows) otherwise.
fn body_rows(
    old: &Body,
    new: &Body,
    old_sections: &[(String, Vec<String>)],
    new_sections: &[(String, Vec<String>)],
) -> Rows {
    match (old, new) {
        (Body::Json(old), Body::Json(new)) => {
            if let (Ok(old), Ok(new)) = (
                serde_json::from_str::<Value>(old),
                serde_json::from_str::<Value>(new),
            ) {
                return json_rows(&old, &new);
            }
        }
        (Body::Form(old), Body::Form(new)) | (Body::Multipart(old), Body::Multipart(new)) => {
            return key_value_rows(old, new, format_query_or_form_line, Kind::Field);
        }
        _ => {}
    }
    line_rows(
        &section_lines(old_sections, "::: body"),
        &section_lines(new_sections, "::: body"),
    )
}

/// Compares two JSON values by the path of their leaves.
fn json_rows(old: &Value, new: &Value) -> Rows {
    let (mut old_leaves, mut new_leaves) = (Vec::new(), Vec::new());
    flatten(old, String::new(), &mut old_leaves);
    flatten(new, String::new(), &mut new_leaves);
    let old_by_path: HashMap<&str, &Value> =
        old_leaves.iter().map(|(p, v)| (p.as_str(), v)).collect();
    let new_by_path: HashMap<&str, &Value> =
        new_leaves.iter().map(|(p, v)| (p.as_str(), v)).collect();

    let mut rows = Rows::default();
    for (path, value) in &new_leaves {
        match old_by_path.get(path.as_str()) {
            None => rows.added(leaf_text(path, value), Some(RowNote::NewField)),
            Some(before) if *before != value => rows.changed(
                leaf_text(path, before),
                leaf_text(path, value),
                RowNote::ChangedField,
            ),
            Some(_) => {}
        }
    }
    for (path, value) in &old_leaves {
        if !new_by_path.contains_key(path.as_str()) {
            rows.removed(leaf_text(path, value), Some(RowNote::RemovedField));
        }
    }
    rows
}

/// Collects the leaves of a JSON value as `(path, value)`, like `user.name` or `items[2]`. Empty
/// objects and arrays are leaves.
fn flatten(value: &Value, path: String, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                flatten(child, child_path, out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, child) in items.iter().enumerate() {
                flatten(child, format!("{path}[{index}]"), out);
            }
        }
        _ => out.push((path, value.clone())),
    }
}

/// A leaf as `"path": value`, or just the value for a body that is a bare scalar.
fn leaf_text(path: &str, value: &Value) -> String {
    if path.is_empty() {
        value.to_string()
    } else {
        format!("\"{path}\": {value}")
    }
}

/// A line diff by longest common subsequence: only added and removed rows, no context.
fn line_rows(old: &[String], new: &[String]) -> Rows {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old = &old[prefix..old.len() - suffix];
    let new = &new[prefix..new.len() - suffix];

    let mut rows = Rows::default();
    if old.len().saturating_mul(new.len()) > MAX_LCS_CELLS {
        old.iter().for_each(|line| rows.removed(line.clone(), None));
        new.iter().for_each(|line| rows.added(line.clone(), None));
        return rows;
    }

    // lengths[i][j] is the LCS length of old[i..] and new[j..].
    let width = new.len() + 1;
    let mut lengths = vec![0u32; (old.len() + 1) * width];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lengths[i * width + j] = if old[i] == new[j] {
                lengths[(i + 1) * width + j + 1] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            i += 1;
            j += 1;
        } else if i < old.len()
            && (j == new.len() || lengths[(i + 1) * width + j] >= lengths[i * width + j + 1])
        {
            rows.removed(old[i].clone(), None);
            i += 1;
        } else {
            rows.added(new[j].clone(), None);
            j += 1;
        }
    }
    rows
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn diff(old: &str, new: &str) -> RequestDiff {
        diff_requests(Some(old), Some(new)).expect("both sides parse")
    }

    fn part(diff: &RequestDiff, part: RequestPart) -> &PartDiff {
        diff.changed
            .iter()
            .find(|p| p.part == part)
            .unwrap_or_else(|| panic!("{part:?} should have changed"))
    }

    fn texts(part: &PartDiff) -> Vec<(DiffRowKind, &str)> {
        part.rows
            .iter()
            .map(|r| (r.kind, r.text.as_str()))
            .collect()
    }

    use DiffRowKind::{Added, Removed};

    #[test]
    fn no_changes_lists_present_parts_as_unchanged() {
        let text = "GET https://a.test\nAccept: x\n\n::: query\nq = 1\n\n::: pre\nlog();\n";
        let result = diff(text, text);
        assert!(result.changed.is_empty());
        assert_eq!(
            result.unchanged,
            vec![
                RequestPart::Request,
                RequestPart::Headers,
                RequestPart::Query,
                RequestPart::PreScript
            ]
        );
    }

    #[test]
    fn method_and_url_change() {
        let result = diff("GET https://a.test/1\n", "POST https://a.test/2\n");
        let request = part(&result, RequestPart::Request);
        assert_eq!(
            texts(request),
            vec![
                (Removed, "GET https://a.test/1"),
                (Added, "POST https://a.test/2")
            ]
        );
        assert!(result.unchanged.is_empty());
    }

    #[test]
    fn only_the_method_changes() {
        let result = diff("GET https://a.test\n", "PUT https://a.test\n");
        assert_eq!(part(&result, RequestPart::Request).rows.len(), 2);
    }

    #[test]
    fn header_added_removed_and_changed() {
        let result = diff("GET u\nA: 1\nB: 2\nC: 3\n", "GET u\nA: 1\nB: 20\nD: 4\n");
        let headers = part(&result, RequestPart::Headers);
        assert_eq!(
            texts(headers),
            vec![
                (Removed, "B: 2"),
                (Added, "B: 20"),
                (Added, "D: 4"),
                (Removed, "C: 3")
            ]
        );
        assert_eq!(headers.rows[1].note, Some(RowNote::ChangedHeader));
        assert_eq!(headers.rows[0].note, None);
        assert_eq!(headers.rows[2].note, Some(RowNote::NewHeader));
        assert_eq!(headers.rows[3].note, Some(RowNote::RemovedHeader));
        assert_eq!(
            headers.summary,
            PartSummary {
                added: 1,
                removed: 1,
                changed: 1
            }
        );
        assert_eq!(result.unchanged, vec![RequestPart::Request]);
    }

    #[test]
    fn disabling_a_header_is_a_change() {
        let result = diff("GET u\nA: 1\n", "GET u\n# A: 1\n");
        let headers = part(&result, RequestPart::Headers);
        assert_eq!(texts(headers), vec![(Removed, "A: 1"), (Added, "# A: 1")]);
        assert_eq!(headers.summary.changed, 1);
    }

    #[test]
    fn removed_header_keeps_file_order() {
        let result = diff("GET u\nA: 1\nB: 2\nC: 3\n", "GET u\nA: 1\nC: 33\n");
        let headers = part(&result, RequestPart::Headers);
        assert_eq!(
            texts(headers),
            vec![(Removed, "B: 2"), (Removed, "C: 3"), (Added, "C: 33")]
        );
    }

    #[test]
    fn duplicate_headers_match_in_order() {
        let result = diff("GET u\nX: 1\nX: 2\n", "GET u\nX: 1\nX: 3\nX: 4\n");
        let headers = part(&result, RequestPart::Headers);
        assert_eq!(
            texts(headers),
            vec![(Removed, "X: 2"), (Added, "X: 3"), (Added, "X: 4")]
        );
    }

    #[test]
    fn all_headers_added_to_a_request_without_any() {
        let result = diff("GET u\n", "GET u\nA: 1\n");
        let headers = part(&result, RequestPart::Headers);
        assert_eq!(texts(headers), vec![(Added, "A: 1")]);
        assert_eq!(headers.rows[0].note, Some(RowNote::NewHeader));
    }

    #[test]
    fn query_entries() {
        let result = diff(
            "GET u\n\n::: query\npage = 1\nsort = asc\n# debug = on\n",
            "GET u\n\n::: query\npage = 2\n# sort = asc\nlimit = 10\n",
        );
        let query = part(&result, RequestPart::Query);
        assert_eq!(
            texts(query),
            vec![
                (Removed, "page = 1"),
                (Added, "page = 2"),
                (Removed, "sort = asc"),
                (Added, "# sort = asc"),
                (Added, "limit = 10"),
                (Removed, "# debug = on"),
            ]
        );
        assert_eq!(query.rows[1].note, Some(RowNote::ChangedField));
        assert_eq!(query.rows[4].note, Some(RowNote::NewField));
        assert_eq!(query.rows[5].note, Some(RowNote::RemovedField));
        assert_eq!(
            query.summary,
            PartSummary {
                added: 1,
                removed: 1,
                changed: 2
            }
        );
    }

    #[test]
    fn auth_is_compared_with_its_heading() {
        let result = diff(
            "GET u\n\n::: auth bearer\ntoken = a\n",
            "GET u\n\n::: auth bearer\ntoken = b\n",
        );
        let auth = part(&result, RequestPart::Auth);
        assert_eq!(
            texts(auth),
            vec![(Removed, "token = a"), (Added, "token = b")]
        );

        let result = diff(
            "GET u\n\n::: auth bearer\ntoken = a\n",
            "GET u\n\n::: auth basic\nusername = x\n",
        );
        let auth = part(&result, RequestPart::Auth);
        assert_eq!(
            texts(auth),
            vec![
                (Removed, "::: auth bearer"),
                (Removed, "token = a"),
                (Added, "::: auth basic"),
                (Added, "username = x"),
            ]
        );
    }

    #[test]
    fn auth_added_and_removed() {
        let result = diff("GET u\n", "GET u\n\n::: auth bearer\ntoken = a\n");
        assert_eq!(
            texts(part(&result, RequestPart::Auth)),
            vec![(Added, "::: auth bearer"), (Added, "token = a")]
        );
    }

    #[test]
    fn json_body_fields_by_path() {
        let old = "POST u\n\n::: body json\n{\"user\": {\"name\": \"a\", \"age\": 3}, \"items\": [1, 2, 3], \"gone\": true}\n";
        let new = "POST u\n\n::: body json\n{\n  \"user\": {\"name\": \"b\", \"age\": 3},\n  \"items\": [1, 5],\n  \"tag\": null\n}\n";
        let result = diff(old, new);
        let body = part(&result, RequestPart::Body);
        let rows = texts(body);
        assert!(rows.contains(&(Removed, "\"user.name\": \"a\"")));
        assert!(rows.contains(&(Added, "\"user.name\": \"b\"")));
        assert!(rows.contains(&(Removed, "\"items[1]\": 2")));
        assert!(rows.contains(&(Added, "\"items[1]\": 5")));
        assert!(rows.contains(&(Removed, "\"items[2]\": 3")));
        assert!(rows.contains(&(Removed, "\"gone\": true")));
        assert!(rows.contains(&(Added, "\"tag\": null")));
        assert!(!rows.iter().any(|(_, t)| t.contains("age")));
        assert_eq!(
            body.summary,
            PartSummary {
                added: 1,
                removed: 2,
                changed: 2
            }
        );
        let tag = body.rows.iter().find(|r| r.text.contains("tag")).unwrap();
        assert_eq!(tag.note, Some(RowNote::NewField));
        let gone = body.rows.iter().find(|r| r.text.contains("gone")).unwrap();
        assert_eq!(gone.note, Some(RowNote::RemovedField));
    }

    #[test]
    fn json_reformatting_is_not_a_change() {
        let result = diff(
            "POST u\n\n::: body json\n{\"a\":1,\"b\":[1,2]}\n",
            "POST u\n\n::: body json\n{\n  \"a\": 1,\n  \"b\": [1, 2]\n}\n",
        );
        assert!(result.changed.is_empty());
        assert_eq!(
            result.unchanged,
            vec![RequestPart::Request, RequestPart::Body]
        );
    }

    #[test]
    fn json_value_type_change_and_empty_containers() {
        let result = diff(
            "POST u\n\n::: body json\n{\"a\": 1, \"b\": {}}\n",
            "POST u\n\n::: body json\n{\"a\": \"1\", \"b\": []}\n",
        );
        let body = part(&result, RequestPart::Body);
        assert_eq!(
            texts(body),
            vec![
                (Removed, "\"a\": 1"),
                (Added, "\"a\": \"1\""),
                (Removed, "\"b\": {}"),
                (Added, "\"b\": []"),
            ]
        );
    }

    #[test]
    fn json_with_placeholders_falls_back_to_lines() {
        let result = diff(
            "POST u\n\n::: body json\n{\n  \"id\": {{id}}\n}\n",
            "POST u\n\n::: body json\n{\n  \"id\": {{other}}\n}\n",
        );
        assert_eq!(
            texts(part(&result, RequestPart::Body)),
            vec![
                (Removed, "  \"id\": {{id}}"),
                (Added, "  \"id\": {{other}}")
            ]
        );
    }

    #[test]
    fn text_body_lines() {
        let result = diff(
            "POST u\n\n::: body text\none\ntwo\nthree\n",
            "POST u\n\n::: body text\none\n2\nthree\nfour\n",
        );
        assert_eq!(
            texts(part(&result, RequestPart::Body)),
            vec![(Removed, "two"), (Added, "2"), (Added, "four")]
        );
    }

    #[test]
    fn form_body_fields() {
        let result = diff(
            "POST u\n\n::: body form\na = 1\nb = 2\n",
            "POST u\n\n::: body form\na = 1\nb = 3\n",
        );
        let body = part(&result, RequestPart::Body);
        assert_eq!(texts(body), vec![(Removed, "b = 2"), (Added, "b = 3")]);
        assert_eq!(body.rows[1].note, Some(RowNote::ChangedField));
    }

    #[test]
    fn body_kind_change_shows_the_headings() {
        let result = diff(
            "POST u\n\n::: body json\n{\"a\": 1}\n",
            "POST u\n\n::: body xml\n<a/>\n",
        );
        assert_eq!(
            texts(part(&result, RequestPart::Body)),
            vec![
                (Removed, "::: body json"),
                (Removed, "{\"a\": 1}"),
                (Added, "::: body xml"),
                (Added, "<a/>"),
            ]
        );
    }

    #[test]
    fn scripts_and_docs_are_line_diffs() {
        let result = diff(
            "GET u\n\n::: pre\na();\nb();\n\n::: post\ntest();\n\n::: docs\nHello\n",
            "GET u\n\n::: pre\na();\nc();\nb();\n\n::: post\ntest();\n\n::: docs\nHello\nWorld\n",
        );
        assert_eq!(
            texts(part(&result, RequestPart::PreScript)),
            vec![(Added, "c();")]
        );
        assert_eq!(
            texts(part(&result, RequestPart::Docs)),
            vec![(Added, "World")]
        );
        assert_eq!(
            part(&result, RequestPart::Docs).summary,
            PartSummary {
                added: 1,
                removed: 0,
                changed: 0
            }
        );
        assert_eq!(
            result.unchanged,
            vec![RequestPart::Request, RequestPart::PostScript]
        );
        assert_eq!(
            result.changed.iter().map(|p| p.part).collect::<Vec<_>>(),
            vec![RequestPart::PreScript, RequestPart::Docs]
        );
    }

    #[test]
    fn script_removed_entirely() {
        let result = diff("GET u\n\n::: post\na();\nb();\n", "GET u\n");
        assert_eq!(
            texts(part(&result, RequestPart::PostScript)),
            vec![(Removed, "a();"), (Removed, "b();")]
        );
    }

    #[test]
    fn line_diff_prefers_the_longest_common_subsequence() {
        let rows = line_rows(&text_lines("a\nb\nc\nd"), &text_lines("b\nc\nd\na"));
        assert_eq!(
            rows.rows
                .iter()
                .map(|r| (r.kind, r.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(Removed, "a"), (Added, "a")]
        );
    }

    #[test]
    fn new_file_adds_everything() {
        let result = diff_requests(
            None,
            Some("POST u\nA: 1\n\n::: query\nq = 1\n\n::: body json\n{\"a\": 1}\n"),
        )
        .unwrap();
        assert_eq!(
            texts(part(&result, RequestPart::Request)),
            vec![(Added, "POST u")]
        );
        assert_eq!(
            texts(part(&result, RequestPart::Headers)),
            vec![(Added, "A: 1")]
        );
        assert_eq!(
            texts(part(&result, RequestPart::Query)),
            vec![(Added, "q = 1")]
        );
        assert_eq!(
            texts(part(&result, RequestPart::Body)),
            vec![(Added, "::: body json"), (Added, "{\"a\": 1}")]
        );
        assert!(result.unchanged.is_empty());
    }

    #[test]
    fn deleted_file_removes_everything() {
        let result = diff_requests(Some("GET u\nA: 1\n\n::: docs\nhi\n"), None).unwrap();
        assert_eq!(
            texts(part(&result, RequestPart::Request)),
            vec![(Removed, "GET u")]
        );
        assert_eq!(
            texts(part(&result, RequestPart::Headers)),
            vec![(Removed, "A: 1")]
        );
        assert_eq!(
            texts(part(&result, RequestPart::Docs)),
            vec![(Removed, "hi")]
        );
        assert!(result.unchanged.is_empty());
    }

    #[test]
    fn both_sides_absent_is_an_empty_diff() {
        let result = diff_requests(None, None).unwrap();
        assert_eq!(result.changed, Vec::new());
        assert_eq!(result.unchanged, vec![RequestPart::Request]);
    }

    #[test]
    fn unparseable_side_is_an_error() {
        assert!(matches!(
            diff_requests(Some("not a request"), Some("GET u\n")),
            Err(RequestDiffError::Old(_))
        ));
        assert!(matches!(
            diff_requests(Some("GET u\n"), Some("")),
            Err(RequestDiffError::New(_))
        ));
        assert!(matches!(
            diff_requests(None, Some("GET\n")),
            Err(RequestDiffError::New(_))
        ));
        assert!(
            RequestDiffError::New(parse("").unwrap_err())
                .to_string()
                .starts_with("the new version does not parse")
        );
    }
}
