//! Syntax highlighting of `.brasa` and `.env` files (and plain JSON and JavaScript files) for
//! the diff tab. Pure, no IO.
//!
//! The result is one list of [`Token`]s per line, so a renderer can color a line at a time, but
//! it is computed from the whole text because the kind of a line depends on the section it is in
//! (a JSON body, a script, a `key = value` list, ...) and a JavaScript block comment or template
//! string can span lines. Every pass is linear in the size of the text.
//!
//! Tokens of a line are sorted, never overlap and always lie on char boundaries. Text that no
//! token covers is plain text. The grammar mirrors `docs/format.md` and `crate::parse`, but the
//! highlighter never fails: a line that the parser would reject just gets fewer tokens.

use std::ops::Range;

const SECTION_PREFIX: &str = "::: ";

/// What a [`Token`] is, which decides its color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// The HTTP method on the request line.
    Method,
    /// The URL on the request line.
    Url,
    /// A `{{ ... }}` marker. It wins over the kind of the text around it.
    Variable,
    /// The `Name` of a `Name: value` header line.
    HeaderName,
    /// The `:` of a header, the `=` of a `key = value` line, a JSON or JavaScript bracket or comma.
    Punctuation,
    /// A whole `::: ...` section line.
    Section,
    /// A `#` comment line (`auth` section, `.env`) or a JavaScript comment.
    Comment,
    /// A disabled header, query or form entry (a line starting with `#`, see `docs/format.md`).
    Disabled,
    /// The key of a `key = value` line, or of a JSON object.
    Key,
    /// A string literal.
    String,
    /// A number literal.
    Number,
    /// `true`, `false` or `null`.
    Boolean,
    /// A JavaScript keyword.
    Keyword,
}

/// A colored piece of one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The byte range inside the line (without its line ending).
    pub range: Range<usize>,
    /// What the text is.
    pub kind: TokenKind,
}

/// The language of a text to highlight, see [`language_for_path`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// A `.brasa` request file.
    Brasa,
    /// An `.env` environment file.
    Env,
    /// A JSON file.
    Json,
    /// A JavaScript file.
    JavaScript,
    /// Anything else, no tokens.
    Plain,
}

/// Picks the [`Language`] from the extension of `path`, ignoring case: `.brasa`, `.env` (also
/// `.local.env`), `.json`, `.js` and `.mjs`. Everything else is [`Language::Plain`].
pub fn language_for_path(path: &str) -> Language {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let Some((_, extension)) = name.rsplit_once('.') else {
        return Language::Plain;
    };
    let is = |wanted: &str| extension.eq_ignore_ascii_case(wanted);
    if is("brasa") {
        Language::Brasa
    } else if is("env") {
        Language::Env
    } else if is("json") {
        Language::Json
    } else if is("js") || is("mjs") {
        Language::JavaScript
    } else {
        Language::Plain
    }
}

/// Highlights `text`, returning one list of tokens per line (the text split on `\n`, a trailing
/// `\r` is not part of the line). The ranges are byte offsets inside their own line.
///
/// For [`Language::Brasa`], `{{ }}` markers become [`TokenKind::Variable`] everywhere they are
/// interpolated (request line, headers, query, auth, form and raw bodies) but not in scripts, docs
/// and `.env` files, where they are plain text. [`Language::Plain`] gives an empty list per line.
pub fn highlight(text: &str, language: Language) -> Vec<Vec<Token>> {
    let lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line));
    match language {
        Language::Plain => lines.map(|_| Vec::new()).collect(),
        Language::Env => lines.map(env_line).collect(),
        Language::Json => lines.map(json_line).collect(),
        Language::JavaScript => {
            let mut state = JsState::Normal;
            lines.map(|line| js_line(line, &mut state)).collect()
        }
        Language::Brasa => highlight_brasa(lines),
    }
}

/// Where the scan of a `.brasa` file is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Before the request line.
    Start,
    /// The header block.
    Headers,
    /// A `key = value` list, `#` disables an entry.
    KeyValue,
    /// The `auth` fields, `#` is a comment.
    Auth,
    /// A JSON body.
    Json,
    /// A `text` or `xml` body, plain except for the markers.
    Text,
    /// A script.
    Script,
    /// Docs, blank lines after the headers, unknown sections: no tokens.
    Nothing,
}

