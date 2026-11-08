//! The `.brasa` request file format: parser and serializer.
//!
//! See `docs/format.md` for the grammar this module implements and a full example. [`parse`] turns file text into a [`Request`], [`serialize`] turns a
//! [`Request`] back into canonically formatted text, and `serialize(parse(text)) == text` for
//! every canonically formatted file (section 3.3).

use brasa_core::{ApiKeyLocation, Auth, Body, KeyValue, Method, Request};

/// The section marker prefix.
const SECTION_PREFIX: &str = "::: ";

/// An error found while parsing a `.brasa` file, with the 1-based line number where it was
/// found and what went wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {kind}")]
pub struct ParseError {
    /// The 1-based line number of the offending line.
    pub line: usize,
    /// What went wrong.
    pub kind: ParseErrorKind,
}

/// The specific problem found while parsing a `.brasa` file. See [`ParseError`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseErrorKind {
    /// The file has no request line: it is empty, or contains only blank lines.
    #[error("missing request line")]
    MissingRequestLine,
    /// The request line has a method but no URL.
    #[error("missing URL in request line")]
    MissingUrl,
    /// The method token of the request line is not uppercase.
    #[error("invalid method {0:?}, expected an uppercase token such as GET or POST")]
    InvalidMethod(String),
    /// A line in the header block is not `Name: value`.
    #[error("invalid header line {0:?}, expected \"Name: value\"")]
    InvalidHeaderLine(String),
    /// A non-blank line was found where a section marker (`::: kind`) was expected.
    #[error("expected a section marker (\"::: kind\"), found {0:?}")]
    ExpectedSection(String),
    /// A `::: kind` line uses a kind that is not `query`, `auth`, `body`, `pre`, `post` or
    /// `docs`.
    #[error("unknown section kind {0:?}")]
    UnknownSection(String),
    /// A section kind appears more than once in the file.
    #[error("duplicated section {0:?}")]
    DuplicatedSection(String),
    /// A section that takes no argument was given one.
    #[error("section {0:?} does not take an argument")]
    UnexpectedArgument(String),
    /// `::: pre`/`::: post` was given an argument other than `off`.
    #[error("invalid argument {0:?} for a pre or post section, expected nothing or \"off\"")]
    InvalidScriptArgument(String),
    /// `::: body` was written without a type.
    #[error("missing body type, expected \"::: body <type>\"")]
    MissingBodyType,
    /// `::: body <type>` uses a type that is not `json`, `text`, `xml` or `form`.
    #[error("unknown body type {0:?}, expected json, text, xml, form or multipart")]
    UnknownBodyType(String),
    /// `::: auth` was written without a type.
    #[error("missing auth type, expected \"::: auth <type>\"")]
    MissingAuthType,
    /// `::: auth <type>` uses a type that is not `basic`, `bearer` or `apikey`.
    #[error("unknown auth type {0:?}, expected basic, bearer or apikey")]
    UnknownAuthType(String),
    /// A line of the `auth` section is not `field = value`.
    #[error("invalid auth line {0:?}, expected \"field = value\"")]
    InvalidAuthLine(String),
    /// A field of the `auth` section is not one its type has.
    #[error("unknown field {field:?} for auth {auth:?}")]
    UnknownAuthField {
        /// The auth type, as written after `::: auth`.
        auth: String,
        /// The field name.
        field: String,
    },
    /// A field appears more than once in the `auth` section.
    #[error("duplicated auth field {0:?}")]
    DuplicatedAuthField(String),
    /// The `in` field of an `apikey` auth is not `header` or `query`.
    #[error("invalid API key location {0:?}, expected header or query")]
    InvalidApiKeyLocation(String),
}

