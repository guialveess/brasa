//! Variable interpolation of `{{ }}` markers (see `docs/format.md`).

use crate::functions::{self, Arg, FunctionError};
use crate::key_value::KeyValue;

/// The three layers of variables looked up when resolving a `{{name}}` marker, first hit wins:
///
/// 1. `request_vars`, set by the pre script for this execution only;
/// 2. `session_env`, runtime overrides set by scripts, kept for the app session;
/// 3. `environment`, the active environment (`.local.env` merged over `.env`).
///
/// Entries with `enabled: false` are skipped, as if they were not present.
#[derive(Debug, Clone, Copy, Default)]
pub struct VarScope<'a> {
    /// Variables set by the pre script (`vars.set`), highest priority.
    pub request_vars: &'a [KeyValue],
    /// Session overrides set by scripts (`env.set`), second priority.
    pub session_env: &'a [KeyValue],
    /// The active environment, lowest priority.
    pub environment: &'a [KeyValue],
}

impl<'a> VarScope<'a> {
    /// Looks up `name` across the three layers, first hit wins. Returns `None` if no enabled
    /// entry with this key exists in any layer.
    pub fn lookup(&self, name: &str) -> Option<&'a str> {
        [self.request_vars, self.session_env, self.environment]
            .into_iter()
            .find_map(|layer| Self::lookup_in(layer, name))
    }

    /// Looks up `name` in a single layer, ignoring disabled entries.
    fn lookup_in(layer: &'a [KeyValue], name: &str) -> Option<&'a str> {
        layer
            .iter()
            .find(|entry| entry.enabled && entry.key == name)
            .map(|entry| entry.value.as_str())
    }
}

/// A problem found while interpolating a `{{ }}` marker.
///
/// None of these abort interpolation (see `docs/format.md`): the offending
/// marker is left untouched in the output and the warning is reported alongside the result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateWarning {
    /// `{{name}}` (or a bare identifier used as a function argument) did not match any variable
    /// in any layer of the [`VarScope`].
    #[error("unknown variable `{0}`")]
    UnknownVariable(String),
    /// `{{ name(args) }}` failed to evaluate: unknown function, wrong arity or a bad argument.
    #[error(transparent)]
    Function(#[from] FunctionError),
}

/// The result of interpolating a piece of text: the text with every resolvable `{{ }}` marker
/// replaced, and the warnings collected along the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interpolated {
    /// The interpolated text. Markers that could not be resolved are left exactly as written.
    pub text: String,
    /// One warning per marker that could not be resolved, in the order encountered.
    pub warnings: Vec<TemplateWarning>,
}

/// Interpolates every `{{ }}` marker in `text` using `scope`.
///
/// A marker is either a variable name (`{{name}}`, whitespace inside the braces is allowed) or
/// a function call (`{{ name(arg, ...) }}`, see [`crate::functions`]). Interpolation is a single
/// pass: the replacement text of a marker is copied into the output as is and never rescanned,
/// so a variable whose value contains `{{x}}` does not expand further. A marker that cannot be
/// resolved (unknown variable, unknown function, wrong arity, bad argument) is left untouched in
/// the output and reported as a warning. An unmatched `{{` with no following `}}` is also left
/// untouched, with no warning: it is not a marker.
pub fn interpolate(text: &str, scope: &VarScope) -> Interpolated {
    let mut output = String::with_capacity(text.len());
    let mut warnings = Vec::new();
    let mut rest = text;

    loop {
        let Some(start) = rest.find("{{") else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            // No closing marker anywhere in the remaining text: not a marker, keep it as is.
            output.push_str(&rest[start..]);
            break;
        };
        let raw = &after_open[..end];
        let full_marker = &rest[start..start + 2 + end + 2];
        match resolve_marker(raw, scope) {
            Ok(value) => output.push_str(&value),
            Err(warning) => {
                output.push_str(full_marker);
                warnings.push(warning);
            }
        }
        rest = &after_open[end + 2..];
    }

    Interpolated {
        text: output,
        warnings,
    }
}

/// Whether a [`VariableSpan`] is a plain variable reference or a template function call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableKind {
    /// `{{name}}`, a reference to a variable.
    Variable,
    /// `{{ name(args) }}`, a call to a built-in template function.
    FunctionCall,
}

