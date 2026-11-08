//! The engine shared by the converters of Postman (`pm.*`) and Bruno (`bru.*`, `req.*`, `res.*`)
//! scripts to Brasa's script API (`docs/scripting.md`). Each source is a [`Dialect`]: the
//! rewrites it knows, and what is left of its API that has no equivalent.
//!
//! The source is split into top level statements, and every known call in a statement is
//! rewritten. A statement that still uses something Brasa does not have is kept as a comment,
//! so a script never fails on an API that does not exist. A script that cannot even be split,
//! because its brackets do not balance as this reads them, is commented out whole.
//!
//! This works on tokens outside strings and comments, without a JavaScript parser. It only has to
//! be safe: what it does not understand ends up commented out, never silently changed.

/// A source of scripts: what its converter rewrites and what it leaves as a comment.
pub(crate) trait Dialect {
    /// The first line of a script with commented out statements.
    fn partial_header(&self) -> &'static str;
    /// The names left after the rewrites that make a statement a comment.
    fn unsupported(&self) -> &'static [&'static str];
    /// Whether `code` sends requests of its own, the most common reason for a comment.
    fn sends_requests(&self, code: &str) -> bool;
    /// The rewrite of the use `rest` starts with: the new text and how much of `rest` it replaces.
    fn rewrite_at(&self, rest: &str) -> Option<(String, usize)>;
}

/// The result of [`convert_script`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConvertedScript {
    /// The script to write.
    pub(crate) text: String,
    /// How many statements it has, comments and blank lines left out.
    pub(crate) total: usize,
    /// How many of them were kept as code.
    pub(crate) converted: usize,
    /// Whether anything sent a request of its own, the most common reason for a comment.
    pub(crate) send_request: bool,
    /// Whether anything was rewritten.
    pub(crate) rewrote: bool,
}

/// Converts `source` with the rewrites of `dialect`. See the module docs.
pub(crate) fn convert_script(dialect: &impl Dialect, source: &str) -> ConvertedScript {
    let Some(segments) = split_statements(source) else {
        let code_lines = source
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        return ConvertedScript {
            text: comment_out(source),
            total: code_lines,
            converted: 0,
            send_request: dialect.sends_requests(source),
            rewrote: false,
        };
    };
    let mut text = String::new();
    let (mut total, mut converted) = (0, 0);
    let (mut send_request, mut rewrote) = (false, false);
    for segment in segments {
        if !has_code(segment) {
            text.push_str(segment);
            continue;
        }
        total += 1;
        let rewritten = rewrite(dialect, segment);
        if uses_unsupported(dialect, &rewritten) {
            send_request |= dialect.sends_requests(segment);
            text.push_str(&comment_out(segment));
            if segment.ends_with('\n') {
                text.push('\n');
            }
        } else {
            converted += 1;
            rewrote |= rewritten != segment;
            text.push_str(&rewritten);
        }
    }
    let text = text.trim_end().to_string();
    let text = if converted < total {
        format!("{}\n{text}", dialect.partial_header())
    } else {
        text
    };
    ConvertedScript {
        text,
        total,
        converted,
        send_request,
        rewrote,
    }
}