/// Parses the text of a `.brasa` file into a [`Request`].
///
/// Both `\n` and `\r\n` line endings are accepted. See `docs/format.md` for the full
/// grammar.
pub fn parse(text: &str) -> Result<Request, ParseError> {
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();

    let mut index = skip_blank(&lines, 0);
    if index >= lines.len() {
        return Err(ParseError {
            line: 1,
            kind: ParseErrorKind::MissingRequestLine,
        });
    }
    let (method, url) = parse_request_line(lines[index], index + 1)?;
    index += 1;

    let mut headers = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if line.trim().is_empty() {
            index += 1;
            break;
        }
        if line.starts_with(SECTION_PREFIX) {
            break;
        }
        headers.push(parse_header_line(line, index + 1)?);
        index += 1;
    }

    let mut request = Request {
        method,
        url,
        headers,
        ..Request::default()
    };
    let mut seen_kinds: Vec<String> = Vec::new();

    loop {
        index = skip_blank(&lines, index);
        if index >= lines.len() {
            break;
        }
        let line = lines[index];
        if !line.starts_with(SECTION_PREFIX) {
            return Err(ParseError {
                line: index + 1,
                kind: ParseErrorKind::ExpectedSection(line.to_string()),
            });
        }
        let marker_line = index + 1;
        let (kind, argument) = split_kind_argument(&line[SECTION_PREFIX.len()..]);
        if !matches!(kind, "query" | "auth" | "body" | "pre" | "post" | "docs") {
            return Err(ParseError {
                line: marker_line,
                kind: ParseErrorKind::UnknownSection(kind.to_string()),
            });
        }
        if seen_kinds.iter().any(|seen| seen == kind) {
            return Err(ParseError {
                line: marker_line,
                kind: ParseErrorKind::DuplicatedSection(kind.to_string()),
            });
        }
        seen_kinds.push(kind.to_string());
        index += 1;

        let content_start = index;
        while index < lines.len() && !lines[index].starts_with(SECTION_PREFIX) {
            index += 1;
        }
        let content = trim_trailing_blank(&lines[content_start..index]);

        match kind {
            "query" => {
                require_no_argument(kind, argument, marker_line)?;
                request.query = parse_key_value_lines(content);
            }
            "auth" => {
                request.auth = parse_auth_section(argument, content, marker_line)?;
            }
            "body" => {
                request.body = parse_body_section(argument, content, marker_line)?;
            }
            "pre" => {
                request.pre_enabled = parse_script_argument(argument, marker_line)?;
                request.pre_script = join_content(content);
            }
            "post" => {
                request.post_enabled = parse_script_argument(argument, marker_line)?;
                request.post_script = join_content(content);
            }
            "docs" => {
                require_no_argument(kind, argument, marker_line)?;
                request.docs = join_content(content);
            }
            // Unreachable: `kind` was already checked against this exact set of values above.
            _ => unreachable!("kind was validated to be one of the known section kinds"),
        }
    }

    Ok(request)
}

/// Advances `index` past every line that is blank (empty or whitespace only), up to `lines.len()`.
fn skip_blank(lines: &[&str], mut index: usize) -> usize {
    while index < lines.len() && lines[index].trim().is_empty() {
        index += 1;
    }
    index
}

