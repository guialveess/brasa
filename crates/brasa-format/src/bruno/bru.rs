//! A reader for Bruno's `.bru` markup, following the grammar of `bruno-lang` v2 (`bruToJson.js`
//! and `envToJson.js` in usebruno/bruno). A file is a list of top level blocks:
//!
//! - dictionary blocks, `name {` then one `key: value` per line and a `}` alone on its line. A key
//!   starting with `~` is disabled, a key with spaces or colons is quoted (`"a:b"`), and a value may
//!   be a `'''` multi-line text or a `[ ... ]` list;
//! - text blocks (bodies, scripts, tests, docs), whose lines are indented by two spaces;
//! - list blocks, `name [` then comma or newline separated items and a `]`;
//! - single lines, `name: value` (`color` and `extends` of an environment).
//!
//! Lines starting with an `@annotation(...)` (descriptions) are skipped. Anything else that does
//! not fit is skipped too: a reader for an import has to be forgiving, not strict.

/// One `key: value` entry of a dictionary block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BruPair {
    /// The key, unquoted, without its `~`.
    pub(super) key: String,
    /// The value: the rest of the line trimmed, or the text of a `'''` block. A list value is
    /// joined with newlines.
    pub(super) value: String,
    /// `false` when the key started with `~`.
    pub(super) enabled: bool,
}

/// The content of one top level block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BruContent {
    /// `key: value` entries.
    Dict(Vec<BruPair>),
    /// Free text, outdented.
    Text(String),
    /// List items.
    List(Vec<String>),
    /// A single `name: value` line.
    Line(String),
}

/// One top level block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BruBlock {
    /// The block name (`meta`, `get`, `headers`, `body:json`, `auth:bearer`, ...).
    pub(super) name: String,
    /// What it holds.
    pub(super) content: BruContent,
}

/// A parsed `.bru` file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct BruFile {
    /// The blocks, in file order.
    pub(super) blocks: Vec<BruBlock>,
}

impl BruFile {
    /// The first block named `name`.
    pub(super) fn block(&self, name: &str) -> Option<&BruContent> {
        self.blocks
            .iter()
            .find(|block| block.name == name)
            .map(|block| &block.content)
    }

    /// The entries of the dictionary block `name`, empty when there is none.
    pub(super) fn dict(&self, name: &str) -> &[BruPair] {
        match self.block(name) {
            Some(BruContent::Dict(pairs)) => pairs,
            _ => &[],
        }
    }

    /// The value of the enabled `key` in the dictionary block `name`.
    pub(super) fn value(&self, name: &str, key: &str) -> Option<&str> {
        self.dict(name)
            .iter()
            .find(|pair| pair.enabled && pair.key == key)
            .map(|pair| pair.value.as_str())
    }

    /// The text of the text block `name`.
    pub(super) fn text(&self, name: &str) -> Option<&str> {
        match self.block(name) {
            Some(BruContent::Text(text)) => Some(text),
            _ => None,
        }
    }

    /// The items of the list block `name`, empty when there is none.
    pub(super) fn list(&self, name: &str) -> &[String] {
        match self.block(name) {
            Some(BruContent::List(items)) => items,
            _ => &[],
        }
    }

    /// The value of the single line `name: value`.
    pub(super) fn line(&self, name: &str) -> Option<&str> {
        match self.block(name) {
            Some(BruContent::Line(value)) => Some(value),
            _ => None,
        }
    }
}

/// Whether the block `name` holds free text rather than `key: value` entries.
fn is_text_block(name: &str) -> bool {
    matches!(name, "tests" | "docs" | "example")
        || name.starts_with("script:")
        || (name == "body"
            || name.starts_with("body:")
                && !matches!(
                    name,
                    "body:form-urlencoded"
                        | "body:multipart-form"
                        | "body:file"
                        | "body:grpc"
                        | "body:ws"
                ))
}

/// Parses the text of a `.bru` file.
pub(super) fn parse_bru(text: &str) -> BruFile {
    let lines: Vec<&str> = text
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() || line.starts_with([' ', '\t']) {
            continue;
        }
        if let Some(name) = trimmed.strip_suffix('{') {
            let name = name.trim().to_string();
            let body = take_until(&lines, &mut index, "}");
            let content = if is_text_block(&name) {
                BruContent::Text(outdent(&body, 2))
            } else {
                BruContent::Dict(parse_pairs(&body))
            };
            blocks.push(BruBlock { name, content });
        } else if let Some(name) = trimmed.strip_suffix('[') {
            let name = name.trim().to_string();
            let body = take_until(&lines, &mut index, "]");
            blocks.push(BruBlock {
                name,
                content: BruContent::List(list_items(&body)),
            });
        } else if let Some((name, items)) = one_line_list(trimmed) {
            blocks.push(BruBlock {
                name: name.to_string(),
                content: BruContent::List(list_items(&[items])),
            });
        } else if let Some((name, value)) = trimmed.split_once(':') {
            let value = value.trim();
            blocks.push(BruBlock {
                name: name.trim().to_string(),
                content: BruContent::Line(value.to_string()),
            });
        }
    }
    BruFile { blocks }
}

