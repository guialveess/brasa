//! Parsing and serialization of `.env` environment files.
//!
//! An `.env` file holds one flat list of variables: `KEY=value` per line, split on the first
//! `=`. This module only deals with the content of a single file; merging `<name>.local.env`
//! over `<name>.env` and attaching the environment name is the job of `brasa-workspace`.

use brasa_core::KeyValue;

/// An error found while parsing an `.env` file, with the 1-based line number where it was found.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {kind}")]
pub struct EnvParseError {
    /// The 1-based line number of the offending line.
    pub line: usize,
    /// What went wrong.
    pub kind: EnvParseErrorKind,
}

/// The specific problem found while parsing an `.env` file. See [`EnvParseError`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvParseErrorKind {
    /// A non-blank, non-comment line has no `=` to split the key from the value.
    #[error("missing '=' in {0:?}, expected \"KEY=value\"")]
    MissingEquals(String),
}

/// Parses the text of an `.env` file into its list of variables, in file order.
///
/// Both `\n` and `\r\n` line endings are accepted. Blank lines are ignored, and a line whose
/// first non-space character is `#` is a comment and is also ignored. Every other line is split
/// on the first `=`: the key is trimmed, the value is kept verbatim (no trimming, no quoting, no
/// escapes, no interpolation between variables, see `docs/format.md`). The returned
/// entries always have `enabled: true`, the `.env` format has no concept of a disabled entry.
pub fn parse(text: &str) -> Result<Vec<KeyValue>, EnvParseError> {
    let normalized = text.replace("\r\n", "\n");
    let mut variables = Vec::new();
    for (index, line) in normalized.split('\n').enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        match line.split_once('=') {
            Some((key, value)) => variables.push(KeyValue::new(key.trim(), value)),
            None => {
                return Err(EnvParseError {
                    line: index + 1,
                    kind: EnvParseErrorKind::MissingEquals(line.to_string()),
                });
            }
        }
    }
    Ok(variables)
}

/// Serializes a list of variables into `.env` file text, one `KEY=value` line per entry, in the
/// given order. The output always ends with a single trailing `\n`.
///
/// The `enabled` flag of each entry is ignored: callers are expected to pass only entries meant
/// to be written, since `.env` files cannot represent a disabled entry.
pub fn serialize(variables: &[KeyValue]) -> String {
    let mut out = String::new();
    for variable in variables {
        out.push_str(&variable.key);
        out.push('=');
        out.push_str(&variable.value);
        out.push('\n');
    }
    out
}

/// A single line of an `.env` file that keeps comments and blank lines, used to edit a file
/// without losing anything [`parse`]/[`serialize`] would drop (preserving the order and comments of other
/// lines). Those two functions above stay as they
/// are, they are enough for reading an environment to resolve variables, where comments do not
/// matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvLine {
    /// A `KEY=value` line.
    Variable(KeyValue),
    /// Any other line (a comment or a blank line), kept exactly as read.
    Verbatim(String),
}

/// Parses the text of an `.env` file into its lines, keeping comments and blank lines verbatim
/// (see [`EnvLine`]) instead of dropping them like [`parse`] does.
pub fn parse_lines(text: &str) -> Result<Vec<EnvLine>, EnvParseError> {
    let normalized = text.replace("\r\n", "\n");
    let mut input_lines: Vec<&str> = normalized.split('\n').collect();
    // `split('\n')` on text ending in "\n" yields a trailing empty element for that final,
    // absent line: drop it so `serialize_lines` does not grow the file by one blank line on
    // every round trip.
    if input_lines.last() == Some(&"") {
        input_lines.pop();
    }
    let mut lines = Vec::with_capacity(input_lines.len());
    for (index, line) in input_lines.into_iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            lines.push(EnvLine::Verbatim(line.to_string()));
            continue;
        }
        match line.split_once('=') {
            Some((key, value)) => lines.push(EnvLine::Variable(KeyValue::new(key.trim(), value))),
            None => {
                return Err(EnvParseError {
                    line: index + 1,
                    kind: EnvParseErrorKind::MissingEquals(line.to_string()),
                });
            }
        }
    }
    Ok(lines)
}

