//! Pretty printing of an HTML text without building a DOM of it, the same spirit as
//! [`super::json_pretty`]: a single text based pass, not a full parser, kept free of `gpui`
//! types so it is unit tested directly.
//!
//! Tag nesting drives the indentation. The content of `script`, `style`, `pre` and `textarea` is
//! copied verbatim, never reformatted or recursed into, since reformatting JavaScript, CSS or
//! whitespace significant text could change what it means.

/// Elements with no closing tag and no content, never indented into.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
    "source", "track", "wbr",
];
/// Elements whose content is copied as is, up to their matching closing tag.
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style", "pre", "textarea"];
/// The deepest nesting that is formatted, the same rationale and limit as `json_pretty`'s.
const MAX_DEPTH: usize = 127;

/// Pretty-prints `body` as HTML with `indent` spaces per nesting level, or `None` when it is not
/// valid UTF-8 or its trimmed start is not `<` (not markup), in which case the caller shows the
/// raw text.
pub fn pretty_print_html(body: &[u8], indent: usize) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    if !text.trim_start().starts_with('<') {
        return None;
    }
    reindent(text, indent)
}

/// Re-indents `text`, which must look like markup (see [`pretty_print_html`]), with `indent`
/// spaces per level. `None` when it is nested deeper than [`MAX_DEPTH`].
fn reindent(text: &str, indent: usize) -> Option<String> {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut depth = 0usize;
    let mut i = 0usize;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        if bytes[i] != b'<' {
            let next = text[i..].find('<').map_or(text.len(), |pos| i + pos);
            let chunk = text[i..next].trim();
            if !chunk.is_empty() {
                push_line(&mut out, depth, indent, chunk);
            }
            i = next;
            continue;
        }

        if text[i..].starts_with("<!--") {
            let end = find_after(text, i, "-->").unwrap_or(text.len());
            push_line(&mut out, depth, indent, text[i..end].trim());
            i = end;
            continue;
        }
        if text[i..].starts_with("<!") {
            let end = text[i..].find('>').map_or(text.len(), |pos| i + pos + 1);
            push_line(&mut out, depth, indent, &text[i..end]);
            i = end;
            continue;
        }

        let Some(tag_end) = text[i..].find('>').map(|pos| i + pos + 1) else {
            // An unclosed `<`: copy the rest verbatim rather than lose it.
            push_line(&mut out, depth, indent, text[i..].trim());
            break;
        };
        let tag_text = text[i..tag_end].trim();
        let is_closing = tag_text.starts_with("</");
        let is_self_closing = tag_text.trim_end_matches('>').trim_end().ends_with('/');
        let name = tag_name(tag_text, is_closing);

        if is_closing {
            depth = depth.saturating_sub(1);
            push_line(&mut out, depth, indent, tag_text);
            i = tag_end;
            continue;
        }

        push_line(&mut out, depth, indent, tag_text);
        let is_void = name.is_some_and(|name| is_one_of(name, VOID_ELEMENTS));
        if is_self_closing || is_void {
            i = tag_end;
            continue;
        }
        if let Some(name) = name
            && is_one_of(name, RAW_TEXT_ELEMENTS)
        {
            let closing = format!("</{}", name.to_ascii_lowercase());
            let rest = &text[tag_end..];
            let close_at = find_ascii_lowercase(rest, &closing).unwrap_or(rest.len());
            let content = rest[..close_at].trim_matches('\n');
            if !content.is_empty() {
                out.push_str(&pad(depth + 1, indent));
                out.push_str(content);
                out.push('\n');
            }
            i = tag_end + close_at;
            continue;
        }
        if depth >= MAX_DEPTH {
            return None;
        }
        depth += 1;
        i = tag_end;
    }
    Some(out)
}

/// `indent * depth` spaces.
fn pad(depth: usize, indent: usize) -> String {
    " ".repeat(depth * indent)
}

/// Appends `pad(depth, indent) + line + "\n"` to `out`.
fn push_line(out: &mut String, depth: usize, indent: usize, line: &str) {
    out.push_str(&pad(depth, indent));
    out.push_str(line);
    out.push('\n');
}

/// The position right after the first `needle` in `text[from..]`, or `None` when it does not
/// appear.
fn find_after(text: &str, from: usize, needle: &str) -> Option<usize> {
    text[from..]
        .find(needle)
        .map(|pos| from + pos + needle.len())
}

/// The position of the first case-insensitive match of `needle` (already lowercase) in `text`.
fn find_ascii_lowercase(text: &str, needle: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    lower.find(needle)
}

/// The tag name of `tag_text` (`<name ...>` or `</name>`), lowercased comparison done by the
/// caller.
fn tag_name(tag_text: &str, is_closing: bool) -> Option<&str> {
    let start = if is_closing { 2 } else { 1 };
    let rest = tag_text.get(start..)?;
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .unwrap_or(rest.len());
    let name = &rest[..end];
    (!name.is_empty()).then_some(name)
}

/// Whether `name` (as written in the markup) case-insensitively matches one of `names` (already
/// lowercase).
fn is_one_of(name: &str, names: &[&str]) -> bool {
    let lower = name.to_ascii_lowercase();
    names.contains(&lower.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn nested_elements_are_indented_one_level_per_depth() {
        let html = "<html><body><h1>Hi</h1></body></html>";
        assert_eq!(
            pretty_print_html(html.as_bytes(), 2),
            Some(
                "<html>\n  <body>\n    <h1>\n      Hi\n    </h1>\n  </body>\n</html>\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn void_elements_do_not_indent_their_siblings() {
        let html = "<div><br><img src=\"x\"><p>a</p></div>";
        let result = pretty_print_html(html.as_bytes(), 2).expect("markup");
        assert!(result.contains("  <br>\n  <img src=\"x\">\n  <p>\n"));
    }

    #[test]
    fn self_closing_tags_do_not_indent_their_siblings() {
        let html = "<svg><path d=\"M0\"/><circle/></svg>";
        let result = pretty_print_html(html.as_bytes(), 2).expect("markup");
        assert!(result.contains("  <path d=\"M0\"/>\n  <circle/>\n"));
    }

    #[test]
    fn script_content_is_kept_verbatim() {
        let html = "<html><script>if (a < b) { f(); }</script></html>";
        let result = pretty_print_html(html.as_bytes(), 2).expect("markup");
        assert!(result.contains("if (a < b) { f(); }"));
    }

    #[test]
    fn a_comment_and_a_doctype_are_their_own_line() {
        let html = "<!doctype html><!-- note --><p>hi</p>";
        let result = pretty_print_html(html.as_bytes(), 2).expect("markup");
        assert!(result.starts_with("<!doctype html>\n<!-- note -->\n<p>\n"));
    }

    #[test]
    fn text_not_starting_with_a_tag_is_not_markup() {
        assert_eq!(pretty_print_html(b"plain text", 2), None);
    }

    #[test]
    fn non_utf8_is_not_markup() {
        assert_eq!(pretty_print_html(&[0xff, 0xfe], 2), None);
    }
}