/// A list written on one line, `vars:secret [a, b]`: its name and what is between the brackets.
fn one_line_list(line: &str) -> Option<(&str, &str)> {
    let (name, rest) = line.split_once('[')?;
    let items = rest.strip_suffix(']')?;
    let name = name.trim();
    (!name.is_empty() && !name.contains(char::is_whitespace)).then_some((name, items))
}

/// The lines after `index` up to the one that is exactly `end` (at column 0, as Bruno writes
/// it), which is consumed. Without one, everything that is left.
fn take_until<'a>(lines: &[&'a str], index: &mut usize, end: &str) -> Vec<&'a str> {
    let start = *index;
    while *index < lines.len() {
        if lines[*index].trim_end() == end {
            let body = lines[start..*index].to_vec();
            *index += 1;
            return body;
        }
        *index += 1;
    }
    lines[start..].to_vec()
}

/// `lines` with up to `spaces` leading spaces removed from each, joined, without the blank lines
/// that open and close the block.
fn outdent(lines: &[&str], spaces: usize) -> String {
    let outdented: Vec<&str> = lines
        .iter()
        .map(|line| {
            let strip = line.len() - line.trim_start_matches(' ').len();
            &line[strip.min(spaces)..]
        })
        .collect();
    let first = outdented.iter().position(|line| !line.trim().is_empty());
    let last = outdented.iter().rposition(|line| !line.trim().is_empty());
    match (first, last) {
        (Some(first), Some(last)) => outdented[first..=last].join("\n"),
        _ => String::new(),
    }
}

/// The items of a list block: separated by commas or line breaks, trimmed, empty ones dropped.
fn list_items(lines: &[&str]) -> Vec<String> {
    lines
        .iter()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|item| !item.is_empty() && !item.starts_with('@'))
        .map(str::to_string)
        .collect()
}

/// Whether the trimmed `line` is an `@annotation(...)` rather than a key starting with `@` (a
/// local post-response variable, `@name: value`).
fn is_annotation(line: &str) -> bool {
    if !line.starts_with('@') {
        return false;
    }
    match (line.find('('), line.find(':')) {
        (Some(paren), Some(colon)) => paren < colon,
        (_, None) => true,
        (None, Some(_)) => false,
    }
}

/// Parses the lines of a dictionary block.
fn parse_pairs(lines: &[&str]) -> Vec<BruPair> {
    let mut pairs = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim();
        index += 1;
        if line.is_empty() {
            continue;
        }
        if is_annotation(line) {
            // An argument written as a `'''` block may go on for several lines.
            if line.matches("'''").count() == 1 {
                while index < lines.len() && !lines[index].contains("'''") {
                    index += 1;
                }
                index += 1;
            }
            continue;
        }
        let Some((key, enabled, rest)) = split_key(line) else {
            continue;
        };
        let rest = rest.trim();
        let value = if let Some(after) = rest.strip_prefix("'''") {
            if let Some(end) = after.find("'''") {
                after[..end].to_string()
            } else {
                let mut body = Vec::new();
                while index < lines.len() {
                    let current = lines[index];
                    index += 1;
                    if current.trim_start().starts_with("'''") {
                        break;
                    }
                    body.push(current);
                }
                outdent(&body, 4)
            }
        } else if rest == "[" {
            let mut items = Vec::new();
            while index < lines.len() {
                let current = lines[index].trim();
                index += 1;
                if current == "]" {
                    break;
                }
                if !current.is_empty() {
                    items.push(current);
                }
            }
            items.join("\n")
        } else {
            rest.to_string()
        };
        pairs.push(BruPair {
            key,
            value,
            enabled,
        });
    }
    pairs
}