/// Serializes [`EnvLine`]s back into `.env` file text, one line per entry, each ending with `\n`.
pub fn serialize_lines(lines: &[EnvLine]) -> String {
    let mut out = String::new();
    for line in lines {
        match line {
            EnvLine::Variable(variable) => {
                out.push_str(&variable.key);
                out.push('=');
                out.push_str(&variable.value);
            }
            EnvLine::Verbatim(text) => out.push_str(text),
        }
        out.push('\n');
    }
    out
}

/// Sets `key` to `value` in `lines`, in place: replaces the value of the existing `key=value`
/// line if one is present, keeping its position and every other line untouched, or appends a new
/// `key=value` line at the end otherwise.
pub fn set_variable(lines: &mut Vec<EnvLine>, key: &str, value: &str) {
    let existing = lines.iter_mut().find_map(|line| match line {
        EnvLine::Variable(variable) if variable.key == key => Some(variable),
        _ => None,
    });
    match existing {
        Some(variable) => variable.value = value.to_string(),
        None => lines.push(EnvLine::Variable(KeyValue::new(key, value))),
    }
}

/// Removes the first `key=value` line for `key` from `lines`, in place, leaving every other line
/// (comments and blank lines included) untouched. Returns whether a line was removed.
pub fn remove_variable(lines: &mut Vec<EnvLine>, key: &str) -> bool {
    let position = lines
        .iter()
        .position(|line| matches!(line, EnvLine::Variable(variable) if variable.key == key));
    match position {
        Some(index) => {
            lines.remove(index);
            true
        }
        None => false,
    }
}

