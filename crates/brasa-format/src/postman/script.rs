//! Converting Postman `pm.*` scripts to Brasa's script API (`docs/scripting.md`), on the engine
//! of [`crate::script_convert`]. A statement that still uses something Brasa does not have
//! (`pm.sendRequest`, `CryptoJS`, ...) is kept as a comment.

use crate::script_convert::{
    self, ConvertedScript, Dialect, balanced, is_identifier_char, rewrite, rewrite_expect,
    split_top_level_comma,
};

/// The first line of a script with commented out statements.
pub(super) const PARTIAL_HEADER: &str =
    "// Imported from Postman. What could not be converted is commented out below.";

/// Rewrites without an argument: the text after `pm.` to its replacement. Longest first where
/// one is the prefix of another.
const REWRITES: &[(&str, &str)] = &[
    ("pm.test(", "test("),
    (
        "pm.response.to.be.ok",
        "expect(res.status).toBeLessThan(300)",
    ),
    (
        "pm.response.to.be.success",
        "expect(res.status).toBeLessThan(300)",
    ),
    ("pm.response.code", "res.status"),
    ("pm.response.json()", "res.json()"),
    ("pm.response.text()", "res.body"),
    ("pm.response.headers.get(", "res.headers.get("),
    ("pm.response.responseTime", "res.timeMs"),
    ("pm.environment.set(", "env.set("),
    ("pm.environment.get(", "env.get("),
    ("pm.environment.unset(", "env.unset("),
    ("pm.collectionVariables.set(", "env.set("),
    ("pm.collectionVariables.get(", "env.get("),
    ("pm.collectionVariables.unset(", "env.unset("),
    ("pm.globals.set(", "env.set("),
    ("pm.globals.get(", "env.get("),
    ("pm.globals.unset(", "env.unset("),
    ("pm.variables.set(", "vars.set("),
    ("pm.request.url", "req.url"),
    ("pm.request.method", "req.method"),
    ("postman.setEnvironmentVariable(", "env.set("),
    ("postman.getEnvironmentVariable(", "env.get("),
    ("postman.clearEnvironmentVariable(", "env.unset("),
];

/// What is left of Postman's API after the rewrites, each making its statement a comment.
const UNSUPPORTED: &[&str] = &[
    "pm",
    "postman",
    "CryptoJS",
    "require",
    "responseBody",
    "responseCode",
    "responseTime",
    "responseHeaders",
    "tests",
    "xml2Json",
    "cheerio",
    "tv4",
    "atob",
    "btoa",
];

/// Postman's scripts.
struct Postman;

impl Dialect for Postman {
    fn partial_header(&self) -> &'static str {
        PARTIAL_HEADER
    }

    fn unsupported(&self) -> &'static [&'static str] {
        UNSUPPORTED
    }

    fn sends_requests(&self, code: &str) -> bool {
        code.contains("pm.sendRequest")
    }

    fn rewrite_at(&self, rest: &str) -> Option<(String, usize)> {
        postman_rewrite_at(rest)
    }
}

/// Converts a Postman script. See the module docs.
pub(super) fn convert_script(source: &str) -> ConvertedScript {
    script_convert::convert_script(&Postman, source)
}

/// The rewrite of the `pm.*` use `rest` starts with: the new text and how much of `rest` it
/// replaces.
fn postman_rewrite_at(rest: &str) -> Option<(String, usize)> {
    if rest.starts_with("pm.expect(") {
        return rewrite_expect(&Postman, rest, "pm.expect(");
    }
    if let Some(after) = rest.strip_prefix("pm.response.to.have.status(") {
        let (argument, used) = balanced(after)?;
        let text = format!("expect(res.status).toBe({})", rewrite(&Postman, argument));
        return Some((text, rest.len() - after.len() + used));
    }
    if let Some(after) = rest.strip_prefix("pm.variables.get(") {
        let (argument, used) = balanced(after)?;
        let argument = rewrite(&Postman, argument);
        let text = format!("(vars.get({argument}) ?? env.get({argument}))");
        return Some((text, rest.len() - after.len() + used));
    }
    if let Some(after) = rest.strip_prefix("pm.request.headers.add(") {
        let (argument, used) = balanced(after)?;
        let (key, value) = header_object(argument)?;
        let text = format!(
            "req.headers.set({}, {})",
            rewrite(&Postman, key),
            rewrite(&Postman, value)
        );
        return Some((text, rest.len() - after.len() + used));
    }
    REWRITES.iter().find_map(|(from, to)| {
        let after = rest.strip_prefix(from)?;
        // `pm.response.code` must not match `pm.response.codes`.
        let ends_word = from.ends_with(['(', ')']) || !after.starts_with(is_identifier_char);
        ends_word.then(|| (to.to_string(), from.len()))
    })
}

