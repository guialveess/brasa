//! Pure formatting helpers for the response viewer (`views/response_view.rs`): pretty-printing the
//! body and formatting size and time. Status coloring now comes from
//! [`crate::theme::Palette::status_colors`], which classifies a status code the same way this
//! module used to.
//!
//! Kept free of `gpui` types so every rule is unit-tested directly.

use std::sync::Arc;
use std::time::{Duration, Instant};

use brasa_core::{ResolvedField, TemplateWarning};

pub use super::html_pretty::pretty_print_html;
use super::locale;
use super::number::{format_decimal, format_integer};

/// The standard reason phrase for a status code (`"OK"` for 200, `"Not Found"` for 404, ...),
/// for the response status badge label ("200 OK", "404 Not Found"). `brasa_core::Response` carries no reason phrase from the wire
/// (`ureq`/the `http` crate normalize away the raw status line), so this is always the standard
/// phrase for the code; a code this table does not cover returns `None`, and the caller then
/// shows the bare code, as before.
pub fn reason_phrase(status: u16) -> Option<&'static str> {
    Some(match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => return None,
    })
}

/// One row for the response pane's warnings strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarningGroup {
    /// A single unknown variable.
    UnknownVariable(String),
    /// More than one unknown variable in the same run: every name, deduplicated, in order of
    /// first appearance. Shown as one row instead of one row per variable.
    UnknownVariables(Vec<String>),
    /// A function-call error's own message.
    Function(String),
}

/// Groups a run's [`TemplateWarning`]s into display rows: every unknown variable name,
/// deduplicated regardless of how many times or where it appeared, becomes a single
/// [`WarningGroup::UnknownVariable`] or [`WarningGroup::UnknownVariables`] row (singular only
/// when there is exactly one), listed before one [`WarningGroup::Function`] row per
/// function-call error, in order of appearance.
pub fn group_warnings(warnings: &[TemplateWarning]) -> Vec<WarningGroup> {
    let mut unknown_names = Vec::new();
    let mut function_groups = Vec::new();
    for warning in warnings {
        match warning {
            TemplateWarning::UnknownVariable(name) => {
                if !unknown_names.contains(name) {
                    unknown_names.push(name.clone());
                }
            }
            TemplateWarning::Function(_) => {
                function_groups.push(WarningGroup::Function(warning.to_string()));
            }
        }
    }
    let unknown_group = match unknown_names.len() {
        0 => None,
        1 => Some(WarningGroup::UnknownVariable(unknown_names.remove(0))),
        _ => Some(WarningGroup::UnknownVariables(unknown_names)),
    };
    unknown_group.into_iter().chain(function_groups).collect()
}

pub use super::json_pretty::pretty_print_json;

/// Renders `body` as text for the raw view: valid UTF-8 shown as is, otherwise a short
/// placeholder noting the byte count (there is no binary/hex viewer).
pub fn body_as_text(body: &[u8]) -> String {
    match std::str::from_utf8(body) {
        Ok(text) => text.to_string(),
        Err(_) => locale::plural("response.body.not_utf8", body.len()),
    }
}

/// Counts the responses stored per tab, so a cheap `(tab id, generation)` key identifies "this
/// exact response" without looking at the body.
#[derive(Debug, Default)]
pub struct ResponseGenerations {
    next: u64,
    by_tab: std::collections::HashMap<String, u64>,
}

impl ResponseGenerations {
    /// Records that a new response was stored for `tab_id`.
    pub fn bump(&mut self, tab_id: &str) -> u64 {
        self.next += 1;
        self.by_tab.insert(tab_id.to_string(), self.next);
        self.next
    }

    /// The generation of the response currently stored for `tab_id` (0 if none).
    pub fn get(&self, tab_id: &str) -> u64 {
        self.by_tab.get(tab_id).copied().unwrap_or(0)
    }
}

/// The pretty-printable shapes [`BodyTextCache`] recognizes, in the order they are tried. Also
/// the language the Body tab's editor highlights the pretty view with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrettyKind {
    /// Valid JSON, reindented by [`pretty_print_json`].
    Json,
    /// Markup, reindented by [`pretty_print_html`]. Covers HTML and, loosely, XML: neither this
    /// crate nor the editor's grammar tells them apart, and indenting by tag nesting is the same
    /// either way.
    Html,
}

impl PrettyKind {
    /// The `EditorState::language` name of this kind's syntax highlighting.
    pub fn language(self) -> &'static str {
        match self {
            PrettyKind::Json => "json",
            PrettyKind::Html => "html",
        }
    }
}

/// The text the Body tab shows, computed once per response instead of once per render.
#[derive(Debug, Clone)]
pub struct ShownBody {
    /// The text to display, cheap to clone.
    pub text: Arc<str>,
    /// Whether `text` is the raw body (raw was requested, or the body matches no [`PrettyKind`]).
    pub raw: bool,
    /// The shape the pretty view was recognized as, so the Pretty view is available and the
    /// editor knows what to highlight it as. `None` when the body matches none.
    pub pretty_kind: Option<PrettyKind>,
}

