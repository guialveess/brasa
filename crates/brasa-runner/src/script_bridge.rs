//! Conversions between `brasa-core`'s `Request`/`Response`/`ResolvedRequest` types and the
//! plain `brasa-script` types a `::: pre`/`::: post` script sees, plus the "apply mutations
//! back" side of that trip. See `docs/scripting.md` for the script API.
//!
//! A script only sees and can change `method`, `url`, `headers` and `body` (never `query`,
//! `auth`, the scripts themselves or `docs`), and only ever sees *enabled* headers and form fields: a
//! disabled entry is invisible to the script and is carried through unchanged, appended after
//! whatever headers or form fields the script leaves behind. Scripts have no notion of a
//! disabled entry, so anything they add or keep is always enabled.
//!
//! A request with no body (`Body::None`) has nothing for a pre script to edit: `req.body` reads
//! as an empty string, and assigning to it is a no-op, since a script has no way to say what
//! type of body it would want to create.

use brasa_core::{Body, KeyValue, Method, Request, ResolvedBody, ResolvedRequest, Response};
use brasa_script::{ScriptRequest, ScriptResponse};

/// Builds the `req` a pre script sees from the current working [`Request`].
///
/// Only enabled headers are visible (see the module docs). A `Body::Form` is shown as its
/// enabled fields joined with `&`, each as `key=value` (not yet percent-encoded: that only
/// happens once the request is resolved, in [`crate::resolve`]).
pub(crate) fn to_script_request(request: &Request) -> ScriptRequest {
    ScriptRequest {
        method: request.method.to_string(),
        url: request.url.clone(),
        headers: enabled_entries(&request.headers),
        body: body_to_script_text(&request.body),
    }
}

/// Builds the `req` a post script sees from the [`ResolvedRequest`] that was actually sent, so
/// `req` reflects reality rather than the pre-interpolation source (`req` is read-only in a post
/// script and reflects what was actually sent, see `docs/scripting.md`).
pub(crate) fn resolved_to_script_request(resolved: &ResolvedRequest) -> ScriptRequest {
    ScriptRequest {
        method: resolved.method.to_string(),
        url: resolved.url.clone(),
        headers: resolved
            .headers
            .iter()
            .map(|field| KeyValue::new(field.name.clone(), field.value.clone()))
            .collect(),
        body: match &resolved.body {
            ResolvedBody::None => String::new(),
            ResolvedBody::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        },
    }
}

/// Applies the script's mutations back onto `original`, producing the working [`Request`] used
/// for the rest of the pipeline. Everything a script cannot see (`query`, `auth`, the scripts
/// themselves, `docs`) is carried over unchanged from `original`.
pub(crate) fn apply_script_request(original: &Request, script_request: ScriptRequest) -> Request {
    let method = match script_request.method.parse::<Method>() {
        Ok(method) => method,
        // `Method::from_str` never fails (its `Err` type is the uninhabited `Infallible`).
        Err(never) => match never {},
    };

    Request {
        method,
        url: script_request.url,
        headers: merge_back(&original.headers, script_request.headers),
        body: body_from_script_text(&original.body, script_request.body),
        query: original.query.clone(),
        auth: original.auth.clone(),
        pre_script: original.pre_script.clone(),
        pre_enabled: original.pre_enabled,
        post_script: original.post_script.clone(),
        post_enabled: original.post_enabled,
        docs: original.docs.clone(),
    }
}

/// Builds the `res` a post script sees from the [`Response`] just received. The body is decoded
/// as UTF-8, lossily if it is not valid UTF-8, so it can be handed to the script as a plain
/// JavaScript string.
pub(crate) fn to_script_response(response: &Response) -> ScriptResponse {
    ScriptResponse {
        status: response.status,
        headers: response
            .headers
            .iter()
            .map(|field| KeyValue::new(field.name.clone(), field.value.clone()))
            .collect(),
        body: String::from_utf8_lossy(&response.body).into_owned(),
        time_ms: response.time.as_millis(),
        size: response.size,
    }
}

/// The enabled entries of `fields`, in order, as the plain entries a script sees.
fn enabled_entries(fields: &[KeyValue]) -> Vec<KeyValue> {
    fields
        .iter()
        .filter(|field| field.enabled)
        .cloned()
        .collect()
}

