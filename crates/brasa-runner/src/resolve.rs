//! Turning a [`Request`] into a [`ResolvedRequest`] ready to send: interpolating every `{{ }}`
//! marker, keeping only enabled headers, query and form entries, appending the query string to
//! the URL, applying the `::: auth` section and setting a default `Content-Type` when the body
//! implies one.

use std::path::{Path, PathBuf};

use brasa_core::{
    ApiKeyLocation, Auth, Body, KeyValue, MultipartValue, Request, ResolvedBody, ResolvedField,
    ResolvedRequest, TemplateWarning, VarScope, functions, interpolate, multipart_value,
};

/// What to do with the files of a multipart body.
#[derive(Debug, Clone, Copy)]
pub(crate) enum MultipartFiles<'a> {
    /// Read them, a relative path starting at this folder (the workspace), at the current
    /// directory without one.
    Read(Option<&'a Path>),
    /// Leave their contents out: only the file names matter, for a preview.
    Names,
}

/// Interpolates `request` into a [`ResolvedRequest`], collecting every [`TemplateWarning`] found
/// along the way. This never fails: an unresolved marker is left as is in the output and reported
/// as a warning instead.
///
/// The third value is why a file of a multipart body could not be read. Nothing can be sent then.
pub(crate) fn resolve(
    request: &Request,
    scope: &VarScope,
    files: MultipartFiles<'_>,
) -> (ResolvedRequest, Vec<TemplateWarning>, Option<String>) {
    let mut warnings = Vec::new();

    let interpolated_url = interpolate(&request.url, scope);
    warnings.extend(interpolated_url.warnings);
    let url = with_default_scheme(interpolated_url.text);

    let mut headers = resolve_fields(&request.headers, scope, &mut warnings);
    let mut auth_query = None;
    match resolve_auth(&request.auth, scope, &mut warnings) {
        Some((ApiKeyLocation::Header, field)) => {
            // An enabled header of the same name, written by hand, wins.
            if !headers
                .iter()
                .any(|header| header.name.eq_ignore_ascii_case(&field.name))
            {
                headers.push(field);
            }
        }
        Some((ApiKeyLocation::Query, field)) => auth_query = Some(field),
        None => {}
    }
    let url = append_query(url, &request.query, auth_query, scope, &mut warnings);
    let mut file_error = None;
    let (body, headers) = match &request.body {
        Body::Multipart(fields) => match multipart_body(fields, scope, files, &mut warnings) {
            Ok((bytes, content_type)) => (
                ResolvedBody::Bytes(bytes),
                set_multipart_content_type(headers, content_type),
            ),
            Err(error) => {
                file_error = Some(error);
                (ResolvedBody::None, headers)
            }
        },
        body => (
            resolve_body(body, scope, &mut warnings),
            apply_default_content_type(headers, body),
        ),
    };

    let resolved = ResolvedRequest {
        method: request.method.clone(),
        url,
        headers,
        body,
    };
    (resolved, warnings, file_error)
}