/// Caches the pretty and raw texts of the last response shown in the Body tab.
#[derive(Debug, Default)]
pub struct BodyTextCache {
    /// The tab, the response generation and the JSON indentation the texts were computed for.
    key: Option<(String, u64, usize)>,
    /// `None` until computed; the inner `None` means the body matched no [`PrettyKind`].
    pretty: Option<Option<(Arc<str>, PrettyKind)>>,
    raw: Option<Arc<str>>,
    /// How many times a text was computed, for tests.
    computations: usize,
}

impl BodyTextCache {
    /// Returns the text to show for the response identified by `(tab_id, generation)`.
    ///
    /// This never pretty prints: that can be slow enough on a large body (`SendOptions`'s own
    /// doc comment measures a 15 MB response at hundreds of milliseconds of extra work) to stall
    /// the window for a beat, so it runs on the background executor instead
    /// (`views/response_view.rs`'s `ResponseEditorEntities::ensure_pretty`), which calls
    /// [`Self::store_pretty`] with the result. Until that lands, [`Self::pretty_pending`] is
    /// true and this falls back to raw, the same as a body that turns out not to be pretty
    /// printable at all. `body` is only read when the key changes or the raw text is first
    /// requested; both are cheap (a UTF-8 check and a copy), unlike pretty printing.
    pub fn shown(&mut self, tab_id: &str, generation: u64, body: &[u8], want_raw: bool, indent: usize) -> ShownBody {
        self.sync_key(tab_id, generation, indent);
        let pretty = self.pretty.clone().flatten();
        let pretty_kind = pretty.as_ref().map(|(_, kind)| *kind);
        match pretty {
            Some((text, _)) if !want_raw => ShownBody {
                text,
                raw: false,
                pretty_kind,
            },
            _ => {
                if self.raw.is_none() {
                    self.computations += 1;
                    let started = Instant::now();
                    self.raw = Some(Arc::from(body_as_text(body)));
                    log::debug!(
                        "response body of {tab_id}: raw text of {} bytes took {} us",
                        body.len(),
                        started.elapsed().as_micros()
                    );
                }
                ShownBody {
                    text: self.raw.clone().unwrap_or_else(|| Arc::from("")),
                    raw: true,
                    pretty_kind,
                }
            }
        }
    }

    /// Whether the response identified by `(tab_id, generation, indent)` still needs its pretty
    /// text computed: `views/response_view.rs` checks this after [`Self::shown`] to decide
    /// whether to spawn [`Self::store_pretty`]'s background computation.
    pub fn pretty_pending(&self, tab_id: &str, generation: u64, indent: usize) -> bool {
        self.pretty.is_none()
            && self.key.as_ref().is_some_and(|(tab, gen_, spaces)| {
                tab == tab_id && *gen_ == generation && *spaces == indent
            })
    }

    /// Resets the cache when `(tab_id, generation, indent)` differs from the last call,
    /// otherwise leaves it as is.
    fn sync_key(&mut self, tab_id: &str, generation: u64, indent: usize) {
        let same = self.key.as_ref().is_some_and(|(tab, gen_, spaces)| {
            tab == tab_id && *gen_ == generation && *spaces == indent
        });
        if !same {
            self.key = Some((tab_id.to_string(), generation, indent));
            self.pretty = None;
            self.raw = None;
        }
    }

    /// Commits a pretty print computed in the background for `(tab_id, generation, indent)`.
    /// Dropped (not a stale write) when the cache has since moved on to a different key, which
    /// happens when the response was resent or the active tab changed while the computation was
    /// still running.
    pub fn store_pretty(
        &mut self,
        tab_id: &str,
        generation: u64,
        indent: usize,
        pretty: Option<(String, PrettyKind)>,
    ) {
        let current = self.key.as_ref().is_some_and(|(tab, gen_, spaces)| {
            tab == tab_id && *gen_ == generation && *spaces == indent
        });
        if !current {
            return;
        }
        self.computations += 1;
        self.pretty = Some(pretty.map(|(text, kind)| (Arc::from(text), kind)));
    }
}

/// Formats a byte count for display: bytes under 1000 as is, otherwise kilobytes or megabytes
/// with one decimal place.
pub fn format_size(bytes: usize) -> String {
    let bytes = bytes as f64;
    if bytes < 1000.0 {
        format!("{} B", format_integer(bytes as u64))
    } else if bytes < 1_000_000.0 {
        format!("{} KB", format_decimal(bytes / 1000.0, 1))
    } else {
        format!("{} MB", format_decimal(bytes / 1_000_000.0, 1))
    }
}