fn highlight_brasa<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<Vec<Token>> {
    let mut mode = Mode::Start;
    let mut js = JsState::Normal;
    lines
        .map(|line| {
            if let Some(rest) = line.strip_prefix(SECTION_PREFIX) {
                let mut words = rest.split_whitespace();
                mode = match (words.next(), words.next()) {
                    (Some("query"), _) | (Some("body"), Some("form")) => Mode::KeyValue,
                    (Some("auth"), _) => Mode::Auth,
                    (Some("body"), Some("json")) => Mode::Json,
                    (Some("body"), Some("text" | "xml")) => Mode::Text,
                    (Some("pre" | "post"), _) => Mode::Script,
                    _ => Mode::Nothing,
                };
                js = JsState::Normal;
                return vec![Token {
                    range: 0..line.len(),
                    kind: TokenKind::Section,
                }];
            }
            let tokens = match mode {
                Mode::Start => {
                    if line.trim().is_empty() {
                        return Vec::new();
                    }
                    mode = Mode::Headers;
                    request_line(line)
                }
                Mode::Headers => {
                    if line.trim().is_empty() {
                        mode = Mode::Nothing;
                        return Vec::new();
                    }
                    key_value_line(line, ':', TokenKind::HeaderName, TokenKind::Disabled)
                }
                Mode::KeyValue => key_value_line(line, '=', TokenKind::Key, TokenKind::Disabled),
                Mode::Auth => key_value_line(line, '=', TokenKind::Key, TokenKind::Comment),
                Mode::Json => json_line(line),
                Mode::Text => Vec::new(),
                Mode::Script => return js_line(line, &mut js),
                Mode::Nothing => return Vec::new(),
            };
            with_variables(line, tokens)
        })
        .collect()
}

/// `METHOD url`: the first word and the rest of the line, trimmed.
fn request_line(line: &str) -> Vec<Token> {
    let start = line.len() - line.trim_start().len();
    let rest = &line[start..];
    let method_end = start + rest.find(char::is_whitespace).unwrap_or(rest.len());
    let mut tokens = vec![Token {
        range: start..method_end,
        kind: TokenKind::Method,
    }];
    let url = &line[method_end..];
    let url_start = method_end + (url.len() - url.trim_start().len());
    let url_end = method_end + url.trim_end().len();
    if url_start < url_end {
        tokens.push(Token {
            range: url_start..url_end,
            kind: TokenKind::Url,
        });
    }
    tokens
}

/// A `key<separator>value` line, split on the first separator. A line starting with `#` is
/// entirely one `hash_kind` token.
fn key_value_line(
    line: &str,
    separator: char,
    key_kind: TokenKind,
    hash_kind: TokenKind,
) -> Vec<Token> {
    let start = line.len() - line.trim_start().len();
    let end = line.trim_end().len();
    if start >= end {
        return Vec::new();
    }
    if line[start..].starts_with('#') {
        return vec![Token {
            range: start..end,
            kind: hash_kind,
        }];
    }
    match line.find(separator) {
        Some(at) => {
            let key_end = line[..at].trim_end().len();
            let mut tokens = Vec::new();
            if start < key_end {
                tokens.push(Token {
                    range: start..key_end,
                    kind: key_kind,
                });
            }
            tokens.push(Token {
                range: at..at + separator.len_utf8(),
                kind: TokenKind::Punctuation,
            });
            tokens
        }
        // No separator: a header is invalid, a query key has an empty value.
        None if separator == '=' => vec![Token {
            range: start..end,
            kind: key_kind,
        }],
        None => Vec::new(),
    }
}

/// `.env`: `#` comments and `KEY=value`. Values are verbatim, so no variable markers.
fn env_line(line: &str) -> Vec<Token> {
    key_value_line(line, '=', TokenKind::Key, TokenKind::Comment)
}

/// The byte ranges of the `{{ ... }}` markers of `line`. An unmatched `{{` is not a marker.
fn marker_ranges(line: &str) -> Vec<Range<usize>> {
    let mut markers = Vec::new();
    let mut from = 0;
    while let Some(open) = line[from..].find("{{") {
        let open = from + open;
        let Some(close) = line[open + 2..].find("}}") else {
            break;
        };
        let end = open + 2 + close + 2;
        markers.push(open..end);
        from = end;
    }
    markers
}

