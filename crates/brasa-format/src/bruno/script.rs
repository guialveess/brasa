//! Converting Bruno scripts, tests, assertions and request variables to Brasa's script API
//! (`docs/scripting.md`), on the engine of [`crate::script_convert`].
//!
//! Bruno's `req` and `res` share their names with Brasa's, but not all their API: `res.body` is
//! the parsed JSON in Bruno and the text in Brasa, so it becomes `res.json()`. Anything with no
//! equivalent (`bru.sendRequest`, `bru.setNextRequest`, cookies, `res.headers` read as an object,
//! a Chai assertion Brasa has no matcher for) makes its statement a comment, never a script
//! that silently does something else.

use brasa_core::KeyValue;

use crate::script_convert::{
    self, ConvertedScript, Dialect, balanced, is_identifier_char, rewrite, rewrite_expect,
};

/// The first line of a script with commented out statements.
const PARTIAL_HEADER: &str =
    "// Imported from Bruno. What could not be converted is commented out below.";

/// The first line of a script imported without converting it (the option is off).
pub(super) const NOT_CONVERTED_HEADER: &str =
    "// Imported from Bruno. Scripts were not converted, adapt the bru.*, req.* and res.* calls.";

/// What a statement is rewritten to when it uses something without an equivalent. It is in
/// [`UNSUPPORTED`], so the statement ends up commented out and this never reaches a file.
const NO_EQUIVALENT: &str = "__bruno_no_equivalent__";

/// The text of a rewrite to [`NO_EQUIVALENT`], with a space so it never joins the identifier
/// that follows it.
fn no_equivalent() -> String {
    format!("{NO_EQUIVALENT} ")
}

/// What is left of Bruno's API after the rewrites, each making its statement a comment.
const UNSUPPORTED: &[&str] = &[NO_EQUIVALENT, "bru", "require", "process", "atob", "btoa"];

/// Rewrites without an argument: the text to its replacement. Longest first where one is the
/// prefix of another.
const REWRITES: &[(&str, &str)] = &[
    ("bru.getEnvVar(", "env.get("),
    ("bru.setEnvVar(", "env.set("),
    ("bru.deleteEnvVar(", "env.unset("),
    ("bru.getVar(", "env.get("),
    ("bru.setVar(", "env.set("),
    ("bru.deleteVar(", "env.unset("),
    ("bru.getCollectionVar(", "env.get("),
    ("bru.setCollectionVar(", "env.set("),
    ("bru.deleteCollectionVar(", "env.unset("),
    ("bru.getFolderVar(", "env.get("),
    ("bru.getGlobalEnvVar(", "env.get("),
    ("bru.setGlobalEnvVar(", "env.set("),
    ("bru.deleteGlobalEnvVar(", "env.unset("),
    ("bru.getRequestVar(", "vars.get("),
    ("req.getUrl()", "req.url"),
    ("req.getMethod()", "req.method"),
    ("req.getHeader(", "req.headers.get("),
    ("req.setHeader(", "req.headers.set("),
    ("req.deleteHeader(", "req.headers.remove("),
    ("res.getStatus()", "res.status"),
    ("res.getHeader(", "res.headers.get("),
    ("res.getBody()", "res.json()"),
    ("res.getResponseTime()", "res.timeMs"),
    ("res.responseTime", "res.timeMs"),
    ("res.getSize().body", "res.size"),
    ("res.body", "res.json()"),
];

/// The properties of Brasa's `req` a statement may still use after the rewrites.
const REQ_PROPERTIES: &[&str] = &["method", "url", "body", "headers"];

/// The properties of Brasa's `res` a statement may still use after the rewrites.
const RES_PROPERTIES: &[&str] = &["status", "json", "timeMs", "size"];

/// The words of a Chai chain before its assertion.
const CHAI_WORDS: &[&str] = &[
    "to", "be", "been", "is", "that", "which", "and", "has", "have", "with", "at", "of", "same",
    "deep",
];

/// Bruno's scripts.
struct Bruno;

impl Dialect for Bruno {
    fn partial_header(&self) -> &'static str {
        PARTIAL_HEADER
    }

    fn unsupported(&self) -> &'static [&'static str] {
        UNSUPPORTED
    }

    fn sends_requests(&self, code: &str) -> bool {
        code.contains("bru.sendRequest") || code.contains("bru.runRequest")
    }

    fn rewrite_at(&self, rest: &str) -> Option<(String, usize)> {
        bruno_rewrite_at(rest)
    }
}