/// `source` with `// ` in front of every line.
fn comment_out(source: &str) -> String {
    source
        .trim_end_matches('\n')
        .lines()
        .map(|line| format!("// {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A position of a script as a scanner sees it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lexeme {
    /// Code: this byte starts a character outside strings and comments.
    Code,
    /// Inside a string or a comment.
    Skipped,
}

/// Classifies every byte of `source`.
fn scan(source: &str) -> Vec<Lexeme> {
    let bytes = source.as_bytes();
    let mut map = vec![Lexeme::Code; bytes.len()];
    let mut index = 0;
    while index < bytes.len() {
        let end = match bytes[index] {
            quote @ (b'"' | b'\'' | b'`') => {
                let mut end = index + 1;
                while end < bytes.len() && bytes[end] != quote {
                    end += if bytes[end] == b'\\' { 2 } else { 1 };
                }
                (end + 1).min(bytes.len())
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => source[index..]
                .find('\n')
                .map_or(bytes.len(), |offset| index + offset),
            b'/' if bytes.get(index + 1) == Some(&b'*') => source[index + 2..]
                .find("*/")
                .map_or(bytes.len(), |offset| index + 2 + offset + 2),
            _ => {
                index += 1;
                continue;
            }
        };
        map[index..end].fill(Lexeme::Skipped);
        index = end;
    }
    map
}

/// Whether `segment` has anything but blanks, comments and semicolons.
fn has_code(segment: &str) -> bool {
    let map = scan(segment);
    segment.char_indices().any(|(index, character)| {
        map[index] == Lexeme::Code && !character.is_whitespace() && character != ';'
    })
}

/// Splits `source` into top level statements, with blanks and comments as segments of their own
/// or attached to a statement. Concatenating them gives `source` back. `None` when the brackets
/// do not balance.
fn split_statements(source: &str) -> Option<Vec<&str>> {
    let map = scan(source);
    let mut segments = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (index, character) in source.char_indices() {
        if map[index] != Lexeme::Code {
            // A line comment ends at its newline, which the scanner leaves as code.
            continue;
        }
        match character {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            ';' if depth == 0 => {
                let end = source[index + 1..]
                    .find(|c: char| c == '\n' || !c.is_whitespace())
                    .map_or(source.len(), |offset| {
                        let at = index + 1 + offset;
                        if source[at..].starts_with('\n') {
                            at + 1
                        } else {
                            index + 1
                        }
                    });
                if end > start {
                    segments.push(&source[start..end]);
                    start = end;
                }
            }
            '\n' if depth == 0
                && index >= start
                && !continues(&source[start..index], &source[index + 1..]) =>
            {
                segments.push(&source[start..=index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    if start < source.len() {
        segments.push(&source[start..]);
    }
    Some(segments)
}

/// Whether a statement whose text so far is `before` goes on after the newline, in `after`.
fn continues(before: &str, after: &str) -> bool {
    if before.trim().is_empty() {
        return false;
    }
    const TRAILING: &[&str] = &[
        "=", "+", "-", "*", "/", ",", "?", ":", ".", "&&", "||", "=>", "!",
    ];
    let before = before.trim_end();
    if TRAILING.iter().any(|end| before.ends_with(end)) && !last_line_is_comment(before) {
        return true;
    }
    let next = after.trim_start();
    if next.starts_with("//") || next.starts_with("/*") {
        return false;
    }
    const LEADING: &[&str] = &[".", "?", ":", "&&", "||", "+", "-", "*", "/", "=", ")"];
    LEADING.iter().any(|start| next.starts_with(start))
        || ["else", "catch", "finally"]
            .iter()
            .any(|word| starts_with_word(next, word))
}

fn last_line_is_comment(text: &str) -> bool {
    text.lines()
        .last()
        .is_some_and(|line| line.trim_start().starts_with("//"))
}

fn starts_with_word(text: &str, word: &str) -> bool {
    text.strip_prefix(word)
        .is_some_and(|rest| !rest.starts_with(is_identifier_char))
}

pub(crate) fn is_identifier_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

/// Whether the code at `index` starts a new identifier: not in the middle of one, nor a property
/// of something else.
fn at_word_start(code: &str, index: usize) -> bool {
    !code[..index]
        .chars()
        .next_back()
        .is_some_and(|before| is_identifier_char(before) || before == '.')
}

/// Whether the rewritten `code` still names a part of the source API.
fn uses_unsupported(dialect: &impl Dialect, code: &str) -> bool {
    let map = scan(code);
    code.char_indices().any(|(index, _)| {
        map[index] == Lexeme::Code
            && at_word_start(code, index)
            && dialect
                .unsupported()
                .iter()
                .any(|name| starts_with_word(&code[index..], name))
    })
}

/// Rewrites every use of the source API in `code` that `dialect` knows.
pub(crate) fn rewrite(dialect: &impl Dialect, code: &str) -> String {
    let map = scan(code);
    let mut out = String::with_capacity(code.len());
    let mut index = 0;
    while index < code.len() {
        let rest = &code[index..];
        if map[index] == Lexeme::Code
            && at_word_start(code, index)
            && let Some((text, used)) = dialect.rewrite_at(rest)
        {
            out.push_str(&text);
            index += used;
            continue;
        }
        let character = rest.chars().next().unwrap_or_default();
        out.push(character);
        index += character.len_utf8().max(1);
    }
    out
}

/// Splits `text` at its only top level comma, ignoring one that ends it.
pub(crate) fn split_top_level_comma(text: &str) -> Option<(&str, &str)> {
    let map = scan(text);
    let mut depth = 0;
    for (index, character) in text.char_indices() {
        if map[index] != Lexeme::Code {
            continue;
        }
        match character {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                let second = text[index + 1..].trim().trim_end_matches(',');
                return Some((&text[..index], second));
            }
            _ => {}
        }
    }
    None
}

/// The text up to the `)` that closes an already open parenthesis, and how far past it `text` was
/// read. `None` when it never closes.
pub(crate) fn balanced(text: &str) -> Option<(&str, usize)> {
    let map = scan(text);
    let mut depth = 1;
    for (index, character) in text.char_indices() {
        if map[index] != Lexeme::Code {
            continue;
        }
        match character {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&text[..index], index + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Rewrites a Chai assertion, `<prefix>value).to...`, where `prefix` is `pm.expect(` or
/// `expect(`. Returns `None` for a chain Brasa has no matcher for.
pub(crate) fn rewrite_expect(
    dialect: &impl Dialect,
    rest: &str,
    prefix: &str,
) -> Option<(String, usize)> {
    let after_open = rest.strip_prefix(prefix)?;
    let (value, used) = balanced(after_open)?;
    let open_len = rest.len() - after_open.len();
    let chain_start = open_len + used;
    let (words, chain_end) = read_chain(&rest[chain_start..]);
    let mut negated = false;
    let mut deep = false;
    let mut terminal = None;
    for word in &words {
        match *word {
            "to" | "be" | "been" | "is" | "that" | "which" | "and" | "has" | "have" | "with"
            | "at" | "of" | "same" => {}
            "not" => negated = !negated,
            "deep" => deep = true,
            other => {
                if terminal.is_some() {
                    return None;
                }
                terminal = Some(other);
            }
        }
    }
    let after_chain = &rest[chain_start + chain_end..];
    let matcher = match terminal? {
        "equal" | "equals" | "eq" => {
            if deep {
                "toEqual"
            } else {
                "toBe"
            }
        }
        "eql" | "eqls" => "toEqual",
        "include" | "includes" | "contain" | "contains" => "toContain",
        "property" => "toHaveProperty",
        "above" | "gt" | "greaterThan" => "toBeGreaterThan",
        "below" | "lt" | "lessThan" => "toBeLessThan",
        "match" | "matches" => "toMatch",
        constant @ ("true" | "false" | "null" | "undefined") => {
            let not = if negated { ".not" } else { "" };
            let text = format!("expect({}){not}.toBe({constant})", rewrite(dialect, value));
            return Some((text, chain_start + chain_end));
        }
        "ok" => {
            let not = if negated { ".not" } else { "" };
            let text = format!("expect({}){not}.toBeTruthy()", rewrite(dialect, value));
            return Some((text, chain_start + chain_end));
        }
        "exist" => {
            let not = if negated { "" } else { ".not" };
            let text = format!("expect({}){not}.toBe(undefined)", rewrite(dialect, value));
            return Some((text, chain_start + chain_end));
        }
        _ => return None,
    };
    let after_paren = after_chain.strip_prefix('(')?;
    let not = if negated { ".not" } else { "" };
    let text = format!("expect({}){not}.{matcher}(", rewrite(dialect, value));
    Some((text, rest.len() - after_paren.len()))
}

/// The `.word.word` chain `text` starts with, line breaks allowed before a dot, and its length.
fn read_chain(text: &str) -> (Vec<&str>, usize) {
    let mut words = Vec::new();
    let mut index = 0;
    loop {
        let next = &text[index..];
        let trimmed = next.trim_start();
        let Some(after_dot) = trimmed.strip_prefix('.') else {
            return (words, index);
        };
        let length = after_dot
            .find(|character: char| !is_identifier_char(character))
            .unwrap_or(after_dot.len());
        if length == 0 {
            return (words, index);
        }
        words.push(&after_dot[..length]);
        index += next.len() - trimmed.len() + 1 + length;
    }
}