/// Splits `line` into its key (unquoted, without `~`), whether it is enabled, and what follows the
/// colon. `None` when there is no colon.
fn split_key(line: &str) -> Option<(String, bool, &str)> {
    let (enabled, line) = match line.strip_prefix('~') {
        Some(rest) => (false, rest),
        None => (true, line),
    };
    if let Some(quoted) = line.strip_prefix('"') {
        let mut key = String::new();
        let mut chars = quoted.char_indices();
        while let Some((at, character)) = chars.next() {
            match character {
                '\\' => {
                    if let Some((_, escaped)) = chars.next() {
                        key.push(escaped);
                    }
                }
                '"' => {
                    let rest = quoted[at + 1..].trim_start().strip_prefix(':')?;
                    return Some((key, enabled, rest));
                }
                other => key.push(other),
            }
        }
        return None;
    }
    let (key, rest) = line.split_once(':')?;
    Some((key.trim().to_string(), enabled, rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn pair(key: &str, value: &str, enabled: bool) -> BruPair {
        BruPair {
            key: key.to_string(),
            value: value.to_string(),
            enabled,
        }
    }

    #[test]
    fn dictionaries_with_quoted_and_disabled_keys() {
        let file = parse_bru(
            "params:query {\n  apiKey: secret\n  \"key with spaces\": is allowed\n  \"nested escaped \\\"quote\\\"\": yes\n  ~\"disabled:colon\": no\n  ~message: hello\n  url: https://a.test:8080/x\n}\n",
        );
        assert_eq!(
            file.dict("params:query"),
            &[
                pair("apiKey", "secret", true),
                pair("key with spaces", "is allowed", true),
                pair("nested escaped \"quote\"", "yes", true),
                pair("disabled:colon", "no", false),
                pair("message", "hello", false),
                pair("url", "https://a.test:8080/x", true),
            ]
        );
        assert_eq!(file.value("params:query", "message"), None);
    }

    #[test]
    fn text_blocks_are_outdented() {
        let file = parse_bru(
            "body:json {\n  {\n    \"hello\": \"world\"\n  }\n}\n\nscript:pre-request {\n  const a = 1;\n  if (a) {\n    log(a);\n  }\n}\n\ndocs {\n  # Title\n\n  Text.\n}\n",
        );
        assert_eq!(file.text("body:json"), Some("{\n  \"hello\": \"world\"\n}"));
        assert_eq!(
            file.text("script:pre-request"),
            Some("const a = 1;\nif (a) {\n  log(a);\n}")
        );
        assert_eq!(file.text("docs"), Some("# Title\n\nText."));
    }

    #[test]
    fn meta_lists_annotations_and_multiline_values() {
        let file = parse_bru(
            "meta {\n  name: Send SMS\n  seq: 2\n  tags: [\n    foo\n    bar\n  ]\n}\n\nvars {\n  @description('''Base URL.''')\n  url: http://localhost\n  @description(\"x\")\n  note: '''\n    line one\n    line two\n  '''\n}\n",
        );
        assert_eq!(file.value("meta", "name"), Some("Send SMS"));
        assert_eq!(file.value("meta", "tags"), Some("foo\nbar"));
        assert_eq!(
            file.dict("vars"),
            &[
                pair("url", "http://localhost", true),
                pair("note", "line one\nline two", true),
            ]
        );
    }

    #[test]
    fn local_variables_are_not_annotations() {
        let file = parse_bru(
            "vars:post-response {\n  token: $res.body.token\n  @orderNumber: $res.body.orderNumber\n  ~@id: $res.body.id\n}\n",
        );
        assert_eq!(
            file.dict("vars:post-response"),
            &[
                pair("token", "$res.body.token", true),
                pair("@orderNumber", "$res.body.orderNumber", true),
                pair("@id", "$res.body.id", false),
            ]
        );
    }

    #[test]
    fn environments_with_secrets_color_and_crlf() {
        let file = parse_bru(
            "vars {\r\n  host: https://a.test\r\n  ~old: 1\r\n}\r\nvars:secret [\r\n  token,\r\n  password\r\n]\r\ncolor: #4CAF50\r\n",
        );
        assert_eq!(
            file.dict("vars"),
            &[
                pair("host", "https://a.test", true),
                pair("old", "1", false)
            ]
        );
        assert_eq!(file.list("vars:secret"), &["token", "password"]);
        assert_eq!(file.line("color"), Some("#4CAF50"));

        let one_line = parse_bru("vars:secret [token, key]\nextends: base\n");
        assert_eq!(one_line.list("vars:secret"), &["token", "key"]);
        assert_eq!(one_line.line("extends"), Some("base"));
    }

    #[test]
    fn an_empty_block_and_a_block_without_its_end() {
        let file = parse_bru("headers {\n}\n\ndocs {\n  unfinished\n");
        assert_eq!(file.dict("headers"), &[]);
        assert_eq!(file.text("docs"), Some("unfinished"));
    }
}