/// The identifier `text` starts with.
fn word(text: &str) -> &str {
    let length = text
        .find(|character: char| !is_identifier_char(character))
        .unwrap_or(text.len());
    &text[..length]
}

/// Whether the chain after `expect(...)` is Chai's (`.to.equal(`) rather than Brasa's own
/// (`.toBe(`, `.not.toBe(`).
fn is_chai_chain(after: &str) -> bool {
    let Some(first) = after.trim_start().strip_prefix('.') else {
        return false;
    };
    let first_word = word(first);
    if first_word == "not" {
        let second = first[3..].trim_start().strip_prefix('.').map(word);
        return second.is_some_and(|second| CHAI_WORDS.contains(&second));
    }
    CHAI_WORDS.contains(&first_word)
}

/// `receiver.property = (argument)` for a setter call `rest` starts with, `prefix` included.
fn setter(rest: &str, prefix: &str, property: &str) -> Option<(String, usize)> {
    let after = rest.strip_prefix(prefix)?;
    let (argument, used) = balanced(after)?;
    Some((
        format!("{property} = {}", rewrite(&Bruno, argument)),
        prefix.len() + used,
    ))
}

fn bruno_rewrite_at(rest: &str) -> Option<(String, usize)> {
    if let Some(after) = rest.strip_prefix("expect(") {
        let (_, used) = balanced(after)?;
        if !is_chai_chain(&after[used..]) {
            return None;
        }
        return rewrite_expect(&Bruno, rest, "expect(")
            .or_else(|| Some((no_equivalent(), "expect(".len())));
    }
    if rest.starts_with("req.setUrl(") {
        return setter(rest, "req.setUrl(", "req.url");
    }
    if rest.starts_with("req.setMethod(") {
        return setter(rest, "req.setMethod(", "req.method");
    }
    if let Some(after) = rest.strip_prefix("req.setBody(") {
        let (argument, used) = balanced(after)?;
        let text = format!(
            "req.body = ((body) => (typeof body === \"string\" ? body : JSON.stringify(body)))({})",
            rewrite(&Bruno, argument)
        );
        return Some((text, "req.setBody(".len() + used));
    }
    if let Some(after) = rest.strip_prefix("req.getBody(") {
        let (argument, used) = balanced(after)?;
        let text = if argument.contains("raw") {
            "req.body"
        } else {
            "JSON.parse(req.body)"
        };
        return Some((text.to_string(), "req.getBody(".len() + used));
    }
    if let Some(found) = REWRITES.iter().find_map(|(from, to)| {
        let after = rest.strip_prefix(from)?;
        let ends_word = from.ends_with(['(', ')']) || !after.starts_with(is_identifier_char);
        ends_word.then(|| (to.to_string(), from.len()))
    }) {
        return Some(found);
    }
    // `res.headers` is an object in Bruno and has only `get` in Brasa.
    if let Some(after) = rest.strip_prefix("res.headers") {
        let ok = after.starts_with(".get(") || !after.starts_with(['.', '[']);
        return (!ok).then(|| (no_equivalent(), "res.headers".len()));
    }
    for (object, allowed) in [("req.", REQ_PROPERTIES), ("res.", RES_PROPERTIES)] {
        if let Some(after) = rest.strip_prefix(object) {
            let name = word(after);
            if !allowed.contains(&name) {
                return Some((no_equivalent(), object.len() + name.len()));
            }
        }
    }
    None
}

/// Converts a Bruno script.
pub(super) fn convert_script(source: &str) -> ConvertedScript {
    script_convert::convert_script(&Bruno, source)
}

/// A JavaScript string literal of `text`.
fn js_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

/// The JavaScript for an assertion or post-response variable expression, `$res.body.id` or
/// `res.status`. `None` for what has no equivalent.
fn response_expression(expression: &str) -> Option<String> {
    let expression = expression.trim();
    let expression = expression.strip_prefix('$').unwrap_or(expression);
    let rest = expression.strip_prefix("res.")?;
    let name = word(rest);
    let after = &rest[name.len()..];
    match name {
        "status" if after.is_empty() => Some("res.status".to_string()),
        "responseTime" if after.is_empty() => Some("res.timeMs".to_string()),
        "body" => Some(format!("res.json(){after}")),
        "headers" => {
            let header = after
                .strip_prefix('.')
                .map(str::to_string)
                .or_else(|| {
                    let inner = after.strip_prefix('[')?.strip_suffix(']')?;
                    Some(inner.trim().trim_matches(['"', '\'']).to_string())
                })
                .filter(|header| !header.is_empty())?;
            Some(format!("res.headers.get({})", js_string(&header)))
        }
        _ => None,
    }
}