/// `key` and `value` of a `{ key: ..., value: ... }` object, the form `pm.request.headers.add`
/// is called with.
fn header_object(argument: &str) -> Option<(&str, &str)> {
    let inner = argument.trim().strip_prefix('{')?.strip_suffix('}')?;
    let (first, second) = split_top_level_comma(inner)?;
    let value_of = |text: &str, name: &str| -> Option<(usize, usize)> {
        let colon = text.find(':')?;
        let label = text[..colon].trim().trim_matches(['"', '\'']);
        (label == name).then_some((colon + 1, text.len()))
    };
    let pick = |name: &str| {
        [first, second].into_iter().find_map(|part| {
            let (start, end) = value_of(part, name)?;
            Some(part[start..end].trim())
        })
    };
    Some((pick("key")?, pick("value")?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn converted(source: &str) -> String {
        let result = convert_script(source);
        assert_eq!(result.converted, result.total, "{}", result.text);
        result.text
    }

    #[test]
    fn the_documented_table_converts() {
        for (from, to) in [
            ("pm.test(\"a\", () => {});", "test(\"a\", () => {});"),
            ("pm.expect(a).to.equal(b);", "expect(a).toBe(b);"),
            ("pm.expect(a).to.eql(b);", "expect(a).toEqual(b);"),
            ("pm.expect(a).to.deep.equal(b);", "expect(a).toEqual(b);"),
            ("pm.expect(a).to.include(b);", "expect(a).toContain(b);"),
            (
                "pm.expect(a).to.have.property(\"p\");",
                "expect(a).toHaveProperty(\"p\");",
            ),
            (
                "pm.expect(a).to.be.above(1);",
                "expect(a).toBeGreaterThan(1);",
            ),
            ("pm.expect(a).to.be.below(1);", "expect(a).toBeLessThan(1);"),
            ("pm.expect(a).to.not.equal(b);", "expect(a).not.toBe(b);"),
            ("pm.expect(a).to.be.true;", "expect(a).toBe(true);"),
            ("pm.expect(a).to.exist;", "expect(a).not.toBe(undefined);"),
            (
                "pm.response.to.have.status(200);",
                "expect(res.status).toBe(200);",
            ),
            (
                "pm.response.to.be.ok;",
                "expect(res.status).toBeLessThan(300);",
            ),
            ("const c = pm.response.code;", "const c = res.status;"),
            ("const j = pm.response.json();", "const j = res.json();"),
            ("const t = pm.response.text();", "const t = res.body;"),
            ("pm.response.headers.get(\"x\");", "res.headers.get(\"x\");"),
            (
                "const t = pm.response.responseTime;",
                "const t = res.timeMs;",
            ),
            ("pm.environment.set(\"k\", 1);", "env.set(\"k\", 1);"),
            (
                "pm.collectionVariables.set(\"k\", 1);",
                "env.set(\"k\", 1);",
            ),
            ("pm.globals.set(\"k\", 1);", "env.set(\"k\", 1);"),
            ("pm.environment.get(\"k\");", "env.get(\"k\");"),
            ("pm.environment.unset(\"k\");", "env.unset(\"k\");"),
            ("pm.variables.set(\"k\", 1);", "vars.set(\"k\", 1);"),
            (
                "pm.variables.get(\"k\");",
                "(vars.get(\"k\") ?? env.get(\"k\"));",
            ),
            (
                "pm.request.headers.add({ key: \"a\", value: b });",
                "req.headers.set(\"a\", b);",
            ),
            ("const u = pm.request.url;", "const u = req.url;"),
            (
                "postman.setEnvironmentVariable(\"k\", 1);",
                "env.set(\"k\", 1);",
            ),
        ] {
            assert_eq!(converted(from), to);
        }
    }

    #[test]
    fn a_multiline_test_converts_inside_and_out() {
        let source = "pm.test('status', function () {\n  pm.response.to.have.status(200);\n  const body = pm.response.json();\n  pm.expect(body.id)\n    .to.equal(pm.environment.get('id'));\n});";
        assert_eq!(
            converted(source),
            "test('status', function () {\n  expect(res.status).toBe(200);\n  const body = res.json();\n  expect(body.id).toBe(env.get('id'));\n});"
        );
    }

    #[test]
    fn strings_and_comments_are_left_alone() {
        let source = "// pm.test is what this was\nconst a = \"pm.response.code\";\nconst b = `pm.globals.set(1)`;";
        assert_eq!(converted(source), source);
    }

    #[test]
    fn an_unknown_api_comments_out_its_statement_only() {
        let source = "pm.environment.set('a', 1);\npm.sendRequest('http://x', (err, res) => {\n  console.log(res);\n});\nconst c = pm.response.code;";
        let result = convert_script(source);
        assert_eq!(result.total, 3);
        assert_eq!(result.converted, 2);
        assert!(result.send_request);
        assert_eq!(
            result.text,
            format!(
                "{PARTIAL_HEADER}\nenv.set('a', 1);\n// pm.sendRequest('http://x', (err, res) => {{\n//   console.log(res);\n// }});\nconst c = res.status;"
            )
        );
    }

    #[test]
    fn an_unsupported_matcher_comments_out_the_test_that_uses_it() {
        let source = "pm.test('t', () => {\n  pm.expect(a).to.be.an('array');\n});";
        let result = convert_script(source);
        assert_eq!((result.total, result.converted), (1, 0));
        assert!(result.text.contains("// pm.test('t', () => {"));
    }

    #[test]
    fn other_names_are_not_confused_with_postman() {
        let source = "const pmo = 1;\nfoo.pm.bar();\nconst tests2 = 3;";
        assert_eq!(converted(source), source);
    }

    #[test]
    fn crypto_and_require_are_not_available() {
        for source in [
            "const h = CryptoJS.MD5('a');",
            "const m = require('moment');",
            "tests['ok'] = true;",
            "const b = responseBody;",
        ] {
            let result = convert_script(source);
            assert_eq!(result.converted, 0, "{source}");
            assert!(result.text.contains(&format!("// {source}")), "{source}");
        }
    }

    #[test]
    fn unbalanced_scripts_are_commented_out_whole() {
        let source = "pm.test('t', () => {\n  a(;\n";
        let result = convert_script(source);
        assert_eq!(result.converted, 0);
        assert!(result.text.lines().all(|line| line.starts_with("//")));
    }

    #[test]
    fn scripts_without_postman_stay_as_they_are() {
        let source = "const a = 1;\nif (a) {\n  console.log(a);\n} else {\n  console.log(0);\n}";
        let result = convert_script(source);
        assert_eq!(result.text, source);
        assert!(!result.rewrote);
    }
}