/// Renames the first `from` key to `to` in `lines`, in place, keeping the line's position and
/// value. Returns whether a line was renamed.
pub fn rename_variable(lines: &mut [EnvLine], from: &str, to: &str) -> bool {
    let existing = lines.iter_mut().find_map(|line| match line {
        EnvLine::Variable(variable) if variable.key == from => Some(variable),
        _ => None,
    });
    match existing {
        Some(variable) => {
            variable.key = to.to_string();
            true
        }
        None => false,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_simple_key_value_pairs() {
        let variables = parse("BASE_URL=https://api.test\nTOKEN=abc123\n").expect("valid env file");
        assert_eq!(
            variables,
            vec![
                KeyValue::new("BASE_URL", "https://api.test"),
                KeyValue::new("TOKEN", "abc123"),
            ]
        );
    }

    #[test]
    fn ignores_blank_lines_and_comments() {
        let variables = parse("# a comment\n\nA=1\n   \n# another\nB=2\n").expect("valid env file");
        assert_eq!(
            variables,
            vec![KeyValue::new("A", "1"), KeyValue::new("B", "2")]
        );
    }

    #[test]
    fn key_is_trimmed_but_value_is_kept_verbatim() {
        let variables = parse("  KEY  =  value with spaces  \n").expect("valid env file");
        assert_eq!(
            variables,
            vec![KeyValue::new("KEY", "  value with spaces  ")]
        );
    }

    #[test]
    fn value_may_contain_an_equals_sign() {
        let variables = parse("CONNECTION=host=localhost;port=5432\n").expect("valid env file");
        assert_eq!(
            variables,
            vec![KeyValue::new("CONNECTION", "host=localhost;port=5432")]
        );
    }

    #[test]
    fn accepts_crlf_line_endings() {
        let variables = parse("A=1\r\nB=2\r\n").expect("valid CRLF env file");
        assert_eq!(
            variables,
            vec![KeyValue::new("A", "1"), KeyValue::new("B", "2")]
        );
    }

    #[test]
    fn line_without_equals_is_an_error() {
        assert_eq!(
            parse("A=1\nnot a valid line\n"),
            Err(EnvParseError {
                line: 2,
                kind: EnvParseErrorKind::MissingEquals("not a valid line".to_string())
            })
        );
    }

    #[test]
    fn empty_file_parses_to_no_variables() {
        assert_eq!(parse(""), Ok(Vec::new()));
    }

    #[test]
    fn serialize_round_trips() {
        let variables = vec![
            KeyValue::new("BASE_URL", "https://api.test"),
            KeyValue::new("TOKEN", "abc123"),
        ];
        let text = serialize(&variables);
        assert_eq!(text, "BASE_URL=https://api.test\nTOKEN=abc123\n");
        assert_eq!(parse(&text).expect("valid serialized env file"), variables);
    }

    #[test]
    fn serialize_of_no_variables_is_an_empty_string() {
        assert_eq!(serialize(&[]), "");
    }

    #[test]
    fn parse_lines_keeps_comments_and_blank_lines_verbatim() {
        let lines = parse_lines("# a comment\nA=1\n\nB=2\n").expect("valid env file");
        assert_eq!(
            lines,
            vec![
                EnvLine::Verbatim("# a comment".to_string()),
                EnvLine::Variable(KeyValue::new("A", "1")),
                EnvLine::Verbatim(String::new()),
                EnvLine::Variable(KeyValue::new("B", "2")),
            ]
        );
    }

    #[test]
    fn parse_lines_round_trips_exactly() {
        let text = "# header\nA=1\n\nB=2\n# trailing\n";
        let lines = parse_lines(text).expect("valid env file");
        assert_eq!(serialize_lines(&lines), text);
    }

    #[test]
    fn parse_lines_of_empty_text_is_no_lines() {
        assert_eq!(parse_lines("").expect("valid env file"), Vec::new());
    }

    #[test]
    fn parse_lines_reports_the_same_error_as_parse() {
        assert_eq!(
            parse_lines("A=1\nnot a valid line\n"),
            Err(EnvParseError {
                line: 2,
                kind: EnvParseErrorKind::MissingEquals("not a valid line".to_string())
            })
        );
    }

    #[test]
    fn set_variable_replaces_an_existing_key_keeping_position_and_comments() {
        let mut lines = parse_lines("# header\nA=1\nB=2\n").expect("valid env file");
        set_variable(&mut lines, "A", "new-value");
        assert_eq!(
            serialize_lines(&lines),
            "# header\nA=new-value\nB=2\n".to_string()
        );
    }

    #[test]
    fn set_variable_appends_a_new_key_at_the_end() {
        let mut lines = parse_lines("A=1\n").expect("valid env file");
        set_variable(&mut lines, "B", "2");
        assert_eq!(serialize_lines(&lines), "A=1\nB=2\n");
    }

    #[test]
    fn set_variable_on_an_empty_file_creates_the_first_line() {
        let mut lines = parse_lines("").expect("valid env file");
        set_variable(&mut lines, "TOKEN", "secret");
        assert_eq!(serialize_lines(&lines), "TOKEN=secret\n");
    }

    #[test]
    fn remove_variable_drops_only_the_first_matching_line() {
        let mut lines = parse_lines("# top\nA=1\n\nB=2\nA=3\n").expect("valid");
        assert!(remove_variable(&mut lines, "A"));
        assert_eq!(serialize_lines(&lines), "# top\n\nB=2\nA=3\n");
        assert!(!remove_variable(&mut lines, "missing"));
    }

    #[test]
    fn rename_variable_keeps_position_and_value() {
        let mut lines = parse_lines("# top\nA=1\nB=2\n").expect("valid");
        assert!(rename_variable(&mut lines, "A", "C"));
        assert_eq!(serialize_lines(&lines), "# top\nC=1\nB=2\n");
        assert!(!rename_variable(&mut lines, "A", "D"));
    }
}