/// A `{{ }}` marker found in a piece of text, without resolving it.
///
/// Used by the UI to style variable chips: an unstyled range of `text` around each marker,
/// underlined when the name is not a known variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableSpan {
    /// The byte range of the whole marker, including the `{{` and `}}` delimiters.
    pub range: std::ops::Range<usize>,
    /// The variable or function name, trimmed of surrounding whitespace.
    pub name: String,
    /// Whether `name` is a variable or a function call.
    pub kind: VariableKind,
}

/// Finds every `{{ }}` marker in `text`, without resolving it.
///
/// Reuses the same marker scan as [`interpolate`]: a marker is `{{` followed, later in the text,
/// by `}}`; an unmatched `{{` with no closing `}}` is not a marker and is not reported. A marker
/// is a [`VariableKind::FunctionCall`] when its content parses as `name(args)`
/// ([`parse_call`]), a [`VariableKind::Variable`] otherwise, whatever its content: this mirrors
/// [`resolve_marker`], which resolves a non-call marker as a variable name unconditionally.
#[must_use]
pub fn variable_spans(text: &str) -> Vec<VariableSpan> {
    let mut spans = Vec::new();
    let mut rest = text;
    let mut offset = 0;

    while let Some(start) = rest.find("{{") {
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            break;
        };
        let raw = &after_open[..end];
        let marker_start = offset + start;
        let marker_end = marker_start + 2 + end + 2;
        let trimmed = raw.trim();
        let (name, kind) = match parse_call(trimmed) {
            Some((name, _args)) => (name.to_string(), VariableKind::FunctionCall),
            None => (trimmed.to_string(), VariableKind::Variable),
        };
        spans.push(VariableSpan {
            range: marker_start..marker_end,
            name,
            kind,
        });
        offset = marker_end;
        rest = &after_open[end + 2..];
    }

    spans
}

/// Resolves the content of a single `{{ ... }}` marker (without the braces) to its replacement
/// text, or the warning explaining why it could not be resolved.
fn resolve_marker(raw: &str, scope: &VarScope) -> Result<String, TemplateWarning> {
    let trimmed = raw.trim();
    match parse_call(trimmed) {
        Some((name, args)) => {
            let args = resolve_args(args, scope)?;
            functions::call(name, &args).map_err(TemplateWarning::from)
        }
        None => scope
            .lookup(trimmed)
            .map(str::to_string)
            .ok_or_else(|| TemplateWarning::UnknownVariable(trimmed.to_string())),
    }
}

/// If `trimmed` has the shape `name(args)`, splits it into the function name and the raw,
/// unparsed argument list. Returns `None` for a plain variable name.
fn parse_call(trimmed: &str) -> Option<(&str, &str)> {
    let open = trimmed.find('(')?;
    if !trimmed.ends_with(')') {
        return None;
    }
    let name = trimmed[..open].trim();
    if name.is_empty() || !is_identifier(name) {
        return None;
    }
    let args = &trimmed[open + 1..trimmed.len() - 1];
    Some((name, args))
}

/// Whether `s` looks like a function or variable identifier: starts with a letter or
/// underscore, followed by letters, digits or underscores.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Parses and resolves a raw, comma-separated argument list into [`Arg`] values, resolving bare
/// identifiers as variables through `scope`.
fn resolve_args(raw: &str, scope: &VarScope) -> Result<Vec<Arg>, TemplateWarning> {
    split_args(raw)
        .into_iter()
        .map(|token| resolve_arg(token.trim(), scope))
        .collect()
}

