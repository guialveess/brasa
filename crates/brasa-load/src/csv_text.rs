//! The bits of CSV this crate needs, written by hand: quoting a field when writing, and undoing
//! it when reading. Used by the exported requests table ([`crate::export::csv`]) and by the saved
//! sample log of [`crate::history`].

/// Appends `field` to `out`, wrapped in double quotes (with inner quotes doubled) only when it
/// contains a comma, a quote or a line break.
pub(crate) fn push_field(out: &mut String, field: &str) {
    if field.contains([',', '"', '\n', '\r']) {
        out.push('"');
        for c in field.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
    } else {
        out.push_str(field);
    }
}

/// Undoes [`push_field`] for one field: removes the outer quotes and un-doubles the inner ones.
/// A field without outer quotes is returned as it is.
pub(crate) fn unquote(field: &str) -> String {
    match field
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        Some(inner) => inner.replace("\"\"", "\""),
        None => field.to_string(),
    }
}

/// Whether `line` ends inside a quoted field, so the record continues on the next line: a field
/// with a line break in it is quoted, and an odd number of quotes means one is still open.
pub(crate) fn is_unterminated(line: &str) -> bool {
    line.bytes().filter(|&byte| byte == b'"').count() % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn quoted(field: &str) -> String {
        let mut out = String::new();
        push_field(&mut out, field);
        out
    }

    #[test]
    fn plain_fields_are_not_quoted() {
        assert_eq!(quoted("http://x/a?b=1"), "http://x/a?b=1");
    }

    #[test]
    fn commas_quotes_and_line_breaks_are_quoted() {
        assert_eq!(quoted("a,b"), "\"a,b\"");
        assert_eq!(quoted("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(quoted("a\nb"), "\"a\nb\"");
    }

    #[test]
    fn unquote_reverses_push_field() {
        for field in ["plain", "a,b", "say \"hi\"", "a\nb", "\"", ""] {
            assert_eq!(unquote(&quoted(field)), field);
        }
    }

    #[test]
    fn an_open_quote_continues_on_the_next_line() {
        assert!(is_unterminated("1,2,\"a"));
        assert!(!is_unterminated("1,2,\"a\""));
        assert!(!is_unterminated("1,2,a"));
    }
}
