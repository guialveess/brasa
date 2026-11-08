//! Helpers that keep secrets out of log files. Users paste their logs into bug reports, so a
//! log line may show where a request went, but never a query string, credentials in the URL, or
//! the value of a sensitive header.

use std::borrow::Cow;

/// The text logged in place of a sensitive value.
pub const REDACTED: &str = "***";

/// `url` without its query string, fragment and `user:password@` credentials, safe to log.
///
/// `https://user:pw@api.example.com/v1/items?token=abc#top` becomes
/// `https://api.example.com/v1/items`.
pub fn url_for_log(url: &str) -> Cow<'_, str> {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    let url = &url[..end];
    let authority_start = url.find("://").map_or(0, |index| index + 3);
    let authority_end = url[authority_start..]
        .find('/')
        .map_or(url.len(), |index| authority_start + index);
    match url[authority_start..authority_end].rfind('@') {
        Some(at) => Cow::Owned(format!(
            "{}{}",
            &url[..authority_start],
            &url[authority_start + at + 1..]
        )),
        None => Cow::Borrowed(url),
    }
}

/// Whether a header's value must not be logged: credentials, cookies, and anything whose name
/// suggests a token, key, secret or password.
pub fn is_sensitive_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie"
    ) || ["token", "key", "secret", "password", "auth", "session"]
        .iter()
        .any(|word| name.contains(word))
}

/// The value to log for the header `name`: `value` itself, or [`REDACTED`] for a sensitive one.
pub fn header_value_for_log<'a>(name: &str, value: &'a str) -> &'a str {
    if is_sensitive_header(name) {
        REDACTED
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_for_log_drops_query_and_fragment() {
        assert_eq!(
            url_for_log("https://api.example.com/v1/items?token=abc#top"),
            "https://api.example.com/v1/items"
        );
        assert_eq!(
            url_for_log("http://localhost:8080/#x"),
            "http://localhost:8080/"
        );
    }

    #[test]
    fn url_for_log_drops_credentials() {
        assert_eq!(
            url_for_log("https://user:pw@api.example.com/v1?a=1"),
            "https://api.example.com/v1"
        );
        assert_eq!(
            url_for_log("https://user@example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn url_for_log_keeps_an_at_sign_in_the_path() {
        assert_eq!(
            url_for_log("https://example.com/users/@me"),
            "https://example.com/users/@me"
        );
    }

    #[test]
    fn url_for_log_borrows_when_nothing_is_removed() {
        assert!(matches!(
            url_for_log("https://example.com/a"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn sensitive_headers_are_redacted() {
        for name in [
            "Authorization",
            "Cookie",
            "Set-Cookie",
            "X-API-Key",
            "X-Auth-Token",
            "Client-Secret",
            "X-Session-Id",
        ] {
            assert_eq!(header_value_for_log(name, "value"), REDACTED, "{name}");
        }
        assert_eq!(
            header_value_for_log("Content-Type", "application/json"),
            "application/json"
        );
    }
}
