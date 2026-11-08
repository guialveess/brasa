//! A syntactic heuristic for names a pre script defines with `vars.set(...)`, used so the live
//! variable preview (`brasa_runner::preview`, which never runs a script) does not flag them as
//! unknown before the request has actually been sent.
//!
//! This is deliberately not a JS parser: it only looks for `vars.set(` followed by a single- or
//! double-quoted string literal as the first argument, with no escape handling. That is enough
//! for every request in the sample workspace and for the common case in general. A pre script
//! that computes the name dynamically (`vars.set(name, ...)`) or writes it with an escaped quote
//! is not recognized; the variable then still shows as unknown until the request is actually
//! sent, which is the safe direction to be wrong in: a chip is never marked defined when it
//! might not be.

use std::collections::HashSet;

/// The exact call prefix this heuristic looks for.
const NEEDLE: &str = "vars.set(";

/// Every variable name that `pre_script` sets with `vars.set("name", ...)` or
/// `vars.set('name', ...)`, found by a simple substring scan.
pub fn vars_set_names(pre_script: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut rest = pre_script;
    while let Some(index) = rest.find(NEEDLE) {
        let after = &rest[index + NEEDLE.len()..];
        let trimmed = after.trim_start();
        if let Some(quote) = trimmed.chars().next()
            && (quote == '"' || quote == '\'')
            && let Some(end) = trimmed[1..].find(quote)
        {
            names.insert(trimmed[1..1 + end].to_string());
        }
        rest = after;
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_double_quoted_name() {
        let names = vars_set_names(r#"vars.set("requestId", util.uuid());"#);
        assert!(names.contains("requestId"));
    }

    #[test]
    fn finds_a_single_quoted_name() {
        let names = vars_set_names("vars.set('token', 'abc');");
        assert!(names.contains("token"));
    }

    #[test]
    fn finds_several_calls() {
        let names = vars_set_names("vars.set(\"a\", 1);\nvars.set(\"b\", 2);");
        assert_eq!(names.len(), 2);
        assert!(names.contains("a"));
        assert!(names.contains("b"));
    }

    #[test]
    fn ignores_a_script_with_no_vars_set_call() {
        assert!(vars_set_names("env.set(\"x\", 1);").is_empty());
    }

    #[test]
    fn ignores_a_dynamically_computed_name() {
        assert!(vars_set_names("vars.set(name, 1);").is_empty());
    }

    #[test]
    fn is_empty_for_an_empty_script() {
        assert!(vars_set_names("").is_empty());
    }
}
