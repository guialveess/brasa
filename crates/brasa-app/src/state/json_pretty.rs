//! Pretty printing of a JSON text without building a tree of it.
//!
//! The first version parsed the body into a `serde_json::Value` and wrote it back, which for a
//! 15 MB body peaked at about 110 MB of heap that the allocator then kept (#75). Here the text is
//! first validated with a streaming pass that keeps nothing (`IgnoredAny`), and then re-indented
//! token by token, so the extra memory is only the output.
//!
//! The text of every token is copied as it is: key order, duplicate keys, number spellings
//! (`1.0`, `1e5`, integers beyond 64 bits) and string escapes (`é`) reach the viewer exactly
//! as the server sent them, which a parse and write back would have normalized. Kept free of
//! `gpui` types so every rule is unit-tested directly.

use serde::de::IgnoredAny;

/// The deepest nesting that is formatted, the limit `serde_json` applies when it builds a tree
/// (the first version of this function). A deeper document is shown raw: it is not a real response,
/// and its indentation alone would take memory quadratic in the depth.
const MAX_DEPTH: usize = 127;

/// Pretty-prints `body` as JSON with `indent` spaces per nesting level (the `json_indent` setting),
/// or returns `None` when it is not valid UTF-8 or not valid JSON (or nested deeper than
/// [`MAX_DEPTH`]), in which case the caller shows the raw text.
pub fn pretty_print_json(body: &[u8], indent: usize) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    // Validation only: `IgnoredAny` visits every token and stores nothing, and `end` rejects
    // anything after the first value.
    let mut deserializer = serde_json::Deserializer::from_str(text);
    serde::Deserialize::deserialize(&mut deserializer)
        .map(|_: IgnoredAny| ())
        .ok()?;
    deserializer.end().ok()?;
    reindent(text, indent)
}

/// Re-indents `text`, which must be valid JSON, with `indent` spaces per level: whitespace between
/// tokens is dropped and rewritten, every token is copied verbatim. `None` when it is nested deeper
/// than [`MAX_DEPTH`].
fn reindent(text: &str, indent: usize) -> Option<String> {
    let bytes = text.as_bytes();
    // A formatted document is usually a third bigger than a compact one.
    let mut out = String::with_capacity(text.len() + text.len() / 3);
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    // The character after a backslash is skipped, so `\"` does not end the string.
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                out.push_str(&text[start..i]);
            }
            open @ (b'{' | b'[') => {
                if depth >= MAX_DEPTH {
                    return None;
                }
                let close = if open == b'{' { b'}' } else { b']' };
                let mut next = i + 1;
                while next < bytes.len() && is_whitespace(bytes[next]) {
                    next += 1;
                }
                out.push(char::from(open));
                if bytes.get(next) == Some(&close) {
                    // An empty object or array stays on one line.
                    out.push(char::from(close));
                    i = next + 1;
                } else {
                    depth += 1;
                    new_line(&mut out, depth, indent);
                    i += 1;
                }
            }
            close @ (b'}' | b']') => {
                depth = depth.saturating_sub(1);
                new_line(&mut out, depth, indent);
                out.push(char::from(close));
                i += 1;
            }
            b',' => {
                out.push(',');
                new_line(&mut out, depth, indent);
                i += 1;
            }
            b':' => {
                out.push_str(": ");
                i += 1;
            }
            byte if is_whitespace(byte) => i += 1,
            _ => {
                // A number, `true`, `false` or `null`: up to the next structural character.
                let start = i;
                while i < bytes.len() && !is_token_end(bytes[i]) {
                    i += 1;
                }
                out.push_str(&text[start..i]);
            }
        }
    }
    Some(out)
}

/// Starts a new line indented for `depth` levels of `indent` spaces.
fn new_line(out: &mut String, depth: usize, indent: usize) {
    out.push('\n');
    out.extend(std::iter::repeat_n(' ', depth * indent));
}

/// The four whitespace characters JSON allows between tokens.
fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