/// Drops trailing blank lines from a captured section content slice.
fn trim_trailing_blank<'a>(lines: &'a [&'a str]) -> &'a [&'a str] {
    let mut end = lines.len();
    while end > 0 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    &lines[..end]
}

/// Parses the request line: `<METHOD> <URL>`.
fn parse_request_line(line: &str, line_number: usize) -> Result<(Method, String), ParseError> {
    let trimmed = line.trim();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    // `trimmed` is non-empty (the caller only reaches here past `skip_blank`), so a first part
    // always exists.
    let method_token = parts.next().unwrap_or_default();
    let url = parts.next().unwrap_or_default().trim();
    if url.is_empty() {
        return Err(ParseError {
            line: line_number,
            kind: ParseErrorKind::MissingUrl,
        });
    }
    if method_token.chars().any(|c| c.is_ascii_lowercase()) {
        return Err(ParseError {
            line: line_number,
            kind: ParseErrorKind::InvalidMethod(method_token.to_string()),
        });
    }
    // `Method::from_str` never fails: its `Err` type is `Infallible`.
    let method = match method_token.parse::<Method>() {
        Ok(method) => method,
        Err(never) => match never {},
    };
    Ok((method, url.to_string()))
}

/// Parses a single header block line: an active `Name: value`, or a disabled `# Name: value`.
fn parse_header_line(line: &str, line_number: usize) -> Result<KeyValue, ParseError> {
    let unescaped = unescape_line(line);
    let (enabled, content) = match unescaped.strip_prefix('#') {
        Some(rest) => (false, rest.trim_start()),
        None => (true, unescaped.as_str()),
    };
    match content.split_once(':') {
        Some((name, value)) => Ok(KeyValue {
            key: name.trim().to_string(),
            value: value.trim().to_string(),
            enabled,
        }),
        None => Err(ParseError {
            line: line_number,
            kind: ParseErrorKind::InvalidHeaderLine(line.to_string()),
        }),
    }
}

/// Splits the text after `::: ` into its section kind and trimmed argument, `"body json"` into
/// `("body", "json")`, `"query"` into `("query", "")`.
fn split_kind_argument(rest: &str) -> (&str, &str) {
    match rest.split_once(char::is_whitespace) {
        Some((kind, argument)) => (kind, argument.trim()),
        None => (rest, ""),
    }
}

/// Parses the optional argument of a `::: pre`/`::: post` section: empty means the script runs,
/// `off` means it is kept in the file but does not run.
fn parse_script_argument(argument: &str, line: usize) -> Result<bool, ParseError> {
    match argument {
        "" => Ok(true),
        "off" => Ok(false),
        _ => Err(ParseError {
            line,
            kind: ParseErrorKind::InvalidScriptArgument(argument.to_string()),
        }),
    }
}

/// Returns [`ParseErrorKind::UnexpectedArgument`] if `argument` is not empty. Used by sections
/// that take no argument (`query`, `docs`).
fn require_no_argument(kind: &str, argument: &str, line: usize) -> Result<(), ParseError> {
    if argument.is_empty() {
        Ok(())
    } else {
        Err(ParseError {
            line,
            kind: ParseErrorKind::UnexpectedArgument(kind.to_string()),
        })
    }
}

/// Parses a `::: body <type>` section into a [`Body`], dispatching on `argument`.
fn parse_body_section(
    argument: &str,
    content: &[&str],
    marker_line: usize,
) -> Result<Body, ParseError> {
    match argument {
        "" => Err(ParseError {
            line: marker_line,
            kind: ParseErrorKind::MissingBodyType,
        }),
        "json" => Ok(Body::Json(join_content(content))),
        "text" => Ok(Body::Text(join_content(content))),
        "xml" => Ok(Body::Xml(join_content(content))),
        "form" => Ok(Body::Form(parse_key_value_lines(content))),
        "multipart" => Ok(Body::Multipart(parse_key_value_lines(content))),
        other => Err(ParseError {
            line: marker_line,
            kind: ParseErrorKind::UnknownBodyType(other.to_string()),
        }),
    }
}

/// Parses a `::: auth <type>` section into an [`Auth`], dispatching on `argument`.
///
/// Each non-blank line is `field = value`, split on the first `=`, both sides trimmed. A line
/// whose first non-space character is `#` is a comment. A field that is not written is empty,
/// and `in` defaults to `header`.
fn parse_auth_section(
    argument: &str,
    content: &[&str],
    marker_line: usize,
) -> Result<Auth, ParseError> {
    let fields: &[&str] = match argument {
        "" => {
            return Err(ParseError {
                line: marker_line,
                kind: ParseErrorKind::MissingAuthType,
            });
        }
        "basic" => &["username", "password"],
        "bearer" => &["token"],
        "apikey" => &["name", "value", "in"],
        other => {
            return Err(ParseError {
                line: marker_line,
                kind: ParseErrorKind::UnknownAuthType(other.to_string()),
            });
        }
    };

    let mut values: Vec<(&str, String)> = Vec::new();
    for (offset, line) in content.iter().enumerate() {
        let line_number = marker_line + offset + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((field, value)) = trimmed.split_once('=') else {
            return Err(ParseError {
                line: line_number,
                kind: ParseErrorKind::InvalidAuthLine(line.to_string()),
            });
        };
        let field = field.trim();
        let Some(known) = fields.iter().find(|known| **known == field) else {
            return Err(ParseError {
                line: line_number,
                kind: ParseErrorKind::UnknownAuthField {
                    auth: argument.to_string(),
                    field: field.to_string(),
                },
            });
        };
        if values.iter().any(|(seen, _)| seen == known) {
            return Err(ParseError {
                line: line_number,
                kind: ParseErrorKind::DuplicatedAuthField(field.to_string()),
            });
        }
        if *known == "in" && !matches!(value.trim(), "header" | "query") {
            return Err(ParseError {
                line: line_number,
                kind: ParseErrorKind::InvalidApiKeyLocation(value.trim().to_string()),
            });
        }
        values.push((known, value.trim().to_string()));
    }

    let mut take = |field: &str| {
        values
            .iter_mut()
            .find(|(name, _)| *name == field)
            .map(|(_, value)| std::mem::take(value))
            .unwrap_or_default()
    };
    Ok(match argument {
        "basic" => Auth::Basic {
            username: take("username"),
            password: take("password"),
        },
        "bearer" => Auth::Bearer {
            token: take("token"),
        },
        _ => Auth::ApiKey {
            name: take("name"),
            value: take("value"),
            location: if take("in") == "query" {
                ApiKeyLocation::Query
            } else {
                ApiKeyLocation::Header
            },
        },
    })
}

/// Unescapes and joins raw section content lines back into a single multi-line string, used by
/// `pre`, `post`, `docs` and the raw `body` types (`json`, `text`, `xml`).
fn join_content(lines: &[&str]) -> String {
    lines
        .iter()
        .map(|line| unescape_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parses `key = value` lines into [`KeyValue`] entries, used by `query` and `body form`.
///
/// A `#` prefix (with optional following spaces) disables the entry. A line without `=` is a
/// key with an empty value. Blank lines are ignored, they do not produce an entry.
fn parse_key_value_lines(lines: &[&str]) -> Vec<KeyValue> {
    lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let unescaped = unescape_line(line);
            let (enabled, content) = match unescaped.strip_prefix('#') {
                Some(rest) => (false, rest.trim_start()),
                None => (true, unescaped.as_str()),
            };
            let (key, value) = match content.split_once('=') {
                Some((key, value)) => (key.trim(), value.trim()),
                None => (content.trim(), ""),
            };
            KeyValue {
                key: key.to_string(),
                value: value.to_string(),
                enabled,
            }
        })
        .collect()
}

/// Removes one level of `::: ` escaping from a line read from a section's raw content, the
/// inverse of [`escape_line`]. A line that is not escaped is returned unchanged.
fn unescape_line(line: &str) -> String {
    if !starts_with_section_marker_ignoring_backslashes(line) {
        return line.to_string();
    }
    match line.strip_prefix('\\') {
        Some(stripped) => stripped.to_string(),
        None => line.to_string(),
    }
}

/// Adds one level of `::: ` escaping to a content line, so that writing it back out never makes
/// it look like a section marker. A line that does not need escaping is returned unchanged. The
/// inverse of [`unescape_line`].
fn escape_line(line: &str) -> String {
    if starts_with_section_marker_ignoring_backslashes(line) {
        format!("\\{line}")
    } else {
        line.to_string()
    }
}

/// Whether `line`, once its leading backslashes are stripped, starts with the section marker
/// prefix `::: `. This is the shared condition behind [`escape_line`] and [`unescape_line`]: it
/// matches an unescaped marker (`"::: foo"`) as well as any already-escaped one
/// (`"\::: foo"`, `"\\::: foo"`, ...), see the escaping rule in `docs/format.md`.
fn starts_with_section_marker_ignoring_backslashes(line: &str) -> bool {
    line.trim_start_matches('\\').starts_with(SECTION_PREFIX)
}

/// Serializes a [`Request`] into the canonical `.brasa` text form.
///
/// The output always uses `\n` line endings and ends with a single trailing `\n`. Sections are
/// written in the canonical order `query`, `auth`, `body`, `pre`, `post`, `docs`; empty sections
/// are omitted.
pub fn serialize(request: &Request) -> String {
    let mut out = String::new();
    out.push_str(&request.method.to_string());
    out.push(' ');
    out.push_str(&request.url);
    out.push('\n');

    for header in &request.headers {
        write_header_line(&mut out, header);
    }

    let sections = build_sections(request);
    if !sections.is_empty() {
        out.push('\n');
        for (position, (heading, content)) in sections.iter().enumerate() {
            if position > 0 {
                out.push('\n');
            }
            out.push_str(heading);
            out.push('\n');
            for line in content {
                out.push_str(&escape_line(line));
                out.push('\n');
            }
        }
    }
    out
}

/// Builds the list of non-empty sections to serialize, in canonical order.
///
/// A section is omitted only when it is structurally absent: an empty `query` list, `Auth::None`,
/// `Body::None`, or an empty `pre_script`/`post_script`/`docs` string. A body whose type is set
/// but whose text happens to be empty (`Body::Json(String::new())`) is still written, with a
/// heading and no content lines, so it round-trips back to the same `Body` variant.
pub(crate) fn build_sections(request: &Request) -> Vec<(String, Vec<String>)> {
    let mut sections = Vec::new();

    if !request.query.is_empty() {
        let content = request
            .query
            .iter()
            .map(format_query_or_form_line)
            .collect();
        sections.push(("::: query".to_string(), content));
    }

    let auth_section = match &request.auth {
        Auth::None => None,
        Auth::Basic { username, password } => Some((
            "basic",
            auth_lines(&[("username", username), ("password", password)]),
        )),
        Auth::Bearer { token } => Some(("bearer", auth_lines(&[("token", token)]))),
        Auth::ApiKey {
            name,
            value,
            location,
        } => {
            let location = match location {
                ApiKeyLocation::Header => "header",
                ApiKeyLocation::Query => "query",
            };
            Some((
                "apikey",
                auth_lines(&[("name", name), ("value", value), ("in", location)]),
            ))
        }
    };
    if let Some((type_name, content)) = auth_section {
        sections.push((format!("::: auth {type_name}"), content));
    }

    let body_section = match &request.body {
        Body::None => None,
        Body::Json(text) => Some(("json", split_raw(text))),
        Body::Text(text) => Some(("text", split_raw(text))),
        Body::Xml(text) => Some(("xml", split_raw(text))),
        Body::Form(fields) => Some((
            "form",
            fields.iter().map(format_query_or_form_line).collect(),
        )),
        Body::Multipart(fields) => Some((
            "multipart",
            fields.iter().map(format_query_or_form_line).collect(),
        )),
    };
    if let Some((type_name, content)) = body_section {
        sections.push((format!("::: body {type_name}"), content));
    }

    if !request.pre_script.is_empty() {
        let marker = if request.pre_enabled { "::: pre" } else { "::: pre off" };
        sections.push((marker.to_string(), split_raw(&request.pre_script)));
    }
    if !request.post_script.is_empty() {
        let marker = if request.post_enabled {
            "::: post"
        } else {
            "::: post off"
        };
        sections.push((marker.to_string(), split_raw(&request.post_script)));
    }
    if !request.docs.is_empty() {
        sections.push(("::: docs".to_string(), split_raw(&request.docs)));
    }

    sections
}

/// Splits a raw multi-line section value (a script or a body's text) back into its lines.
fn split_raw(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n').map(str::to_string).collect()
}

/// Formats the fields of an `auth` section as `field = value` lines, in the given order. An empty
/// field is left out: it reads back as empty.
fn auth_lines(fields: &[(&str, &str)]) -> Vec<String> {
    fields
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(field, value)| format!("{field} = {value}"))
        .collect()
}