/// Puts a [`TokenKind::Variable`] token over every marker, cutting the tokens they overlap.
fn with_variables(line: &str, tokens: Vec<Token>) -> Vec<Token> {
    if !line.contains("{{") {
        return tokens;
    }
    let markers = marker_ranges(line);
    if markers.is_empty() {
        return tokens;
    }
    let mut result = Vec::with_capacity(tokens.len() + markers.len());
    let mut first = 0;
    for token in tokens {
        while first < markers.len() && markers[first].end <= token.range.start {
            first += 1;
        }
        let mut cursor = token.range.start;
        for marker in markers[first..]
            .iter()
            .take_while(|m| m.start < token.range.end)
        {
            if cursor < marker.start {
                result.push(Token {
                    range: cursor..marker.start,
                    kind: token.kind,
                });
            }
            cursor = cursor.max(marker.end);
        }
        if cursor < token.range.end {
            result.push(Token {
                range: cursor..token.range.end,
                kind: token.kind,
            });
        }
    }
    result.extend(markers.into_iter().map(|range| Token {
        range,
        kind: TokenKind::Variable,
    }));
    result.sort_by_key(|token| token.range.start);
    result
}

/// Index after the closing `quote` of a literal whose content starts at `from`, or `None` when
/// the line ends first. A backslash skips the next byte.
fn scan_quoted(bytes: &[u8], from: usize, quote: u8) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// The end of a number starting at `from`: its first byte (a digit, or a JSON `-`), then digits,
/// letters, `_` and `.` (and a sign right after an exponent in JSON). Always past `from`.
fn scan_number(bytes: &[u8], from: usize, json: bool) -> usize {
    let mut i = from + 1;
    while i < bytes.len() {
        let b = bytes[i];
        let sign =
            json && matches!(b, b'+' | b'-') && i > from && matches!(bytes[i - 1], b'e' | b'E');
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_') || sign {
            i += 1;
        } else {
            break;
        }
    }
    i
}

fn scan_word(bytes: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'$')) {
        i += 1;
    }
    i
}

/// Appends a punctuation token, merging it into the previous one when they touch.
fn push_punctuation(tokens: &mut Vec<Token>, at: usize) {
    if let Some(last) = tokens.last_mut()
        && last.kind == TokenKind::Punctuation
        && last.range.end == at
    {
        last.range.end = at + 1;
        return;
    }
    tokens.push(Token {
        range: at..at + 1,
        kind: TokenKind::Punctuation,
    });
}

/// One line of JSON. Every line is scanned alone, a string never spans lines.
fn json_line(line: &str) -> Vec<Token> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let end = scan_quoted(bytes, i + 1, b'"')
                    .unwrap_or(bytes.len())
                    .min(bytes.len());
                let after = line[end..].trim_start_matches([' ', '\t']);
                let is_key = after.starts_with(':');
                let kind = if is_key {
                    TokenKind::Key
                } else {
                    TokenKind::String
                };
                tokens.push(Token {
                    range: i..end,
                    kind,
                });
                i = end;
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                push_punctuation(&mut tokens, i);
                i += 1;
            }
            b'-' | b'0'..=b'9' => {
                let end = scan_number(bytes, i, true);
                tokens.push(Token {
                    range: i..end,
                    kind: TokenKind::Number,
                });
                i = end;
            }
            b'a'..=b'z' | b'A'..=b'Z' => {
                let end = scan_word(bytes, i);
                if matches!(&line[i..end], "true" | "false" | "null") {
                    tokens.push(Token {
                        range: i..end,
                        kind: TokenKind::Boolean,
                    });
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    tokens
}

/// What a JavaScript line starts inside, carried from the previous line.
#[derive(Clone, Copy, PartialEq, Eq)]
enum JsState {
    Normal,
    BlockComment,
    Template,
}

const JS_KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "of",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
];