/// Encodes the enabled fields of a multipart body, returning the bytes and the `Content-Type`
/// with its boundary. Fails on a file that cannot be read.
fn multipart_body(
    fields: &[KeyValue],
    scope: &VarScope,
    files: MultipartFiles<'_>,
    warnings: &mut Vec<TemplateWarning>,
) -> Result<(Vec<u8>, String), String> {
    /// One part, ready to encode: its name, file name and contents.
    struct Part {
        name: String,
        file_name: Option<String>,
        content: Vec<u8>,
    }
    let mut parts = Vec::new();
    for field in fields.iter().filter(|field| field.enabled) {
        let name = interpolate(&field.key, scope);
        warnings.extend(name.warnings);
        let value = interpolate(&field.value, scope);
        warnings.extend(value.warnings);
        let (file_name, content) = match multipart_value(&value.text) {
            MultipartValue::Text(text) => (None, text.as_bytes().to_vec()),
            MultipartValue::File(path) => {
                let file_name = Path::new(path).file_name().map_or_else(
                    || path.to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                let content = match files {
                    MultipartFiles::Names => Vec::new(),
                    MultipartFiles::Read(base) => {
                        let full = multipart_path(path, base);
                        std::fs::read(&full).map_err(|error| {
                            format!(
                                "cannot read {} for the field {}: {error}",
                                full.display(),
                                name.text
                            )
                        })?
                    }
                };
                (Some(file_name), content)
            }
        };
        parts.push(Part {
            name: name.text,
            file_name,
            content,
        });
    }
    // The boundary depends on the contents, so the same request always encodes the same bytes,
    // and a boundary that shows up in a part is never used.
    let mut seed: u64 = 0xcbf2_9ce4_8422_2325;
    for part in &parts {
        for byte in part.name.bytes().chain(part.content.iter().copied()) {
            seed ^= u64::from(byte);
            seed = seed.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    let mut boundary = format!("brasa-{seed:016x}");
    while parts
        .iter()
        .any(|part| contains(&part.content, boundary.as_bytes()))
    {
        boundary.push('x');
    }
    let mut bytes = Vec::new();
    for part in &parts {
        bytes.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        let name = quote_multipart(&part.name);
        match &part.file_name {
            Some(file_name) => bytes.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{}\"\r\n\
                     Content-Type: application/octet-stream\r\n\r\n",
                    quote_multipart(file_name)
                )
                .as_bytes(),
            ),
            None => bytes.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            ),
        }
        bytes.extend_from_slice(&part.content);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok((bytes, format!("multipart/form-data; boundary={boundary}")))
}

/// The path of a multipart file: relative ones start at `base`.
fn multipart_path(path: &str, base: Option<&Path>) -> PathBuf {
    let path = Path::new(path);
    match base {
        Some(base) if path.is_relative() => base.join(path),
        _ => path.to_path_buf(),
    }
}

/// Whether `haystack` contains `needle`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Escapes a field or file name for a `Content-Disposition` header, as browsers do.
fn quote_multipart(name: &str) -> String {
    name.replace('"', "%22")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Sets the `Content-Type` of a multipart body. The boundary is only known here, so it replaces a
/// `multipart/form-data` written by hand without one; any other value written by hand stays.
fn set_multipart_content_type(
    mut headers: Vec<ResolvedField>,
    content_type: String,
) -> Vec<ResolvedField> {
    match headers
        .iter_mut()
        .find(|header| header.name.eq_ignore_ascii_case("content-type"))
    {
        Some(header) => {
            let value = header.value.to_ascii_lowercase();
            if value.starts_with("multipart/form-data") && !value.contains("boundary=") {
                header.value = content_type;
            }
        }
        None => headers.push(ResolvedField {
            name: "Content-Type".to_string(),
            value: content_type,
        }),
    }
    headers
}

/// Prepends `http://` to a URL that names no scheme, as Postman does, so `localhost:8080/users`
/// works as typed. An empty URL, or one that starts with a scheme (`https://`, `HTTP://`, ...),
/// is returned unchanged.
fn with_default_scheme(url: String) -> String {
    if url.trim().is_empty() || has_scheme(&url) {
        url
    } else {
        format!("http://{}", url.trim_start())
    }
}

/// Whether `url` starts with `scheme://`, where the scheme is a letter followed by letters,
/// digits, `+`, `-` or `.` (RFC 3986). A `://` later in the URL, as in
/// `localhost/go?to=http://x`, does not count.
fn has_scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.trim_start().split_once("://") else {
        return false;
    };
    let mut chars = scheme.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Interpolates the enabled entries of `fields` (both key and value), skipping disabled ones.
fn resolve_fields(
    fields: &[KeyValue],
    scope: &VarScope,
    warnings: &mut Vec<TemplateWarning>,
) -> Vec<ResolvedField> {
    fields
        .iter()
        .filter(|field| field.enabled)
        .map(|field| {
            let name = interpolate(&field.key, scope);
            warnings.extend(name.warnings);
            let value = interpolate(&field.value, scope);
            warnings.extend(value.warnings);
            ResolvedField {
                name: name.text,
                value: value.text,
            }
        })
        .collect()
}

/// Interpolates the `::: auth` section into the header (or, for an API key sent in the query,
/// the query entry) it adds, and where that goes. `None` when there is no auth, or when the
/// field it cannot do without is empty: the token of a bearer, the name of an API key, both the
/// user name and the password of a basic auth.
fn resolve_auth(
    auth: &Auth,
    scope: &VarScope,
    warnings: &mut Vec<TemplateWarning>,
) -> Option<(ApiKeyLocation, ResolvedField)> {
    let mut resolve = |text: &str| {
        let interpolated = interpolate(text, scope);
        warnings.extend(interpolated.warnings);
        interpolated.text
    };
    match auth {
        Auth::None => None,
        Auth::Basic { username, password } => {
            if username.is_empty() && password.is_empty() {
                return None;
            }
            let credentials = format!("{}:{}", resolve(username), resolve(password));
            Some((
                ApiKeyLocation::Header,
                ResolvedField {
                    name: "Authorization".to_string(),
                    value: format!("Basic {}", functions::base64_encode(&credentials)),
                },
            ))
        }
        Auth::Bearer { token } => {
            if token.is_empty() {
                return None;
            }
            Some((
                ApiKeyLocation::Header,
                ResolvedField {
                    name: "Authorization".to_string(),
                    value: format!("Bearer {}", resolve(token)),
                },
            ))
        }
        Auth::ApiKey {
            name,
            value,
            location,
        } => {
            if name.is_empty() {
                return None;
            }
            Some((
                *location,
                ResolvedField {
                    name: resolve(name),
                    value: resolve(value),
                },
            ))
        }
    }
}

/// Interpolates the enabled `::: query` entries and appends them to `base_url` as a query
/// string, percent-encoding each key and value with [`functions::url_encode`]. Appends with `&`
/// when `base_url` already has a query string (contains `?`), with a leading `?` otherwise.
/// `auth` (an API key sent in the query) goes last, unless an enabled entry already has its name.
/// Returns `base_url` unchanged when there is nothing to append.
fn append_query(
    base_url: String,
    query: &[KeyValue],
    auth: Option<ResolvedField>,
    scope: &VarScope,
    warnings: &mut Vec<TemplateWarning>,
) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .filter(|entry| entry.enabled)
        .map(|entry| {
            let key = interpolate(&entry.key, scope);
            warnings.extend(key.warnings);
            let value = interpolate(&entry.value, scope);
            warnings.extend(value.warnings);
            (key.text, value.text)
        })
        .collect();
    if let Some(field) = auth
        && !pairs.iter().any(|(key, _)| *key == field.name)
    {
        pairs.push((field.name, field.value));
    }

    if pairs.is_empty() {
        return base_url;
    }

    let encoded = pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                functions::url_encode(key),
                functions::url_encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let separator = if base_url.contains('?') { '&' } else { '?' };
    format!("{base_url}{separator}{encoded}")
}

/// Interpolates the body and encodes it to the bytes actually sent.
fn resolve_body(
    body: &Body,
    scope: &VarScope,
    warnings: &mut Vec<TemplateWarning>,
) -> ResolvedBody {
    match body {
        Body::None => ResolvedBody::None,
        Body::Json(text) | Body::Text(text) | Body::Xml(text) => {
            let interpolated = interpolate(text, scope);
            warnings.extend(interpolated.warnings);
            ResolvedBody::Bytes(interpolated.text.into_bytes())
        }
        Body::Form(fields) => {
            let pairs: Vec<(String, String)> = fields
                .iter()
                .filter(|field| field.enabled)
                .map(|field| {
                    let key = interpolate(&field.key, scope);
                    warnings.extend(key.warnings);
                    let value = interpolate(&field.value, scope);
                    warnings.extend(value.warnings);
                    (key.text, value.text)
                })
                .collect();
            let encoded = pairs
                .iter()
                .map(|(key, value)| format!("{}={}", form_encode(key), form_encode(value)))
                .collect::<Vec<_>>()
                .join("&");
            ResolvedBody::Bytes(encoded.into_bytes())
        }
        // Encoded by `multipart_body`, which can fail.
        Body::Multipart(_) => ResolvedBody::None,
    }
}

/// Percent-encodes a string for an `application/x-www-form-urlencoded` body: like
/// [`functions::url_encode`], but a space becomes `+` rather than `%20`, the historical
/// convention every HTTP server expects for this content type.
fn form_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Sets a default `Content-Type` header when the body implies one (`json`, `xml`, `form`) and
/// the user did not already set one (a case-insensitive match against the enabled headers).
fn apply_default_content_type(mut headers: Vec<ResolvedField>, body: &Body) -> Vec<ResolvedField> {
    let Some(default) = default_content_type(body) else {
        return headers;
    };
    let already_set = headers
        .iter()
        .any(|header| header.name.eq_ignore_ascii_case("content-type"));
    if !already_set {
        headers.push(ResolvedField {
            name: "Content-Type".to_string(),
            value: default.to_string(),
        });
    }
    headers
}

/// The default `Content-Type` for a body type, or `None` if it should not get one. Only
/// `json`, `xml` and `form` bodies get a default; `text` does
/// not, since there is no single sensible guess for its content type.
fn default_content_type(body: &Body) -> Option<&'static str> {
    match body {
        Body::Json(_) => Some("application/json"),
        Body::Xml(_) => Some("application/xml"),
        Body::Form(_) => Some("application/x-www-form-urlencoded"),
        Body::Text(_) | Body::Multipart(_) | Body::None => None,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use brasa_core::Method;
    use pretty_assertions::assert_eq;

    /// [`super::resolve`], for tests about anything but multipart files.
    fn resolve(request: &Request, scope: &VarScope) -> (ResolvedRequest, Vec<TemplateWarning>) {
        let (resolved, warnings, _) = super::resolve(request, scope, MultipartFiles::Names);
        (resolved, warnings)
    }

    fn multipart_request(fields: Vec<KeyValue>) -> Request {
        let mut request = base_request("https://example.com");
        request.body = Body::Multipart(fields);
        request
    }

    fn content_type(resolved: &ResolvedRequest) -> &str {
        resolved
            .headers
            .iter()
            .find(|header| header.name == "Content-Type")
            .map_or("", |header| header.value.as_str())
    }

    #[test]
    fn multipart_encodes_text_and_file_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.bin"), [0u8, 159, 146, 150]).expect("write");
        let mut disabled = KeyValue::new("off", "x");
        disabled.enabled = false;
        let request = multipart_request(vec![
            KeyValue::new("note", "{{word}}"),
            KeyValue::new("doc", "@a.bin"),
            KeyValue::new("handle", "@@me"),
            disabled,
        ]);
        let variables = [KeyValue::new("word", "hi")];
        let scope = VarScope {
            environment: &variables,
            ..VarScope::default()
        };

        let (resolved, warnings, error) =
            super::resolve(&request, &scope, MultipartFiles::Read(Some(dir.path())));

        assert_eq!(error, None);
        assert!(warnings.is_empty());
        let header = content_type(&resolved);
        let boundary = header
            .strip_prefix("multipart/form-data; boundary=")
            .expect("a boundary");
        let ResolvedBody::Bytes(bytes) = &resolved.body else {
            panic!("expected bytes");
        };
        let mut expected = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"note\"\r\n\r\nhi\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"doc\"; filename=\"a.bin\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .into_bytes();
        expected.extend_from_slice(&[0, 159, 146, 150]);
        expected.extend_from_slice(
            format!(
                "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"handle\"\r\n\r\n@me\r\n\
                 --{boundary}--\r\n"
            )
            .as_bytes(),
        );
        assert_eq!(bytes, &expected);
    }

    #[test]
    fn multipart_reports_a_file_it_cannot_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let request = multipart_request(vec![KeyValue::new("doc", "@missing.bin")]);
        let (resolved, _, error) =
            super::resolve(&request, &scope(), MultipartFiles::Read(Some(dir.path())));
        assert_eq!(resolved.body, ResolvedBody::None);
        let error = error.expect("an error");
        assert!(
            error.contains("missing.bin") && error.contains("doc"),
            "{error}"
        );
    }

    #[test]
    fn multipart_names_never_read_the_file() {
        let request = multipart_request(vec![KeyValue::new("doc", "@/no/such/file.png")]);
        let (resolved, _, error) = super::resolve(&request, &scope(), MultipartFiles::Names);
        assert_eq!(error, None);
        let ResolvedBody::Bytes(bytes) = &resolved.body else {
            panic!("expected bytes");
        };
        assert!(String::from_utf8_lossy(bytes).contains("filename=\"file.png\""));
    }

    #[test]
    fn a_handwritten_multipart_content_type_gets_the_boundary() {
        let mut request = multipart_request(vec![KeyValue::new("a", "1")]);
        request.headers = vec![KeyValue::new("content-type", "multipart/form-data")];
        let (resolved, _) = resolve(&request, &scope());
        assert!(resolved.headers[0].value.contains("boundary=brasa-"));

        request.headers = vec![KeyValue::new("Content-Type", "multipart/mixed; boundary=x")];
        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(resolved.headers[0].value, "multipart/mixed; boundary=x");
    }

    fn scope() -> VarScope<'static> {
        VarScope::default()
    }

    fn base_request(url: &str) -> Request {
        Request {
            method: Method::Get,
            url: url.to_string(),
            ..Request::default()
        }
    }

    #[test]
    fn a_url_without_a_scheme_defaults_to_http() {
        let (resolved, _) = resolve(&base_request("localhost:8080/users"), &scope());
        assert_eq!(resolved.url, "http://localhost:8080/users");
    }

    #[test]
    fn a_url_with_a_scheme_is_kept() {
        for url in [
            "https://example.com",
            "HTTP://example.com",
            "http://localhost:1",
        ] {
            let (resolved, _) = resolve(&base_request(url), &scope());
            assert_eq!(resolved.url, url);
        }
    }

    #[test]
    fn a_scheme_later_in_the_url_does_not_count() {
        let (resolved, _) = resolve(&base_request("localhost/go?to=http://x"), &scope());
        assert_eq!(resolved.url, "http://localhost/go?to=http://x");
    }

    #[test]
    fn the_default_scheme_comes_before_the_query() {
        let mut request = base_request("example.com/path");
        request.query.push(KeyValue::new("page", "1"));
        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(resolved.url, "http://example.com/path?page=1");
    }

    #[test]
    fn an_empty_url_stays_empty() {
        let (resolved, _) = resolve(&base_request(""), &scope());
        assert_eq!(resolved.url, "");
    }

    #[test]
    fn disabled_headers_are_skipped() {
        let mut request = base_request("http://example.com");
        request.headers.push(KeyValue::new("X-Enabled", "yes"));
        let mut disabled = KeyValue::new("X-Disabled", "no");
        disabled.enabled = false;
        request.headers.push(disabled);

        let (resolved, warnings) = resolve(&request, &scope());
        assert!(warnings.is_empty());
        assert_eq!(
            resolved.headers,
            vec![ResolvedField {
                name: "X-Enabled".to_string(),
                value: "yes".to_string(),
            }]
        );
    }

    #[test]
    fn disabled_query_entries_are_skipped() {
        let mut request = base_request("http://example.com");
        request.query.push(KeyValue::new("page", "1"));
        let mut disabled = KeyValue::new("verbose", "true");
        disabled.enabled = false;
        request.query.push(disabled);

        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(resolved.url, "http://example.com?page=1");
    }

    #[test]
    fn query_is_appended_to_a_bare_url() {
        let mut request = base_request("http://example.com/path");
        request.query.push(KeyValue::new("page", "1"));

        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(resolved.url, "http://example.com/path?page=1");
    }

    #[test]
    fn query_is_appended_to_a_url_that_already_has_a_query_string() {
        let mut request = base_request("http://example.com/path?existing=1");
        request.query.push(KeyValue::new("page", "2"));

        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(resolved.url, "http://example.com/path?existing=1&page=2");
    }

    #[test]
    fn query_keys_and_values_are_percent_encoded() {
        let mut request = base_request("http://example.com");
        request.query.push(KeyValue::new("a b", "c&d"));

        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(resolved.url, "http://example.com?a%20b=c%26d");
    }

    #[test]
    fn json_body_gets_a_default_content_type_when_missing() {
        let mut request = base_request("http://example.com");
        request.body = Body::Json("{}".to_string());

        let (resolved, _) = resolve(&request, &scope());
        assert!(
            resolved
                .headers
                .iter()
                .any(|header| header.name == "Content-Type" && header.value == "application/json")
        );
    }

    #[test]
    fn json_body_keeps_a_user_set_content_type() {
        let mut request = base_request("http://example.com");
        request.headers.push(KeyValue::new(
            "content-type",
            "application/json; charset=utf-8",
        ));
        request.body = Body::Json("{}".to_string());

        let (resolved, _) = resolve(&request, &scope());
        let content_types: Vec<&ResolvedField> = resolved
            .headers
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case("content-type"))
            .collect();
        assert_eq!(content_types.len(), 1);
        assert_eq!(content_types[0].value, "application/json; charset=utf-8");
    }

    #[test]
    fn xml_body_gets_a_default_content_type() {
        let mut request = base_request("http://example.com");
        request.body = Body::Xml("<a/>".to_string());

        let (resolved, _) = resolve(&request, &scope());
        assert!(
            resolved
                .headers
                .iter()
                .any(|header| header.name == "Content-Type" && header.value == "application/xml")
        );
    }

    #[test]
    fn text_body_gets_no_default_content_type() {
        let mut request = base_request("http://example.com");
        request.body = Body::Text("hello".to_string());

        let (resolved, _) = resolve(&request, &scope());
        assert!(
            !resolved
                .headers
                .iter()
                .any(|header| header.name.eq_ignore_ascii_case("content-type"))
        );
    }

    #[test]
    fn no_body_gets_no_default_content_type() {
        let request = base_request("http://example.com");

        let (resolved, _) = resolve(&request, &scope());
        assert!(
            !resolved
                .headers
                .iter()
                .any(|header| header.name.eq_ignore_ascii_case("content-type"))
        );
    }

    #[test]
    fn form_body_is_urlencoded_with_a_default_content_type() {
        let mut request = base_request("http://example.com");
        request.body = Body::Form(vec![
            KeyValue::new("name", "a b"),
            KeyValue::new("id", "1&2"),
        ]);

        let (resolved, _) = resolve(&request, &scope());
        let ResolvedBody::Bytes(bytes) = &resolved.body else {
            panic!("expected a body");
        };
        assert_eq!(String::from_utf8_lossy(bytes), "name=a+b&id=1%262");
        assert!(
            resolved
                .headers
                .iter()
                .any(|header| header.name == "Content-Type"
                    && header.value == "application/x-www-form-urlencoded")
        );
    }

    #[test]
    fn disabled_form_fields_are_skipped() {
        let mut request = base_request("http://example.com");
        let mut disabled = KeyValue::new("secret", "shh");
        disabled.enabled = false;
        request.body = Body::Form(vec![KeyValue::new("a", "1"), disabled]);

        let (resolved, _) = resolve(&request, &scope());
        let ResolvedBody::Bytes(bytes) = &resolved.body else {
            panic!("expected a body");
        };
        assert_eq!(String::from_utf8_lossy(bytes), "a=1");
    }

    #[test]
    fn missing_variable_in_the_url_is_reported() {
        let request = base_request("http://example.com/{{missing}}");

        let (resolved, warnings) = resolve(&request, &scope());
        assert_eq!(resolved.url, "http://example.com/{{missing}}");
        assert_eq!(
            warnings,
            vec![TemplateWarning::UnknownVariable("missing".to_string())]
        );
    }
    fn header<'a>(resolved: &'a ResolvedRequest, name: &str) -> Vec<&'a str> {
        resolved
            .headers
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
            .collect()
    }

    #[test]
    fn basic_auth_adds_an_encoded_authorization_header() {
        let mut request = base_request("http://example.com");
        request.auth = Auth::Basic {
            username: "{{user}}".to_string(),
            password: "pass".to_string(),
        };
        let environment = [KeyValue::new("user", "demo")];
        let scope = VarScope {
            environment: &environment,
            ..VarScope::default()
        };

        let (resolved, warnings) = resolve(&request, &scope);
        assert!(warnings.is_empty());
        // base64("demo:pass")
        assert_eq!(
            header(&resolved, "authorization"),
            vec!["Basic ZGVtbzpwYXNz"]
        );
    }

    #[test]
    fn bearer_auth_adds_an_authorization_header() {
        let mut request = base_request("http://example.com");
        request.auth = Auth::Bearer {
            token: "{{token}}".to_string(),
        };

        let (resolved, warnings) = resolve(&request, &scope());
        assert_eq!(header(&resolved, "authorization"), vec!["Bearer {{token}}"]);
        assert_eq!(
            warnings,
            vec![TemplateWarning::UnknownVariable("token".to_string())]
        );
    }

    #[test]
    fn an_enabled_header_written_by_hand_wins_over_the_auth() {
        let mut request = base_request("http://example.com");
        request
            .headers
            .push(KeyValue::new("authorization", "Bearer by-hand"));
        request.auth = Auth::Bearer {
            token: "from-auth".to_string(),
        };

        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(header(&resolved, "authorization"), vec!["Bearer by-hand"]);

        request.headers[0].enabled = false;
        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(header(&resolved, "authorization"), vec!["Bearer from-auth"]);
    }

    #[test]
    fn api_key_in_a_header() {
        let mut request = base_request("http://example.com");
        request.auth = Auth::ApiKey {
            name: "X-API-Key".to_string(),
            value: "k".to_string(),
            location: ApiKeyLocation::Header,
        };

        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(header(&resolved, "x-api-key"), vec!["k"]);
        assert_eq!(resolved.url, "http://example.com");
    }

    #[test]
    fn api_key_in_the_query_goes_after_the_query_entries() {
        let mut request = base_request("http://example.com?a=1");
        request.query.push(KeyValue::new("page", "2"));
        request.auth = Auth::ApiKey {
            name: "api key".to_string(),
            value: "k&v".to_string(),
            location: ApiKeyLocation::Query,
        };

        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(
            resolved.url,
            "http://example.com?a=1&page=2&api%20key=k%26v"
        );
        assert!(header(&resolved, "api key").is_empty());

        request.query[0] = KeyValue::new("api key", "by-hand");
        let (resolved, _) = resolve(&request, &scope());
        assert_eq!(resolved.url, "http://example.com?a=1&api%20key=by-hand");
    }

    #[test]
    fn an_auth_without_its_required_field_adds_nothing() {
        for auth in [
            Auth::Basic {
                username: String::new(),
                password: String::new(),
            },
            Auth::Bearer {
                token: String::new(),
            },
            Auth::ApiKey {
                name: String::new(),
                value: "k".to_string(),
                location: ApiKeyLocation::Query,
            },
        ] {
            let mut request = base_request("http://example.com");
            request.auth = auth;
            let (resolved, _) = resolve(&request, &scope());
            assert!(resolved.headers.is_empty());
            assert_eq!(resolved.url, "http://example.com");
        }
    }
}