/// The JavaScript for the value of an assertion: numbers, booleans, `null`, quoted strings and
/// `{{variables}}` as such, anything else as a string.
fn literal(value: &str) -> String {
    let value = value.trim();
    if let Some(name) = value
        .strip_prefix("{{")
        .and_then(|rest| rest.strip_suffix("}}"))
    {
        let name = js_string(name.trim());
        return format!("(vars.get({name}) ?? env.get({name}))");
    }
    let quoted = (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''));
    if quoted && value.len() >= 2 {
        return js_string(&value[1..value.len() - 1]);
    }
    if matches!(value, "true" | "false" | "null" | "undefined") || value.parse::<f64>().is_ok() {
        return value.to_string();
    }
    js_string(value)
}

/// The `expect` statement of one assertion, `None` for an operator Brasa has no equivalent of.
fn assertion(target: &str, operator: &str, value: &str) -> Option<String> {
    let v = || literal(value);
    let range = || {
        let (low, high) = value.split_once(',')?;
        Some((literal(low), literal(high)))
    };
    let list = || {
        let items: Vec<String> = value
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(literal)
            .collect();
        format!("[{}]", items.join(", "))
    };
    Some(match operator {
        "eq" => format!("expect({target}).toBe({})", v()),
        "neq" => format!("expect({target}).not.toBe({})", v()),
        "gt" => format!("expect({target}).toBeGreaterThan({})", v()),
        "lt" => format!("expect({target}).toBeLessThan({})", v()),
        "gte" => format!("expect({target} >= {}).toBe(true)", v()),
        "lte" => format!("expect({target} <= {}).toBe(true)", v()),
        "in" => format!("expect({}.includes({target})).toBe(true)", list()),
        "notIn" => format!("expect({}.includes({target})).toBe(false)", list()),
        "contains" => format!("expect({target}).toContain({})", v()),
        "notContains" => format!("expect({target}).not.toContain({})", v()),
        "length" => format!("expect({target}.length).toBe({})", v()),
        "matches" => format!(
            "expect({target}).toMatch(new RegExp({}))",
            js_string(value.trim())
        ),
        "notMatches" => format!(
            "expect({target}).not.toMatch(new RegExp({}))",
            js_string(value.trim())
        ),
        "startsWith" => format!("expect(String({target}).startsWith({})).toBe(true)", v()),
        "endsWith" => format!("expect(String({target}).endsWith({})).toBe(true)", v()),
        "between" => {
            let (low, high) = range()?;
            format!("expect({target} >= {low} && {target} <= {high}).toBe(true)")
        }
        "isEmpty" => format!("expect(Object.keys({target} ?? {{}}).length).toBe(0)"),
        "isNotEmpty" => format!("expect(Object.keys({target} ?? {{}}).length).toBeGreaterThan(0)"),
        "isNull" => format!("expect({target}).toBe(null)"),
        "isUndefined" => format!("expect({target}).toBe(undefined)"),
        "isDefined" => format!("expect({target}).not.toBe(undefined)"),
        "isTruthy" => format!("expect({target}).toBeTruthy()"),
        "isFalsy" => format!("expect({target}).toBeFalsy()"),
        "isNumber" => format!("expect(typeof {target}).toBe(\"number\")"),
        "isString" => format!("expect(typeof {target}).toBe(\"string\")"),
        "isBoolean" => format!("expect(typeof {target}).toBe(\"boolean\")"),
        "isJson" => format!("expect(typeof {target}).toBe(\"object\")"),
        "isArray" => format!("expect(Array.isArray({target})).toBe(true)"),
        _ => return None,
    })
}

