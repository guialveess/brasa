//! The header suggestions of the autocompletion menu (`state::completion`) in the Headers table:
//! common request header names for a key cell, and the usual values of a known header for its
//! value cell. A suggestion replaces the whole text of the cell. Plain Rust, unit tested without
//! a window.

use super::completion::{CompletionItem, rank};

/// Common request headers, in their canonical spelling.
pub const HEADER_NAMES: &[&str] = &[
    "Accept",
    "Accept-Charset",
    "Accept-Encoding",
    "Accept-Language",
    "Access-Control-Request-Headers",
    "Access-Control-Request-Method",
    "Api-Key",
    "Authorization",
    "Cache-Control",
    "Connection",
    "Content-Encoding",
    "Content-Language",
    "Content-Length",
    "Content-Type",
    "Cookie",
    "Date",
    "Expect",
    "Forwarded",
    "From",
    "Host",
    "Idempotency-Key",
    "If-Match",
    "If-Modified-Since",
    "If-None-Match",
    "If-Range",
    "If-Unmodified-Since",
    "Max-Forwards",
    "Origin",
    "Pragma",
    "Prefer",
    "Proxy-Authorization",
    "Range",
    "Referer",
    "TE",
    "Upgrade",
    "User-Agent",
    "Via",
    "X-API-Key",
    "X-Correlation-ID",
    "X-Forwarded-For",
    "X-Forwarded-Host",
    "X-Forwarded-Proto",
    "X-Request-ID",
    "X-Requested-With",
];

/// Media types offered for `Content-Type` and `Accept`.
const MEDIA_TYPES: &[&str] = &[
    "application/json",
    "application/json; charset=utf-8",
    "application/xml",
    "application/xml; charset=utf-8",
    "text/plain",
    "text/plain; charset=utf-8",
    "text/html",
    "text/csv",
    "text/xml",
    "application/x-www-form-urlencoded",
    "multipart/form-data",
    "application/octet-stream",
    "application/pdf",
    "application/graphql",
    "application/ld+json",
    "application/problem+json",
    "application/vnd.api+json",
];

/// The usual values of the header `name` (any letter case), empty for a header with none.
pub fn header_values(name: &str) -> Vec<&'static str> {
    let name = name.trim().to_ascii_lowercase();
    match name.as_str() {
        "content-type" => MEDIA_TYPES.to_vec(),
        "accept" => [&["*/*"][..], MEDIA_TYPES].concat(),
        "accept-encoding" => vec![
            "gzip, deflate, br",
            "gzip",
            "deflate",
            "br",
            "identity",
            "*",
        ],
        "content-encoding" => vec!["gzip", "deflate", "br", "identity"],
        "accept-language" => vec!["en-US,en;q=0.9", "es-ES,es;q=0.9", "*"],
        "accept-charset" => vec!["utf-8"],
        "authorization" | "proxy-authorization" => vec!["Bearer ", "Basic "],
        "cache-control" => vec![
            "no-cache",
            "no-store",
            "max-age=0",
            "must-revalidate",
            "no-transform",
            "only-if-cached",
        ],
        "connection" => vec!["keep-alive", "close"],
        "pragma" => vec!["no-cache"],
        "prefer" => vec!["return=minimal", "return=representation", "respond-async"],
        "expect" => vec!["100-continue"],
        "te" => vec!["trailers"],
        "upgrade" => vec!["websocket"],
        "x-requested-with" => vec!["XMLHttpRequest"],
        "x-forwarded-proto" => vec!["https", "http"],
        "access-control-request-method" => {
            vec!["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]
        }
        _ => Vec::new(),
    }
}

/// The suggestions among `candidates` for a cell whose whole text is `text`.
///
/// While typing, the menu lists the candidates matching the text, ranked by [`rank`], and none
/// for an empty cell, a text that already is one of them (any letter case) or a `{{` marker. On
/// demand (`explicit`, `Ctrl+Space`) an empty or already complete cell lists every candidate.
pub fn suggestions(candidates: &[&'static str], text: &str, explicit: bool) -> Vec<&'static str> {
    let query = text.trim();
    if text.contains("{{") {
        return Vec::new();
    }
    let complete = candidates
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(query));
    if query.is_empty() || complete {
        return if explicit {
            candidates.to_vec()
        } else {
            Vec::new()
        };
    }
    rank(candidates, query, |candidate| candidate)
        .into_iter()
        .copied()
        .collect()
}

/// The menu rows for [`suggestions`], each replacing the whole `text` of the cell.
pub fn items(candidates: &[&'static str], text: &str, explicit: bool) -> Vec<CompletionItem> {
    suggestions(candidates, text, explicit)
        .into_iter()
        .map(|suggestion| CompletionItem {
            label: suggestion.to_string(),
            insert: suggestion.to_string(),
            replace: 0..text.len(),
            variable: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn typing_part_of_a_name_lists_the_matching_headers() {
        let found = suggestions(HEADER_NAMES, "Acc", false);
        assert_eq!(found[0], "Accept");
        assert!(found.contains(&"Accept-Encoding"));
        assert!(found.iter().all(|name| name.starts_with("Acc")));
        // Prefix matches first, then the names that only contain it.
        let found = suggestions(HEADER_NAMES, "type", false);
        assert_eq!(found, vec!["Content-Type"]);
        assert_eq!(suggestions(HEADER_NAMES, "x-req", false)[0], "X-Request-ID");
    }

    #[test]
    fn nothing_while_typing_in_an_empty_complete_or_unknown_cell() {
        assert!(suggestions(HEADER_NAMES, "", false).is_empty());
        assert!(suggestions(HEADER_NAMES, "accept", false).is_empty());
        assert!(suggestions(HEADER_NAMES, "X-My-Header", false).is_empty());
        assert!(suggestions(HEADER_NAMES, "{{name", false).is_empty());
    }

    #[test]
    fn ctrl_space_lists_everything_in_an_empty_or_complete_cell() {
        assert_eq!(
            suggestions(HEADER_NAMES, "", true).len(),
            HEADER_NAMES.len()
        );
        assert_eq!(
            suggestions(HEADER_NAMES, "Accept", true).len(),
            HEADER_NAMES.len()
        );
        assert_eq!(suggestions(HEADER_NAMES, "Acc", true)[0], "Accept");
    }

    #[test]
    fn the_usual_api_key_headers_are_offered() {
        let found = suggestions(HEADER_NAMES, "api", false);
        assert_eq!(found[0], "Api-Key");
        assert!(found.contains(&"X-API-Key"));
    }

    #[test]
    fn content_type_offers_json_with_and_without_a_charset() {
        let values = header_values("content-TYPE");
        let found = suggestions(&values, "application/js", false);
        assert_eq!(
            found,
            vec!["application/json", "application/json; charset=utf-8"]
        );
        assert!(suggestions(&values, "Application/JSON", false).is_empty());
        assert!(header_values("Accept").contains(&"*/*"));
        assert!(header_values("X-Custom").is_empty());
    }

    #[test]
    fn a_suggestion_replaces_the_whole_cell() {
        let rows = items(HEADER_NAMES, "acc", false);
        assert_eq!(rows[0].label, "Accept");
        assert_eq!(rows[0].insert, "Accept");
        assert_eq!(rows[0].replace, 0..3);
        assert!(rows[0].variable.is_none());
    }
}