/// One line of JavaScript, updating `state` for the next one. Regex literals are not recognized.
fn js_line(line: &str, state: &mut JsState) -> Vec<Token> {
    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    match *state {
        JsState::Normal => {}
        JsState::BlockComment => match line.find("*/") {
            Some(at) => {
                i = at + 2;
                tokens.push(Token {
                    range: 0..i,
                    kind: TokenKind::Comment,
                });
                *state = JsState::Normal;
            }
            None => {
                if !line.is_empty() {
                    tokens.push(Token {
                        range: 0..line.len(),
                        kind: TokenKind::Comment,
                    });
                }
                return tokens;
            }
        },
        JsState::Template => match scan_quoted(bytes, 0, b'`') {
            Some(end) => {
                i = end.min(line.len());
                tokens.push(Token {
                    range: 0..i,
                    kind: TokenKind::String,
                });
                *state = JsState::Normal;
            }
            None => {
                if !line.is_empty() {
                    tokens.push(Token {
                        range: 0..line.len(),
                        kind: TokenKind::String,
                    });
                }
                return tokens;
            }
        },
    }
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                tokens.push(Token {
                    range: i..line.len(),
                    kind: TokenKind::Comment,
                });
                break;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => match line[i + 2..].find("*/") {
                Some(at) => {
                    let end = i + 2 + at + 2;
                    tokens.push(Token {
                        range: i..end,
                        kind: TokenKind::Comment,
                    });
                    i = end;
                }
                None => {
                    tokens.push(Token {
                        range: i..line.len(),
                        kind: TokenKind::Comment,
                    });
                    *state = JsState::BlockComment;
                    break;
                }
            },
            b'"' | b'\'' | b'`' => match scan_quoted(bytes, i + 1, b) {
                Some(end) => {
                    let end = end.min(line.len());
                    tokens.push(Token {
                        range: i..end,
                        kind: TokenKind::String,
                    });
                    i = end;
                }
                None => {
                    tokens.push(Token {
                        range: i..line.len(),
                        kind: TokenKind::String,
                    });
                    if b == b'`' {
                        *state = JsState::Template;
                    }
                    break;
                }
            },
            b'{' | b'}' | b'(' | b')' | b'[' | b']' | b',' => {
                push_punctuation(&mut tokens, i);
                i += 1;
            }
            b'0'..=b'9' => {
                let end = scan_number(bytes, i, false);
                tokens.push(Token {
                    range: i..end,
                    kind: TokenKind::Number,
                });
                i = end;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => {
                let end = scan_word(bytes, i);
                let word = &line[i..end];
                let is_property = i > 0 && bytes[i - 1] == b'.';
                if matches!(word, "true" | "false" | "null") {
                    tokens.push(Token {
                        range: i..end,
                        kind: TokenKind::Boolean,
                    });
                } else if !is_property && JS_KEYWORDS.contains(&word) {
                    tokens.push(Token {
                        range: i..end,
                        kind: TokenKind::Keyword,
                    });
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::TokenKind::*;
    use super::*;

    /// The tokens of a line as `(text, kind)` pairs, also checking the invariants.
    fn texts<'a>(line: &'a str, tokens: &[Token]) -> Vec<(&'a str, TokenKind)> {
        let mut previous_end = 0;
        for token in tokens {
            assert!(
                token.range.start >= previous_end,
                "overlap or unsorted in {line:?}"
            );
            assert!(
                token.range.start < token.range.end,
                "empty token in {line:?}"
            );
            assert!(
                line.is_char_boundary(token.range.start),
                "boundary in {line:?}"
            );
            assert!(
                line.is_char_boundary(token.range.end),
                "boundary in {line:?}"
            );
            previous_end = token.range.end;
        }
        tokens
            .iter()
            .map(|t| (&line[t.range.clone()], t.kind))
            .collect()
    }

    fn lines(text: &str, language: Language) -> Vec<Vec<(std::string::String, TokenKind)>> {
        let result = highlight(text, language);
        let source: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        assert_eq!(result.len(), source.len());
        result
            .iter()
            .zip(source)
            .map(|(tokens, line)| {
                texts(line, tokens)
                    .into_iter()
                    .map(|(s, k)| (s.to_string(), k))
                    .collect()
            })
            .collect()
    }

    const FULL: &str = "\
POST {{baseUrl}}/users?x=1
Content-Type: application/json
# X-Debug: {{debug}}
Authorization: Bearer {{ token }}

::: query
page = 1
# verbose = true
flag

::: auth bearer
# a comment
token = {{token}}

::: body json
{
  \"name\": \"{{userName}}\",
  \"n\": -1.5e+3, \"ok\": true, \"x\": null,
  \"list\": [1, 2]
}

::: pre
/* start
   middle */ const a = 1; // done
vars.set(\"id\", `multi
line ${a}`);

::: post
test(\"created\", () => { return res.status; });

::: docs
Some {{docs}} text
";

    fn line_of(
        result: &[Vec<(std::string::String, TokenKind)>],
        at: usize,
    ) -> Vec<(&str, TokenKind)> {
        result[at].iter().map(|(s, k)| (s.as_str(), *k)).collect()
    }

    #[test]
    fn request_line_headers_and_sections() {
        let r = lines(FULL, Language::Brasa);
        assert_eq!(
            line_of(&r, 0),
            [
                ("POST", Method),
                ("{{baseUrl}}", Variable),
                ("/users?x=1", Url)
            ]
        );
        assert_eq!(
            line_of(&r, 1),
            [("Content-Type", HeaderName), (":", Punctuation)]
        );
        assert_eq!(
            line_of(&r, 2),
            [("# X-Debug: ", Disabled), ("{{debug}}", Variable)]
        );
        assert_eq!(
            line_of(&r, 3),
            [
                ("Authorization", HeaderName),
                (":", Punctuation),
                ("{{ token }}", Variable)
            ]
        );
        assert!(r[4].is_empty());
        assert_eq!(line_of(&r, 5), [("::: query", Section)]);
        assert_eq!(line_of(&r, 6), [("page", Key), ("=", Punctuation)]);
        assert_eq!(line_of(&r, 7), [("# verbose = true", Disabled)]);
        assert_eq!(line_of(&r, 8), [("flag", Key)]);
        assert_eq!(line_of(&r, 10), [("::: auth bearer", Section)]);
        assert_eq!(line_of(&r, 11), [("# a comment", Comment)]);
        assert_eq!(
            line_of(&r, 12),
            [("token", Key), ("=", Punctuation), ("{{token}}", Variable)]
        );
        assert_eq!(line_of(&r, 14), [("::: body json", Section)]);
    }

    #[test]
    fn json_body() {
        let r = lines(FULL, Language::Brasa);
        assert_eq!(line_of(&r, 15), [("{", Punctuation)]);
        assert_eq!(
            line_of(&r, 16),
            [
                ("\"name\"", Key),
                (":", Punctuation),
                ("\"", String),
                ("{{userName}}", Variable),
                ("\"", String),
                (",", Punctuation),
            ]
        );
        let numbers = line_of(&r, 17);
        assert!(numbers.contains(&("-1.5e+3", Number)));
        assert!(numbers.contains(&("true", Boolean)));
        assert!(numbers.contains(&("null", Boolean)));
        assert_eq!(line_of(&r, 18)[2..4], [("[", Punctuation), ("1", Number)]);
    }

    #[test]
    fn scripts_docs_and_multiline_javascript() {
        let r = lines(FULL, Language::Brasa);
        assert_eq!(line_of(&r, 21), [("::: pre", Section)]);
        assert_eq!(line_of(&r, 22), [("/* start", Comment)]);
        assert_eq!(
            line_of(&r, 23),
            [
                ("   middle */", Comment),
                ("const", Keyword),
                ("1", Number),
                ("// done", Comment)
            ]
        );
        let set = line_of(&r, 24);
        assert!(set.contains(&("\"id\"", String)));
        assert!(set.contains(&("`multi", String)));
        assert_eq!(
            line_of(&r, 25),
            [("line ${a}`", String), (")", Punctuation)]
        );
        let post = line_of(&r, 28);
        assert!(post.contains(&("\"created\"", String)));
        assert!(post.contains(&("return", Keyword)));
        // docs: no tokens, markers are plain text there
        assert_eq!(line_of(&r, 30), [("::: docs", Section)]);
        assert!(r[31].is_empty());
    }

    #[test]
    fn text_body_only_has_variables() {
        let r = lines(
            "GET x\n\n::: body xml\n<a b=\"1\">{{v}}</a>\n",
            Language::Brasa,
        );
        assert_eq!(line_of(&r, 3), [("{{v}}", Variable)]);
    }

    #[test]
    fn form_body_and_apikey_auth() {
        let r = lines("GET x\n\n::: body form\nk = {{v}}\n", Language::Brasa);
        assert_eq!(
            line_of(&r, 3),
            [("k", Key), ("=", Punctuation), ("{{v}}", Variable)]
        );
    }

    #[test]
    fn header_value_keeps_colons_and_unmatched_marker_is_plain() {
        let r = lines(
            "GET x\nLocation: https://a.test:80/{{ oops\n",
            Language::Brasa,
        );
        assert_eq!(
            line_of(&r, 1),
            [("Location", HeaderName), (":", Punctuation)]
        );
    }

    #[test]
    fn escaped_marker_is_not_a_section() {
        let r = lines("GET x\n\n::: pre\n\\::: not a section\n", Language::Brasa);
        assert!(r[3].is_empty());
    }

    #[test]
    fn non_ascii_text() {
        let text = "GET https://ñ.test/{{ä}}\nX-Ñame: vålue\n\n::: body json\n{\"ключ\": \"日本語 \\\" é\", \"é\": 1}\n\n::: pre\nconst s = \"héllo\"; // ñandú\nlet é = 'ü';\n";
        // `lines` checks that every range is on a char boundary.
        let r = lines(text, Language::Brasa);
        assert_eq!(line_of(&r, 0)[2], ("{{ä}}", Variable));
        assert!(line_of(&r, 4).contains(&("\"ключ\"", Key)));
        assert!(line_of(&r, 7).contains(&("// ñandú", Comment)));
    }

    #[test]
    fn crlf_file() {
        let r = lines(
            "GET x\r\nA: b\r\n\r\n::: query\r\nk = v\r\n",
            Language::Brasa,
        );
        assert_eq!(line_of(&r, 0), [("GET", Method), ("x", Url)]);
        assert_eq!(line_of(&r, 1), [("A", HeaderName), (":", Punctuation)]);
        assert_eq!(line_of(&r, 3), [("::: query", Section)]);
        assert_eq!(line_of(&r, 4), [("k", Key), ("=", Punctuation)]);
    }

    #[test]
    fn empty_file() {
        assert_eq!(highlight("", Language::Brasa), vec![Vec::<Token>::new()]);
        assert_eq!(highlight("", Language::Env), vec![Vec::<Token>::new()]);
    }

    #[test]
    fn env_lines() {
        let r = lines("# comment\n  KEY = a=b {{x}}\nbare\n\nÑ=ü\n", Language::Env);
        assert_eq!(line_of(&r, 0), [("# comment", Comment)]);
        assert_eq!(line_of(&r, 1), [("KEY", Key), ("=", Punctuation)]);
        assert_eq!(line_of(&r, 2), [("bare", Key)]);
        assert!(r[3].is_empty());
        assert_eq!(line_of(&r, 4), [("Ñ", Key), ("=", Punctuation)]);
    }

    #[test]
    fn standalone_json_and_javascript() {
        let r = lines("{\"a\": [true, 1]}", Language::Json);
        assert_eq!(line_of(&r, 0)[0], ("{", Punctuation));
        let r = lines("/* a\nb */ let x = 'it\\'s'; // c\n", Language::JavaScript);
        assert_eq!(line_of(&r, 0), [("/* a", Comment)]);
        assert_eq!(
            line_of(&r, 1),
            [
                ("b */", Comment),
                ("let", Keyword),
                ("'it\\'s'", String),
                ("// c", Comment)
            ]
        );
    }

    #[test]
    fn plain_has_no_tokens() {
        assert_eq!(highlight("a\nb", Language::Plain), vec![vec![], vec![]]);
    }

    #[test]
    fn language_from_path() {
        assert_eq!(language_for_path("a/b/req.brasa"), Language::Brasa);
        assert_eq!(language_for_path("environments/dev.env"), Language::Env);
        assert_eq!(
            language_for_path("environments/dev.local.env"),
            Language::Env
        );
        assert_eq!(language_for_path(".env"), Language::Env);
        assert_eq!(language_for_path("x.JSON"), Language::Json);
        assert_eq!(language_for_path("x.js"), Language::JavaScript);
        assert_eq!(language_for_path("x.mjs"), Language::JavaScript);
        assert_eq!(language_for_path("README"), Language::Plain);
        assert_eq!(language_for_path("a.env/readme"), Language::Plain);
    }

    #[test]
    fn adversarial_input_does_not_panic() {
        for text in [
            "\"", "\\", "`\\", "/*", "{{", "}}{{", "::: ", "é\\é", "\"\\é", "'\\",
        ] {
            for language in [
                Language::Brasa,
                Language::Env,
                Language::Json,
                Language::JavaScript,
            ] {
                lines(text, language);
                lines(
                    &format!("GET x\n\n::: body json\n{text}\n::: pre\n{text}"),
                    language,
                );
            }
        }
    }
}