/// The operators of Bruno's assertions.
const OPERATORS: &[&str] = &[
    "eq",
    "neq",
    "gt",
    "gte",
    "lt",
    "lte",
    "in",
    "notIn",
    "contains",
    "notContains",
    "length",
    "matches",
    "notMatches",
    "startsWith",
    "endsWith",
    "between",
    "isEmpty",
    "isNotEmpty",
    "isNull",
    "isUndefined",
    "isDefined",
    "isTruthy",
    "isFalsy",
    "isNumber",
    "isString",
    "isBoolean",
    "isJson",
    "isArray",
];

/// Lines of script, and how many of the source's items each kind kept as code.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Generated {
    /// The lines.
    pub(super) text: String,
    /// The items.
    pub(super) total: usize,
    /// The items written as code; the others are comments.
    pub(super) converted: usize,
}

impl Generated {
    fn code(&mut self, line: &str) {
        self.total += 1;
        self.converted += 1;
        self.push(line);
    }

    fn comment(&mut self, line: &str) {
        self.total += 1;
        self.push(&format!("// {line}"));
    }

    fn push(&mut self, line: &str) {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(line);
    }
}

/// `test()` calls for the enabled `assert` entries.
pub(super) fn assertions_to_tests(asserts: &[KeyValue]) -> Generated {
    let mut out = Generated::default();
    for entry in asserts.iter().filter(|entry| entry.enabled) {
        let source = format!("{}: {}", entry.key, entry.value);
        let value = entry.value.trim();
        let (operator, operand) = match value.split_once(char::is_whitespace) {
            Some((first, rest)) if OPERATORS.contains(&first) => (first, rest.trim()),
            _ if OPERATORS.contains(&value) => (value, ""),
            _ => ("eq", value),
        };
        let statement = response_expression(&entry.key)
            .and_then(|target| assertion(&target, operator, operand));
        match statement {
            Some(statement) => out.code(&format!(
                "test({}, () => {{\n  {statement};\n}});",
                js_string(&source)
            )),
            None => out.comment(&format!("assert {source}")),
        }
    }
    out
}

/// `vars.set` calls for the enabled `vars:pre-request` entries.
pub(super) fn pre_request_vars(vars: &[KeyValue]) -> Generated {
    let mut out = Generated::default();
    for entry in vars.iter().filter(|entry| entry.enabled) {
        out.code(&format!(
            "vars.set({}, {});",
            js_string(&entry.key),
            js_string(&entry.value)
        ));
    }
    out
}