/// Rebuilds a field list after a script ran: the script's own list (all enabled, a script has no
/// notion of a disabled entry) followed by the original disabled entries, unchanged.
fn merge_back(original: &[KeyValue], script_fields: Vec<KeyValue>) -> Vec<KeyValue> {
    let mut merged = script_fields;
    merged.extend(original.iter().filter(|field| !field.enabled).cloned());
    merged
}

/// The text a pre script sees for `req.body`.
fn body_to_script_text(body: &Body) -> String {
    match body {
        Body::None => String::new(),
        Body::Json(text) | Body::Text(text) | Body::Xml(text) => text.clone(),
        Body::Form(fields) | Body::Multipart(fields) => join_form(&enabled_entries(fields)),
    }
}

/// Rebuilds the body after a script ran, given the original body (for its type) and the new
/// `req.body` text.
///
/// `Body::None` stays `Body::None`: a script can edit the text of an existing body but cannot
/// invent a new one out of nothing, since it has no way to say which type it would be.
fn body_from_script_text(original: &Body, text: String) -> Body {
    match original {
        Body::None => Body::None,
        Body::Json(_) => Body::Json(text),
        Body::Text(_) => Body::Text(text),
        Body::Xml(_) => Body::Xml(text),
        Body::Form(fields) => Body::Form(merge_back(fields, split_form(&text))),
        Body::Multipart(fields) => Body::Multipart(merge_back(fields, split_form(&text))),
    }
}

/// Joins enabled form fields as `key=value` pairs separated by `&`, using the raw (not yet
/// percent-encoded) key and value text. Percent-encoding only happens once the request is
/// resolved, in [`crate::resolve`].
fn join_form(fields: &[KeyValue]) -> String {
    fields
        .iter()
        .map(|field| format!("{}={}", field.key, field.value))
        .collect::<Vec<_>>()
        .join("&")
}

/// The inverse of [`join_form`]: splits `text` on `&`, then each piece on its first `=` into a
/// key and a value (a piece with no `=` becomes a key with an empty value). An empty `text`
/// produces no fields.
fn split_form(text: &str) -> Vec<KeyValue> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split('&')
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => KeyValue::new(key, value),
            None => KeyValue::new(pair, ""),
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn to_script_request_hides_disabled_headers() {
        let mut disabled = KeyValue::new("X-Disabled", "no");
        disabled.enabled = false;
        let request = Request {
            headers: vec![KeyValue::new("X-Enabled", "yes"), disabled],
            ..Request::default()
        };

        let script_request = to_script_request(&request);
        assert_eq!(
            script_request.headers,
            vec![KeyValue::new("X-Enabled", "yes")]
        );
    }

    #[test]
    fn apply_script_request_keeps_disabled_headers_unchanged() {
        let mut disabled = KeyValue::new("X-Disabled", "no");
        disabled.enabled = false;
        let original = Request {
            headers: vec![KeyValue::new("X-Enabled", "yes"), disabled.clone()],
            ..Request::default()
        };

        let script_request = ScriptRequest {
            headers: vec![KeyValue::new("X-Added", "new")],
            ..to_script_request(&original)
        };
        let applied = apply_script_request(&original, script_request);
        assert_eq!(
            applied.headers,
            vec![KeyValue::new("X-Added", "new"), disabled]
        );
    }

    #[test]
    fn form_body_round_trips_through_the_script_untouched() {
        let original = Request {
            body: Body::Form(vec![KeyValue::new("a", "1"), KeyValue::new("b", "2")]),
            ..Request::default()
        };

        let script_request = to_script_request(&original);
        assert_eq!(script_request.body, "a=1&b=2");

        let applied = apply_script_request(&original, script_request);
        assert_eq!(
            applied.body,
            Body::Form(vec![KeyValue::new("a", "1"), KeyValue::new("b", "2")])
        );
    }

    #[test]
    fn body_none_stays_none_even_if_the_script_writes_to_it() {
        let original = Request::default();
        let script_request = ScriptRequest {
            body: "ignored".to_string(),
            ..to_script_request(&original)
        };

        let applied = apply_script_request(&original, script_request);
        assert_eq!(applied.body, Body::None);
    }
}