/// Formats a `query`/`form` entry as a `key = value` line, with a `# ` prefix when disabled.
pub(crate) fn format_query_or_form_line(entry: &KeyValue) -> String {
    let prefix = if entry.enabled { "" } else { "# " };
    format!("{prefix}{} = {}", entry.key, entry.value)
}

/// Writes a header line (`Name: value`, or `# Name: value` when disabled) to `out`.
fn write_header_line(out: &mut String, entry: &KeyValue) {
    if !entry.enabled {
        out.push_str("# ");
    }
    out.push_str(&escape_line(&format!("{}: {}", entry.key, entry.value)));
    out.push('\n');
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn a_multipart_body_round_trips() {
        let text = "POST https://example.com\n\n::: body multipart\nnote = hi\ndoc = @a/b.png\n# old = 1\n";
        let request = parse(text).expect("valid multipart body");
        let mut disabled = KeyValue::new("old", "1");
        disabled.enabled = false;
        assert_eq!(
            request.body,
            Body::Multipart(vec![
                KeyValue::new("note", "hi"),
                KeyValue::new("doc", "@a/b.png"),
                disabled,
            ])
        );
        assert_eq!(parse(&serialize(&request)).expect("round trip"), request);
    }

    #[test]
    fn minimal_request_round_trips() {
        let text = "GET https://example.com\n";
        let request = parse(text).expect("valid minimal request");
        assert_eq!(request.method, Method::Get);
        assert_eq!(request.url, "https://example.com");
        assert_eq!(serialize(&request), text);
    }

    #[test]
    fn empty_body_is_a_heading_with_no_content_lines() {
        let text = "POST https://example.com\n\n::: body json\n\n::: post\nlog();\n";
        let request = parse(text).expect("valid request with an empty body");
        assert_eq!(request.body, Body::Json(String::new()));
        assert_eq!(serialize(&request), text);

        let last = parse("POST https://example.com\n\n::: body xml\n").expect("valid request");
        assert_eq!(last.body, Body::Xml(String::new()));
        assert_eq!(
            serialize(&last),
            "POST https://example.com\n\n::: body xml\n"
        );
    }

    #[test]
    fn missing_request_line_on_empty_file() {
        assert_eq!(
            parse(""),
            Err(ParseError {
                line: 1,
                kind: ParseErrorKind::MissingRequestLine
            })
        );
    }

    #[test]
    fn missing_request_line_on_blank_only_file() {
        assert_eq!(
            parse("\n\n   \n"),
            Err(ParseError {
                line: 1,
                kind: ParseErrorKind::MissingRequestLine
            })
        );
    }

    #[test]
    fn missing_url_in_request_line() {
        assert_eq!(
            parse("GET\n"),
            Err(ParseError {
                line: 1,
                kind: ParseErrorKind::MissingUrl
            })
        );
    }

    #[test]
    fn invalid_method_lowercase() {
        assert_eq!(
            parse("get https://example.com\n"),
            Err(ParseError {
                line: 1,
                kind: ParseErrorKind::InvalidMethod("get".to_string())
            })
        );
    }

    #[test]
    fn custom_uppercase_method_is_accepted() {
        let request = parse("PURGE https://example.com/cache\n").expect("valid custom method");
        assert_eq!(request.method, Method::Custom("PURGE".to_string()));
    }

    #[test]
    fn invalid_header_line_without_colon() {
        assert_eq!(
            parse("GET https://example.com\nnot-a-header\n"),
            Err(ParseError {
                line: 2,
                kind: ParseErrorKind::InvalidHeaderLine("not-a-header".to_string())
            })
        );
    }

    #[test]
    fn disabled_header_is_parsed_but_not_enabled() {
        let request =
            parse("GET https://example.com\n# X-Debug: 1\n").expect("valid disabled header");
        let mut debug = KeyValue::new("X-Debug", "1");
        debug.enabled = false;
        assert_eq!(request.headers, vec![debug]);
    }

    #[test]
    fn unknown_section_kind() {
        assert_eq!(
            parse("GET https://example.com\n\n::: nope\nsomething\n"),
            Err(ParseError {
                line: 3,
                kind: ParseErrorKind::UnknownSection("nope".to_string())
            })
        );
    }

    #[test]
    fn duplicated_query_section() {
        let text = "GET https://example.com\n\n::: query\na = 1\n\n::: query\nb = 2\n";
        assert_eq!(
            parse(text),
            Err(ParseError {
                line: 6,
                kind: ParseErrorKind::DuplicatedSection("query".to_string())
            })
        );
    }

    #[test]
    fn duplicated_body_section_with_different_types() {
        let text = "POST https://example.com\n\n::: body json\n{}\n\n::: body text\nhi\n";
        assert_eq!(
            parse(text),
            Err(ParseError {
                line: 6,
                kind: ParseErrorKind::DuplicatedSection("body".to_string())
            })
        );
    }

    #[test]
    fn missing_body_type() {
        assert_eq!(
            parse("POST https://example.com\n\n::: body\n{}\n"),
            Err(ParseError {
                line: 3,
                kind: ParseErrorKind::MissingBodyType
            })
        );
    }

    #[test]
    fn unknown_body_type() {
        assert_eq!(
            parse("POST https://example.com\n\n::: body yaml\nfoo: 1\n"),
            Err(ParseError {
                line: 3,
                kind: ParseErrorKind::UnknownBodyType("yaml".to_string())
            })
        );
    }

    #[test]
    fn unexpected_argument_for_a_no_argument_section() {
        assert_eq!(
            parse("GET https://example.com\n\n::: docs extra\nnote\n"),
            Err(ParseError {
                line: 3,
                kind: ParseErrorKind::UnexpectedArgument("docs".to_string())
            })
        );
    }

    #[test]
    fn invalid_argument_for_a_pre_or_post_section() {
        assert_eq!(
            parse("GET https://example.com\n\n::: pre extra\ncode();\n"),
            Err(ParseError {
                line: 3,
                kind: ParseErrorKind::InvalidScriptArgument("extra".to_string())
            })
        );
    }

    #[test]
    fn pre_and_post_off_round_trip_as_disabled_but_kept() {
        let text = "GET https://example.com\n\n::: pre off\nnoop();\n\n::: post off\nnoop();\n";
        let request = parse(text).expect("valid request");
        assert!(!request.pre_enabled);
        assert_eq!(request.pre_script, "noop();");
        assert!(!request.post_enabled);
        assert_eq!(request.post_script, "noop();");
        assert_eq!(serialize(&request), text);
    }

    #[test]
    fn content_outside_section_after_header_block_is_an_error() {
        assert_eq!(
            parse("GET https://example.com\nAccept: */*\n\nstray text\n"),
            Err(ParseError {
                line: 4,
                kind: ParseErrorKind::ExpectedSection("stray text".to_string())
            })
        );
    }

    #[test]
    fn header_value_may_contain_a_colon() {
        let request = parse("GET https://example.com\nLocation: https://a.test/b:8080\n")
            .expect("valid header with a colon in its value");
        assert_eq!(
            request.headers,
            vec![KeyValue::new("Location", "https://a.test/b:8080")]
        );
    }

    #[test]
    fn query_entry_without_equals_has_an_empty_value() {
        let request =
            parse("GET https://example.com\n\n::: query\nverbose\n").expect("valid query entry");
        assert_eq!(request.query, vec![KeyValue::new("verbose", "")]);
    }

    #[test]
    fn query_value_may_contain_an_equals_sign() {
        let request = parse("GET https://example.com\n\n::: query\nfilter = a=b\n")
            .expect("valid query entry with = in the value");
        assert_eq!(request.query, vec![KeyValue::new("filter", "a=b")]);
    }

    #[test]
    fn crlf_line_endings_are_accepted_and_normalized_on_write() {
        let text = "GET https://example.com\r\nAccept: */*\r\n";
        let request = parse(text).expect("valid CRLF request");
        assert_eq!(
            serialize(&request),
            "GET https://example.com\nAccept: */*\n"
        );
    }

    #[test]
    fn escaped_section_marker_round_trips_in_docs() {
        let text = "GET https://example.com\n\n::: docs\n\\::: not a section\n";
        let request = parse(text).expect("valid escaped marker");
        assert_eq!(request.docs, "::: not a section");
        assert_eq!(serialize(&request), text);
    }

    #[test]
    fn doubly_escaped_backslash_marker_round_trips() {
        let text = "GET https://example.com\n\n::: docs\n\\\\::: literal backslash marker\n";
        let request = parse(text).expect("valid double escape");
        assert_eq!(request.docs, "\\::: literal backslash marker");
        assert_eq!(serialize(&request), text);
    }

    #[test]
    fn unicode_in_url_and_body_round_trips() {
        let text = "POST https://example.com/café\n\n::: body text\nhëllo wörld 世界\n";
        let request = parse(text).expect("valid unicode request");
        assert_eq!(request.url, "https://example.com/café");
        assert_eq!(request.body, Body::Text("hëllo wörld 世界".to_string()));
        assert_eq!(serialize(&request), text);
    }

    #[test]
    fn interior_blank_lines_in_docs_are_kept_but_trailing_ones_are_stripped() {
        let text = "GET https://example.com\n\n::: docs\nfirst paragraph\n\nsecond paragraph\n";
        let request = parse(text).expect("valid docs section");
        assert_eq!(request.docs, "first paragraph\n\nsecond paragraph");
        assert_eq!(serialize(&request), text);

        let with_trailing_blanks =
            "GET https://example.com\n\n::: docs\nkept\n\n\n\n::: pre\nnoop();\n";
        let request = parse(with_trailing_blanks).expect("valid docs with trailing blanks");
        assert_eq!(request.docs, "kept");
    }

    #[test]
    fn header_and_query_order_is_preserved() {
        let text = "GET https://example.com\nB: 2\nA: 1\n\n::: query\nz = 1\ny = 2\n";
        let request = parse(text).expect("valid request");
        assert_eq!(
            request.headers,
            vec![KeyValue::new("B", "2"), KeyValue::new("A", "1")]
        );
        assert_eq!(
            request.query,
            vec![KeyValue::new("z", "1"), KeyValue::new("y", "2")]
        );
        assert_eq!(serialize(&request), text);
    }

    #[test]
    fn body_form_round_trips_with_a_disabled_field() {
        let text = "POST https://example.com\n\n::: body form\nuser = tanis\n# debug = 1\n";
        let request = parse(text).expect("valid form body");
        let mut debug = KeyValue::new("debug", "1");
        debug.enabled = false;
        assert_eq!(
            request.body,
            Body::Form(vec![KeyValue::new("user", "tanis"), debug])
        );
        assert_eq!(serialize(&request), text);
    }

    #[test]
    fn body_xml_round_trips() {
        let text = "POST https://example.com\n\n::: body xml\n<a><b>1</b></a>\n";
        let request = parse(text).expect("valid xml body");
        assert_eq!(request.body, Body::Xml("<a><b>1</b></a>".to_string()));
        assert_eq!(serialize(&request), text);
    }

    #[test]
    fn full_example_from_the_spec_round_trips() {
        let text = "POST {{baseUrl}}/users\n\
Content-Type: application/json\n\
Authorization: Bearer {{token}}\n\
X-Timestamp: {{ now() }}\n\
# X-Debug: 1\n\
\n\
::: query\n\
page = 1\n\
# verbose = true\n\
\n\
::: body json\n\
{ \"name\": \"{{userName}}\", \"id\": \"{{requestId}}\" }\n\
\n\
::: pre\n\
vars.set(\"requestId\", util.uuid());\n\
\n\
::: post\n\
test(\"created\", () => {\n\
  expect(res.status).toBe(201);\n\
});\n\
env.set(\"userId\", res.json().id);\n";
        let request = parse(text).expect("valid full example");
        assert_eq!(serialize(&request), text);
    }

    #[test]
    fn empty_request_model_round_trips() {
        let request = Request {
            url: "https://example.com".to_string(),
            ..Request::default()
        };
        let text = serialize(&request);
        assert_eq!(parse(&text).expect("valid serialized request"), request);
    }

    #[test]
    fn hand_built_full_model_round_trips() {
        let request = Request {
            method: Method::Custom("PURGE".to_string()),
            url: "{{baseUrl}}/x".to_string(),
            headers: vec![KeyValue::new("Accept", "*/*")],
            query: vec![KeyValue::new("page", "1")],
            auth: Auth::Bearer {
                token: "{{token}}".to_string(),
            },
            body: Body::Json("{}".to_string()),
            pre_script: "vars.set(\"a\", 1);".to_string(),
            pre_enabled: true,
            post_script: "test(\"ok\", () => {});".to_string(),
            post_enabled: true,
            docs: "Some notes.".to_string(),
        };
        let text = serialize(&request);
        assert_eq!(parse(&text).expect("valid serialized request"), request);
    }

    #[test]
    fn auth_sections_round_trip() {
        for text in [
            "GET https://a.test\n\n::: auth basic\nusername = demo\npassword = {{password}}\n",
            "GET https://a.test\n\n::: auth bearer\ntoken = {{token}}\n",
            "GET https://a.test\n\n::: auth apikey\nname = X-API-Key\nvalue = k\nin = header\n",
            "GET https://a.test\n\n::: auth apikey\nname = api_key\nvalue = k\nin = query\n",
        ] {
            let request = parse(text).expect("valid auth section");
            assert_eq!(serialize(&request), text);
        }
    }

    #[test]
    fn auth_section_is_parsed_into_its_fields() {
        let request = parse("GET https://a.test\n\n::: auth basic\nusername = demo\n")
            .expect("valid auth section");
        assert_eq!(
            request.auth,
            Auth::Basic {
                username: "demo".to_string(),
                password: String::new(),
            }
        );

        let request = parse(
            "GET https://a.test\n\n::: auth apikey\n# a comment\n\n value = a=b \nname = k\n",
        )
        .expect("valid auth section");
        assert_eq!(
            request.auth,
            Auth::ApiKey {
                name: "k".to_string(),
                value: "a=b".to_string(),
                location: ApiKeyLocation::Header,
            }
        );
    }

    #[test]
    fn auth_section_goes_between_query_and_body() {
        let request = Request {
            url: "https://a.test".to_string(),
            query: vec![KeyValue::new("page", "1")],
            auth: Auth::Bearer {
                token: "t".to_string(),
            },
            body: Body::Json("{}".to_string()),
            ..Request::default()
        };
        assert_eq!(
            serialize(&request),
            "GET https://a.test\n\n::: query\npage = 1\n\n::: auth bearer\ntoken = t\n\n\
             ::: body json\n{}\n"
        );
    }

    #[test]
    fn auth_section_with_empty_fields_keeps_its_type() {
        let request = Request {
            url: "https://a.test".to_string(),
            auth: Auth::Bearer {
                token: String::new(),
            },
            ..Request::default()
        };
        let text = serialize(&request);
        assert_eq!(text, "GET https://a.test\n\n::: auth bearer\n");
        assert_eq!(parse(&text).expect("valid auth section"), request);
    }

    #[test]
    fn auth_section_errors() {
        let error = |text: &str| parse(text).expect_err("invalid auth section");
        assert_eq!(
            error("GET https://a.test\n\n::: auth\n"),
            ParseError {
                line: 3,
                kind: ParseErrorKind::MissingAuthType
            }
        );
        assert_eq!(
            error("GET https://a.test\n\n::: auth digest\n").kind,
            ParseErrorKind::UnknownAuthType("digest".to_string())
        );
        assert_eq!(
            error("GET https://a.test\n\n::: auth bearer\ntoken = a\nnope\n"),
            ParseError {
                line: 5,
                kind: ParseErrorKind::InvalidAuthLine("nope".to_string())
            }
        );
        assert_eq!(
            error("GET https://a.test\n\n::: auth bearer\nusername = a\n").kind,
            ParseErrorKind::UnknownAuthField {
                auth: "bearer".to_string(),
                field: "username".to_string(),
            }
        );
        assert_eq!(
            error("GET https://a.test\n\n::: auth bearer\ntoken = a\ntoken = b\n").kind,
            ParseErrorKind::DuplicatedAuthField("token".to_string())
        );
        assert_eq!(
            error("GET https://a.test\n\n::: auth apikey\nin = cookie\n").kind,
            ParseErrorKind::InvalidApiKeyLocation("cookie".to_string())
        );
        assert_eq!(
            error("GET https://a.test\n\n::: auth bearer\n\n::: auth basic\n").kind,
            ParseErrorKind::DuplicatedSection("auth".to_string())
        );
    }

    #[test]
    fn serialization_is_stable_across_a_second_round_trip() {
        let text = "POST https://example.com\n\n::: body form\na = 1\n";
        let once = serialize(&parse(text).expect("valid first parse"));
        let twice = serialize(&parse(&once).expect("valid second parse"));
        assert_eq!(once, twice);
    }
}