/// `env.set` (or, for a `@local` one, `vars.set`) calls for the enabled `vars:post-response`
/// entries, whose values are response expressions.
pub(super) fn post_response_vars(vars: &[KeyValue]) -> Generated {
    let mut out = Generated::default();
    for entry in vars.iter().filter(|entry| entry.enabled) {
        let (setter, name) = match entry.key.strip_prefix('@') {
            Some(name) => ("vars.set", name),
            None => ("env.set", entry.key.as_str()),
        };
        match response_expression(&entry.value) {
            Some(expression) => {
                out.code(&format!("{setter}({}, {expression});", js_string(name)));
            }
            None => out.comment(&format!("{}: {}", entry.key, entry.value)),
        }
    }
    out
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
    fn variables_and_request_calls() {
        for (from, to) in [
            (
                "bru.setEnvVar(\"t\", res.body.token);",
                "env.set(\"t\", res.json().token);",
            ),
            ("const a = bru.getVar('a');", "const a = env.get('a');"),
            ("bru.setVar('a', 1);", "env.set('a', 1);"),
            ("bru.getRequestVar('id');", "vars.get('id');"),
            (
                "req.setHeader('X-A', bru.getEnvVar('a'));",
                "req.headers.set('X-A', env.get('a'));",
            ),
            (
                "req.setUrl(req.getUrl() + '?x=1');",
                "req.url = req.url + '?x=1';",
            ),
            ("req.setMethod('POST');", "req.method = 'POST';"),
            (
                "const b = req.getBody();",
                "const b = JSON.parse(req.body);",
            ),
            (
                "const raw = req.getBody({ raw: true });",
                "const raw = req.body;",
            ),
            (
                "req.setBody({ a: 1 });",
                "req.body = ((body) => (typeof body === \"string\" ? body : JSON.stringify(body)))({ a: 1 });",
            ),
            ("const s = res.getStatus();", "const s = res.status;"),
            ("res.getHeader('etag');", "res.headers.get('etag');"),
            ("const t = res.getResponseTime();", "const t = res.timeMs;"),
            (
                "const n = res.body.items.length;",
                "const n = res.json().items.length;",
            ),
        ] {
            assert_eq!(converted(from), to, "{from}");
        }
    }

    #[test]
    fn chai_assertions_become_brasa_matchers() {
        assert_eq!(
            converted(
                "test('ok', function () {\n  expect(res.status).to.equal(200);\n  expect(res.body.id).to.not.be.undefined;\n});"
            ),
            "test('ok', function () {\n  expect(res.status).toBe(200);\n  expect(res.json().id).not.toBe(undefined);\n});"
        );
        // Brasa's own matchers stay as they are.
        assert_eq!(
            converted("expect(res.status).toBe(200);"),
            "expect(res.status).toBe(200);"
        );
        assert_eq!(
            converted("expect(res.status).not.toBe(500);"),
            "expect(res.status).not.toBe(500);"
        );
    }

    #[test]
    fn what_has_no_equivalent_is_commented_out() {
        let result = convert_script(
            "bru.setVar('a', 1);\nbru.sendRequest({ url: 'x' });\nconst ct = res.headers['content-type'];\nexpect(res.body.items).to.have.lengthOf(2);\nbru.setNextRequest('Login');\nconst name = req.getName();",
        );
        assert_eq!(result.total, 6, "{}", result.text);
        assert_eq!(result.converted, 1, "{}", result.text);
        assert!(result.send_request);
        assert!(result.text.starts_with(PARTIAL_HEADER), "{}", result.text);
        assert!(result.text.contains("// bru.sendRequest"));
        assert!(
            result
                .text
                .contains("// const ct = res.headers['content-type'];")
        );
        assert!(
            result
                .text
                .contains("// expect(res.body.items).to.have.lengthOf(2);")
        );
        assert!(!result.text.contains(NO_EQUIVALENT));
    }

    #[test]
    fn assertions_become_tests() {
        let asserts = vec![
            KeyValue::new("$res.status", "eq 200"),
            KeyValue::new("res.body.name", "eq \"Ann\""),
            KeyValue::new("$res.body.items", "isArray"),
            KeyValue::new("$res.headers.content-type", "contains json"),
            KeyValue::new("$res.body.count", "between 1, 5"),
            KeyValue::new("$res.body.id", "{{userId}}"),
            KeyValue::new("$res('data.items')", "isNotEmpty"),
            KeyValue {
                key: "$res.body.off".to_string(),
                value: "eq 1".to_string(),
                enabled: false,
            },
        ];
        let generated = assertions_to_tests(&asserts);
        assert_eq!((generated.total, generated.converted), (7, 6));
        assert_eq!(
            generated.text,
            [
                "test(\"$res.status: eq 200\", () => {\n  expect(res.status).toBe(200);\n});",
                "test(\"res.body.name: eq \\\"Ann\\\"\", () => {\n  expect(res.json().name).toBe(\"Ann\");\n});",
                "test(\"$res.body.items: isArray\", () => {\n  expect(Array.isArray(res.json().items)).toBe(true);\n});",
                "test(\"$res.headers.content-type: contains json\", () => {\n  expect(res.headers.get(\"content-type\")).toContain(\"json\");\n});",
                "test(\"$res.body.count: between 1, 5\", () => {\n  expect(res.json().count >= 1 && res.json().count <= 5).toBe(true);\n});",
                "test(\"$res.body.id: {{userId}}\", () => {\n  expect(res.json().id).toBe((vars.get(\"userId\") ?? env.get(\"userId\")));\n});",
                "// assert $res('data.items'): isNotEmpty",
            ]
            .join("\n")
        );
    }

    #[test]
    fn request_variables() {
        let pre = pre_request_vars(&[
            KeyValue::new("date", "2020-01-01"),
            KeyValue {
                key: "off".to_string(),
                value: "x".to_string(),
                enabled: false,
            },
        ]);
        assert_eq!(pre.text, "vars.set(\"date\", \"2020-01-01\");");
        let post = post_response_vars(&[
            KeyValue::new("token", "$res.body.token"),
            KeyValue::new("@order", "res.body.order"),
            KeyValue::new("odd", "Math.random()"),
        ]);
        assert_eq!(
            post.text,
            "env.set(\"token\", res.json().token);\nvars.set(\"order\", res.json().order);\n// odd: Math.random()"
        );
        assert_eq!((post.total, post.converted), (3, 2));
    }
}