/// Formats a duration for display: whole milliseconds under one second, otherwise seconds with
/// two decimal places.
pub fn format_duration(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1000 {
        format!("{} ms", format_integer(millis as u64))
    } else {
        format!("{} s", format_decimal(duration.as_secs_f64(), 2))
    }
}

/// The response headers as the "Copy all headers" button puts them on the clipboard: one
/// `Name: value` line per header, in the order received, repeated headers on lines of their own.
pub fn headers_as_text(headers: &[ResolvedField]) -> String {
    headers
        .iter()
        .map(|header| format!("{}: {}", header.name, header.value))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn headers_as_text_puts_one_header_per_line_in_order() {
        let header = |name: &str, value: &str| ResolvedField {
            name: name.to_string(),
            value: value.to_string(),
        };
        let headers = [
            header("Content-Type", "application/json"),
            header("Set-Cookie", "a=1; Path=/"),
            header("Set-Cookie", "b=2"),
        ];
        assert_eq!(
            headers_as_text(&headers),
            "Content-Type: application/json\nSet-Cookie: a=1; Path=/\nSet-Cookie: b=2"
        );
        assert_eq!(headers_as_text(&[]), "");
    }

    /// What `views/response_view.rs`'s background spawn does, inline: computes the pretty text
    /// the same way it would, then commits it with [`BodyTextCache::store_pretty`].
    fn compute_and_store_pretty(cache: &mut BodyTextCache, tab_id: &str, generation: u64, body: &[u8], indent: usize) {
        let pretty = pretty_print_json(body, indent)
            .map(|text| (text, PrettyKind::Json))
            .or_else(|| pretty_print_html(body, indent).map(|text| (text, PrettyKind::Html)));
        cache.store_pretty(tab_id, generation, indent, pretty);
    }

    #[test]
    fn shown_is_raw_and_pending_until_the_pretty_text_is_stored() {
        let mut cache = BodyTextCache::default();
        let body = br#"{"a":1}"#;
        let first = cache.shown("t", 1, body, false, 2);
        assert!(first.raw && first.pretty_kind.is_none());
        assert!(cache.pretty_pending("t", 1, 2));

        compute_and_store_pretty(&mut cache, "t", 1, body, 2);
        assert!(!cache.pretty_pending("t", 1, 2));

        let pretty = cache.shown("t", 1, body, false, 2);
        assert!(!pretty.raw && pretty.pretty_kind == Some(PrettyKind::Json));
        let before = cache.computations;
        let again = cache.shown("t", 1, body, false, 2);
        assert!(Arc::ptr_eq(&pretty.text, &again.text));
        assert_eq!(cache.computations, before);

        let raw = cache.shown("t", 1, body, true, 2);
        assert!(raw.raw);
        assert_eq!(&*raw.text, r#"{"a":1}"#);
    }

    #[test]
    fn body_cache_recomputes_for_new_response_tab_or_indent() {
        let mut cache = BodyTextCache::default();
        let text = |cache: &mut BodyTextCache, tab: &str, generation: u64, body: &[u8], indent: usize| {
            cache.shown(tab, generation, body, false, indent);
            compute_and_store_pretty(cache, tab, generation, body, indent);
            cache.shown(tab, generation, body, false, indent).text
        };
        assert_eq!(&*text(&mut cache, "t", 1, b"[1]", 2), "[\n  1\n]");
        assert_eq!(&*text(&mut cache, "t", 2, b"[2]", 2), "[\n  2\n]");
        assert_eq!(&*text(&mut cache, "u", 2, b"[3]", 2), "[\n  3\n]");
        // A new indentation recomputes the same response.
        assert_eq!(&*text(&mut cache, "u", 2, b"[3]", 4), "[\n    3\n]");
    }

    #[test]
    fn body_cache_falls_back_to_raw_for_non_json() {
        let mut cache = BodyTextCache::default();
        cache.shown("t", 1, b"hello", false, 2);
        compute_and_store_pretty(&mut cache, "t", 1, b"hello", 2);
        let shown = cache.shown("t", 1, b"hello", false, 2);
        assert!(shown.raw && shown.pretty_kind.is_none());
        assert_eq!(&*shown.text, "hello");
    }

    #[test]
    fn body_cache_pretty_prints_html_when_the_body_is_not_json() {
        let mut cache = BodyTextCache::default();
        cache.shown("t", 1, b"<p>hi</p>", false, 2);
        compute_and_store_pretty(&mut cache, "t", 1, b"<p>hi</p>", 2);
        let shown = cache.shown("t", 1, b"<p>hi</p>", false, 2);
        assert!(!shown.raw && shown.pretty_kind == Some(PrettyKind::Html));
        assert_eq!(&*shown.text, "<p>\n  hi\n</p>\n");
    }

    #[test]
    fn a_stale_pretty_result_is_dropped_when_the_key_moved_on() {
        let mut cache = BodyTextCache::default();
        cache.shown("t", 1, br#"{"a":1}"#, false, 2);
        // The response changed (a resend) before the background computation for generation 1
        // returned.
        cache.shown("t", 2, br#"{"b":2}"#, false, 2);
        compute_and_store_pretty(&mut cache, "t", 1, br#"{"a":1}"#, 2);
        assert!(cache.pretty_pending("t", 2, 2));
    }

    #[test]
    fn generations_change_on_every_store() {
        let mut gens = ResponseGenerations::default();
        assert_eq!(gens.get("a"), 0);
        let first = gens.bump("a");
        let second = gens.bump("a");
        assert_ne!(first, second);
        assert_eq!(gens.get("a"), second);
    }

    #[test]
    fn pretty_print_json_formats_valid_json() {
        let result = pretty_print_json(br#"{"a":1,"b":[1,2]}"#, 2).expect("valid JSON");
        assert_eq!(result, "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ]\n}");
    }

    #[test]
    fn pretty_print_json_rejects_invalid_json() {
        assert_eq!(pretty_print_json(b"not json", 2), None);
    }

    #[test]
    fn pretty_print_json_rejects_non_utf8() {
        assert_eq!(pretty_print_json(&[0xff, 0xfe], 2), None);
    }

    #[test]
    fn body_as_text_shows_utf8_as_is() {
        assert_eq!(body_as_text(b"hello"), "hello");
    }

    #[test]
    fn body_as_text_placeholders_non_utf8() {
        assert_eq!(body_as_text(&[0xff, 0xfe]), "<2 bytes, not valid UTF-8>");
    }

    #[test]
    fn format_size_uses_bytes_below_1000() {
        assert_eq!(format_size(42), "42 B");
        assert_eq!(format_size(999), "999 B");
    }

    #[test]
    fn format_size_uses_kilobytes() {
        assert_eq!(format_size(1500), "1.5 KB");
    }

    #[test]
    fn format_size_uses_megabytes() {
        assert_eq!(format_size(2_500_000), "2.5 MB");
    }

    #[test]
    fn format_duration_uses_milliseconds_below_one_second() {
        assert_eq!(format_duration(Duration::from_millis(842)), "842 ms");
    }

    #[test]
    fn format_duration_uses_seconds_at_and_above_one_second() {
        assert_eq!(format_duration(Duration::from_millis(1200)), "1.20 s");
    }

    #[test]
    fn reason_phrase_covers_common_codes() {
        assert_eq!(reason_phrase(200), Some("OK"));
        assert_eq!(reason_phrase(404), Some("Not Found"));
        assert_eq!(reason_phrase(500), Some("Internal Server Error"));
    }

    #[test]
    fn reason_phrase_is_none_for_an_uncovered_code() {
        assert_eq!(reason_phrase(299), None);
    }

    #[test]
    fn group_warnings_is_empty_for_no_warnings() {
        assert_eq!(group_warnings(&[]), Vec::new());
    }

    #[test]
    fn group_warnings_singular_form_for_one_unknown_variable() {
        let warnings = vec![TemplateWarning::UnknownVariable("token".to_string())];
        assert_eq!(
            group_warnings(&warnings),
            vec![WarningGroup::UnknownVariable("token".to_string())]
        );
    }

    #[test]
    fn group_warnings_plural_form_for_several_unknown_variables() {
        let warnings = vec![
            TemplateWarning::UnknownVariable("token".to_string()),
            TemplateWarning::UnknownVariable("userId".to_string()),
        ];
        assert_eq!(
            group_warnings(&warnings),
            vec![WarningGroup::UnknownVariables(vec![
                "token".to_string(),
                "userId".to_string()
            ])]
        );
    }

    #[test]
    fn group_warnings_deduplicates_repeated_names() {
        let warnings = vec![
            TemplateWarning::UnknownVariable("token".to_string()),
            TemplateWarning::UnknownVariable("token".to_string()),
        ];
        assert_eq!(
            group_warnings(&warnings),
            vec![WarningGroup::UnknownVariable("token".to_string())]
        );
    }

    #[test]
    fn group_warnings_lists_unknown_variables_before_function_errors() {
        use brasa_core::VarScope;
        let scope = VarScope {
            request_vars: &[],
            session_env: &[],
            environment: &[],
        };
        let warnings = vec![
            TemplateWarning::UnknownVariable("token".to_string()),
            brasa_core::interpolate("{{ noSuchFunction() }}", &scope).warnings[0].clone(),
        ];
        let groups = group_warnings(&warnings);
        assert_eq!(
            groups[0],
            WarningGroup::UnknownVariable("token".to_string())
        );
        assert!(matches!(groups[1], WarningGroup::Function(_)));
    }
}