/// Splits a raw argument list on top-level commas, respecting double-quoted string literals so
/// a comma inside a string does not split it. Returns an empty list for an empty (or
/// whitespace-only) input.
fn split_args(raw: &str) -> Vec<&str> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut start = 0;
    for (index, ch) in raw.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            ',' => {
                result.push(&raw[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    result.push(&raw[start..]);
    result
}

/// Resolves a single trimmed argument token to an [`Arg`]: a double-quoted string literal, an
/// integer literal, or a bare identifier looked up as a variable.
fn resolve_arg(token: &str, scope: &VarScope) -> Result<Arg, TemplateWarning> {
    if let Some(literal) = parse_string_literal(token) {
        return Ok(Arg::Str(literal));
    }
    if let Ok(value) = token.parse::<i64>() {
        return Ok(Arg::Int(value));
    }
    scope
        .lookup(token)
        .map(|value| Arg::Str(value.to_string()))
        .ok_or_else(|| TemplateWarning::UnknownVariable(token.to_string()))
}

/// Parses a double-quoted string literal with `\"` and `\\` escapes. Returns `None` if `token`
/// is not a well formed literal (missing quotes, or an escape other than `\"`/`\\`).
fn parse_string_literal(token: &str) -> Option<String> {
    let inner = token.strip_prefix('"')?.strip_suffix('"')?;
    let mut result = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                '"' => result.push('"'),
                '\\' => result.push('\\'),
                _ => return None,
            }
        } else {
            result.push(c);
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn scope<'a>(vars: &'a [KeyValue]) -> VarScope<'a> {
        VarScope {
            request_vars: &[],
            session_env: &[],
            environment: vars,
        }
    }

    #[test]
    fn replaces_a_simple_variable() {
        let vars = [KeyValue::new("name", "world")];
        let result = interpolate("hello {{name}}", &scope(&vars));
        assert_eq!(result.text, "hello world");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn allows_whitespace_inside_braces() {
        let vars = [KeyValue::new("name", "world")];
        let result = interpolate("hello {{ name }}", &scope(&vars));
        assert_eq!(result.text, "hello world");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn missing_variable_is_left_untouched_with_a_warning() {
        let result = interpolate("hello {{missing}}", &scope(&[]));
        assert_eq!(result.text, "hello {{missing}}");
        assert_eq!(
            result.warnings,
            vec![TemplateWarning::UnknownVariable("missing".to_string())]
        );
    }

    #[test]
    fn request_vars_take_priority_over_session_env_and_environment() {
        let request_vars = [KeyValue::new("id", "from-request")];
        let session_env = [KeyValue::new("id", "from-session")];
        let environment = [KeyValue::new("id", "from-environment")];
        let scope = VarScope {
            request_vars: &request_vars,
            session_env: &session_env,
            environment: &environment,
        };
        assert_eq!(interpolate("{{id}}", &scope).text, "from-request");
    }

    #[test]
    fn session_env_takes_priority_over_environment() {
        let session_env = [KeyValue::new("id", "from-session")];
        let environment = [KeyValue::new("id", "from-environment")];
        let scope = VarScope {
            request_vars: &[],
            session_env: &session_env,
            environment: &environment,
        };
        assert_eq!(interpolate("{{id}}", &scope).text, "from-session");
    }

    #[test]
    fn falls_through_to_the_next_layer_when_disabled() {
        let mut request_vars = [KeyValue::new("id", "from-request")];
        request_vars[0].enabled = false;
        let environment = [KeyValue::new("id", "from-environment")];
        let scope = VarScope {
            request_vars: &request_vars,
            session_env: &[],
            environment: &environment,
        };
        assert_eq!(interpolate("{{id}}", &scope).text, "from-environment");
    }

    #[test]
    fn does_not_expand_twice() {
        // The value of `outer` itself contains a marker, which must not be expanded further.
        let vars = [
            KeyValue::new("outer", "{{inner}}"),
            KeyValue::new("inner", "leaked"),
        ];
        let result = interpolate("{{outer}}", &scope(&vars));
        assert_eq!(result.text, "{{inner}}");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn handles_unicode_variable_values() {
        let vars = [KeyValue::new("greeting", "héllo wörld 世界")];
        let result = interpolate("say: {{greeting}}!", &scope(&vars));
        assert_eq!(result.text, "say: héllo wörld 世界!");
    }

    #[test]
    fn handles_adjacent_markers() {
        let vars = [KeyValue::new("a", "1"), KeyValue::new("b", "2")];
        let result = interpolate("{{a}}{{b}}", &scope(&vars));
        assert_eq!(result.text, "12");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn unmatched_opening_braces_are_left_untouched() {
        let result = interpolate("hello {{name", &scope(&[]));
        assert_eq!(result.text, "hello {{name");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn unmatched_closing_braces_are_left_untouched() {
        let result = interpolate("hello name}}", &scope(&[]));
        assert_eq!(result.text, "hello name}}");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn lone_brace_is_left_untouched() {
        let result = interpolate("cost: {5}", &scope(&[]));
        assert_eq!(result.text, "cost: {5}");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn calls_a_function_with_no_arguments() {
        let result = interpolate("{{ uuid() }}", &scope(&[]));
        assert!(result.warnings.is_empty());
        assert!(result.text.parse::<uuid::Uuid>().is_ok());
    }

    #[test]
    fn calls_a_function_with_string_and_int_literal_arguments() {
        let result = interpolate(r#"{{ base64Encode("user:pass") }}"#, &scope(&[]));
        assert_eq!(result.text, functions::base64_encode("user:pass"));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn calls_a_function_with_a_variable_argument() {
        let vars = [KeyValue::new("username", "tanis")];
        let result = interpolate("{{ base64Encode(username) }}", &scope(&vars));
        assert_eq!(result.text, functions::base64_encode("tanis"));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn each_call_occurrence_is_evaluated_independently() {
        // "|" (not "-") separates the two calls: a UUID itself contains dashes.
        let result = interpolate("{{ uuid() }}|{{ uuid() }}", &scope(&[]));
        let parts: Vec<&str> = result.text.split('|').collect();
        assert_eq!(parts.len(), 2);
        assert_ne!(parts[0], parts[1]);
    }

    #[test]
    fn unknown_function_is_left_untouched_with_a_warning() {
        let result = interpolate("{{ notAFunction() }}", &scope(&[]));
        assert_eq!(result.text, "{{ notAFunction() }}");
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn wrong_arity_is_left_untouched_with_a_warning() {
        let result = interpolate("{{ uuid(1) }}", &scope(&[]));
        assert_eq!(result.text, "{{ uuid(1) }}");
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn bad_argument_type_is_left_untouched_with_a_warning() {
        let result = interpolate(r#"{{ randomInt("a", 1) }}"#, &scope(&[]));
        assert_eq!(result.text, r#"{{ randomInt("a", 1) }}"#);
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn unknown_variable_argument_is_left_untouched_with_a_warning() {
        let result = interpolate("{{ base64Encode(missing) }}", &scope(&[]));
        assert_eq!(result.text, "{{ base64Encode(missing) }}");
        assert_eq!(
            result.warnings,
            vec![TemplateWarning::UnknownVariable("missing".to_string())]
        );
    }

    #[test]
    fn variable_spans_finds_a_plain_variable() {
        let spans = variable_spans("hello {{name}}");
        assert_eq!(
            spans,
            vec![VariableSpan {
                range: 6..14,
                name: "name".to_string(),
                kind: VariableKind::Variable,
            }]
        );
        assert_eq!(&"hello {{name}}"[6..14], "{{name}}");
    }

    #[test]
    fn variable_spans_trims_whitespace_inside_braces() {
        let spans = variable_spans("{{ name }}");
        assert_eq!(spans[0].name, "name");
        assert_eq!(spans[0].kind, VariableKind::Variable);
    }

    #[test]
    fn variable_spans_classifies_a_function_call() {
        let spans = variable_spans(r#"{{ base64Encode("a") }}"#);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "base64Encode");
        assert_eq!(spans[0].kind, VariableKind::FunctionCall);
    }

    #[test]
    fn variable_spans_finds_several_markers_in_order() {
        let spans = variable_spans("{{a}} and {{b}}");
        let names: Vec<&str> = spans.iter().map(|span| span.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn variable_spans_ignores_an_unmatched_opening_marker() {
        let spans = variable_spans("hello {{name");
        assert!(spans.is_empty());
    }

    #[test]
    fn variable_spans_has_correct_byte_ranges_around_multi_byte_utf8_text() {
        let text = "héllo wörld {{name}} 世界";
        let spans = variable_spans(text);
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(&text[span.range.clone()], "{{name}}");
        assert_eq!(span.name, "name");
    }
}