/// Whether `byte` ends a number or a literal.
fn is_token_end(byte: u8) -> bool {
    is_whitespace(byte) || matches!(byte, b',' | b':' | b'}' | b']')
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn pretty(text: &str) -> String {
        pretty_print_json(text.as_bytes(), 2).expect("valid JSON")
    }

    /// The previous implementation: parse into a tree and write it back.
    fn through_a_tree(text: &str) -> Option<String> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        serde_json::to_string_pretty(&value).ok()
    }

    #[test]
    fn indents_by_the_given_width() {
        assert_eq!(
            pretty_print_json(br#"{"a":[1,{"b":null}]}"#, 4).expect("valid JSON"),
            "{\n    \"a\": [\n        1,\n        {\n            \"b\": null\n        }\n    ]\n}"
        );
    }

    #[test]
    fn formats_nested_objects_and_arrays() {
        assert_eq!(
            pretty(r#"{"a":1,"b":[1,2,{"c":null}],"d":{"e":true,"f":"x"}}"#),
            "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2,\n    {\n      \"c\": null\n    }\n  ],\n  \"d\": {\n    \"e\": true,\n    \"f\": \"x\"\n  }\n}"
        );
    }

    #[test]
    fn empty_containers_stay_on_one_line() {
        assert_eq!(pretty("{}"), "{}");
        assert_eq!(pretty("[]"), "[]");
        assert_eq!(pretty("[ \n ]"), "[]");
        assert_eq!(
            pretty(r#"{"a":{},"b":[],"c":[{}]}"#),
            "{\n  \"a\": {},\n  \"b\": [],\n  \"c\": [\n    {}\n  ]\n}"
        );
    }

    #[test]
    fn top_level_scalars() {
        assert_eq!(pretty("42"), "42");
        assert_eq!(pretty(" \"text\" "), "\"text\"");
        assert_eq!(pretty("null"), "null");
        assert_eq!(pretty("\ntrue\n"), "true");
        assert_eq!(pretty("-1.5e3"), "-1.5e3");
    }

    #[test]
    fn any_whitespace_between_tokens_is_normalized() {
        let messy = "{\r\n\t\"a\" :\t1 ,\n\n \"b\"\r:  [ 1 ,2 ]\n}\n";
        assert_eq!(pretty(messy), pretty(r#"{"a":1,"b":[1,2]}"#));
    }

    #[test]
    fn structural_characters_inside_strings_are_left_alone() {
        let text = r#"{"k{ey}":"a,b:c [d] {e} \" \\ end","x":"  spaced  "}"#;
        assert_eq!(
            pretty(text),
            "{\n  \"k{ey}\": \"a,b:c [d] {e} \\\" \\\\ end\",\n  \"x\": \"  spaced  \"\n}"
        );
    }

    #[test]
    fn a_string_ending_in_an_escaped_backslash_does_end() {
        // `\\` before the closing quote is an escaped backslash, not an escaped quote.
        assert_eq!(pretty(r#"["a\\","b"]"#), "[\n  \"a\\\\\",\n  \"b\"\n]");
    }

    #[test]
    fn tokens_are_copied_exactly_as_sent() {
        let text = r#"{"b":1,"a":2,"a":3,"n":[1.0,1e5,1E+2,12345678901234567890123,-0,0.10],"s":"é\n\/","e":"é"}"#;
        let result = pretty(text);
        for kept in [
            "1.0",
            "1e5",
            "1E+2",
            "12345678901234567890123",
            "-0",
            "0.10",
            r#""é\n\/""#,
            r#""é""#,
        ] {
            assert!(result.contains(kept), "{kept} was changed in {result}");
        }
        // Key order and the duplicate key are kept too.
        let positions: Vec<usize> = [r#""b": 1"#, r#""a": 2"#, r#""a": 3"#]
            .iter()
            .map(|key| result.find(key).expect("key present"))
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn formatting_twice_changes_nothing() {
        let once = pretty(r#"{"a":[1,{"b":[]},"x"],"c":{}}"#);
        assert_eq!(pretty(&once), once);
    }

    #[test]
    fn rejects_what_is_not_json() {
        for text in [
            "",
            "   ",
            "not json",
            "{",
            "[1,2",
            r#"{"a":}"#,
            r#"{"a":1,}"#,
            "[1 2]",
            "{'a':1}",
            r#"{"a":1} trailing"#,
            "1 2",
            "\"unterminated",
            "nul",
            "[01]",
        ] {
            assert_eq!(pretty_print_json(text.as_bytes(), 2), None, "{text:?}");
        }
    }

    #[test]
    fn rejects_non_utf8() {
        assert_eq!(pretty_print_json(&[b'"', 0xff, 0xfe, b'"'], 2), None);
        assert_eq!(
            pretty_print_json(&[0xef, 0xbb, 0xbf, b'1'], 2),
            None,
            "a BOM is not JSON"
        );
    }

    #[test]
    fn nesting_limit_is_the_same_as_before() {
        for depth in [100, 127, 128, 129, 200] {
            let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
            assert_eq!(
                pretty_print_json(text.as_bytes(), 2).is_some(),
                through_a_tree(&text).is_some(),
                "depth {depth}"
            );
        }
    }

    /// A small deterministic generator, so the property tests need no crate and never flake.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// A random compact JSON value of limited depth, with distinct keys, integers and plain
    /// strings, so the tree based version writes it back unchanged.
    fn random_json(rng: &mut Rng, depth: usize, out: &mut String) {
        let kind = if depth == 0 {
            rng.below(4)
        } else {
            rng.below(6)
        };
        match kind {
            0 => out.push_str(&(rng.below(2000) as i64 - 1000).to_string()),
            1 => out.push_str(["true", "false", "null"][rng.below(3) as usize]),
            2 | 3 => {
                let words = [
                    "",
                    "a",
                    "hello world",
                    "a,b:c",
                    "{x}",
                    "[y]",
                    "tab\\t",
                    "q\\\"q",
                    "ñandú",
                ];
                out.push('"');
                out.push_str(words[rng.below(words.len() as u64) as usize]);
                out.push('"');
            }
            4 => {
                out.push('[');
                for index in 0..rng.below(4) {
                    if index > 0 {
                        out.push(',');
                    }
                    random_json(rng, depth - 1, out);
                }
                out.push(']');
            }
            _ => {
                out.push('{');
                for index in 0..rng.below(4) {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&format!("\"key{index}\":"));
                    random_json(rng, depth - 1, out);
                }
                out.push('}');
            }
        }
    }

    #[test]
    fn matches_the_tree_based_version_on_random_documents() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for case in 0..500 {
            let mut text = String::new();
            random_json(&mut rng, 5, &mut text);
            assert_eq!(
                Some(pretty(&text)),
                through_a_tree(&text),
                "case {case}: {text}"
            );
        }
    }

    #[test]
    fn keeps_the_meaning_of_documents_with_odd_spellings() {
        let documents = [
            r#"{"b":1,"a":[1.0,1e5,"é",{"c":false}],"a2":{}}"#,
            "[ 1 ,\n 2e-3 ,\t \"x\\\"y\" , null ]",
            r#"{"nested":{"deep":{"deeper":[[],[[]],[{}]]}}}"#,
        ];
        for text in documents {
            let before: serde_json::Value = serde_json::from_str(text).expect("valid");
            let after: serde_json::Value = serde_json::from_str(&pretty(text)).expect("valid");
            assert_eq!(before, after, "{text}");
        }
    }

    #[test]
    fn a_big_body_is_formatted_like_before() {
        // About 5 MB, the size of the sample suite's `responses/large-5mb`.
        let mut text = String::from(r#"{"items":["#);
        for index in 0..60_000 {
            if index > 0 {
                text.push(',');
            }
            text.push_str(&format!(
                r#"{{"index":{index},"text":"lorem ipsum dolor sit amet, consectetur adipiscing elit"}}"#
            ));
        }
        text.push_str("]}");
        assert!(text.len() > 4_000_000);
        assert_eq!(Some(pretty(&text)), through_a_tree(&text));
    }

    /// How long pretty printing a response near `brasa_http::DEFAULT_MAX_RESPONSE_SIZE` (20 MB)
    /// actually takes, the number `views/response_view.rs`'s `AppView::ensure_pretty` doc
    /// comment points at to explain why this never runs on the window's own thread. `cargo test
    /// -p brasa-app --lib -- --ignored --nocapture big_body_pretty_print_timing` to see it.
    #[test]
    #[ignore = "a timing measurement, not a correctness check; run explicitly"]
    fn big_body_pretty_print_timing() {
        let mut text = String::from(r#"{"items":["#);
        for index in 0..230_000 {
            if index > 0 {
                text.push(',');
            }
            text.push_str(&format!(
                r#"{{"index":{index},"text":"lorem ipsum dolor sit amet, consectetur adipiscing elit"}}"#
            ));
        }
        text.push_str("]}");
        println!("body size: {} bytes", text.len());
        let started = std::time::Instant::now();
        let result = pretty_print_json(text.as_bytes(), 2);
        println!("pretty_print_json: {:?}", started.elapsed());
        assert!(result.is_some());
    }
}
