//! [`preview`], a variables-only resolution of a request: no pre/post script runs and nothing is
//! sent over the network. Used by the UI to show a live preview of the request (underlining unknown
//! variables as the user types) and by the code snippet generator, which needs a
//! [`brasa_core::ResolvedRequest`] without running arbitrary scripts or touching the network.

use std::collections::HashSet;

use brasa_core::{
    Auth, Body, Environment, Request, ResolvedRequest, TemplateWarning, VarScope, VariableKind,
    variable_spans,
};

use crate::resolve::{MultipartFiles, resolve};
use crate::session_env::SessionEnv;

/// Where an [`UnknownVariable`] was found in the request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VariableLocation {
    /// The request URL, before the query string is appended.
    Url,
    /// A `::: query` entry, key or value.
    Query,
    /// A header, key or value. Carries the header's key as written in the request, so the UI
    /// can point at the right row even if the key itself is a marker.
    Header(String),
    /// A field of the `::: auth` section.
    Auth,
    /// The request body.
    Body,
}

/// A `{{name}}` marker that did not match any variable in the scope [`preview`] resolved
/// against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownVariable {
    /// The variable name, as written inside the braces.
    pub name: String,
    /// Where it was found.
    pub location: VariableLocation,
}

/// The result of [`preview`]ing a request.
#[derive(Debug, Clone)]
pub struct Preview {
    /// The request with every resolvable `{{ }}` marker replaced, exactly as
    /// [`crate::Runner::run`] would build it for sending.
    pub resolved: ResolvedRequest,
    /// Every [`TemplateWarning`] collected while interpolating, in the same shape
    /// [`crate::RunResult::warnings`] reports.
    pub warnings: Vec<TemplateWarning>,
    /// Every unresolved variable reference, deduplicated by (name, location), in order of
    /// appearance. Template function calls (`{{ uuid() }}`) are never reported here.
    pub unknown_variables: Vec<UnknownVariable>,
}

/// Resolves `request` against `environment` and `session_env`, without running its `pre`/`post`
/// scripts and without sending it over the network.
///
/// `request_vars` (the pre script's variable scope layer) is empty: a preview never runs a
/// script, so nothing can populate it. This makes `preview` safe to call on every keystroke.
#[must_use]
pub fn preview(request: &Request, environment: &Environment, session_env: &SessionEnv) -> Preview {
    let scope = VarScope {
        request_vars: &[],
        session_env: session_env.as_slice(),
        environment: &environment.variables,
    };
    let (resolved, warnings, _) = resolve(request, &scope, MultipartFiles::Names);
    let unknown_variables = find_unknown_variables(request, &scope);
    Preview {
        resolved,
        warnings,
        unknown_variables,
    }
}

/// Scans every field of `request` for `{{name}}` markers that do not resolve in `scope`,
/// deduplicated by (name, location), in order of appearance. Function call markers are skipped:
/// per the plan, they are never reported as unknown variables.
fn find_unknown_variables(request: &Request, scope: &VarScope) -> Vec<UnknownVariable> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();

    let mut scan = |text: &str, location: VariableLocation| {
        for span in variable_spans(text) {
            if span.kind != VariableKind::Variable || scope.lookup(&span.name).is_some() {
                continue;
            }
            if seen.insert((span.name.clone(), location.clone())) {
                found.push(UnknownVariable {
                    name: span.name,
                    location: location.clone(),
                });
            }
        }
    };

    scan(&request.url, VariableLocation::Url);
    for entry in request.query.iter().filter(|entry| entry.enabled) {
        scan(&entry.key, VariableLocation::Query);
        scan(&entry.value, VariableLocation::Query);
    }
    for header in request.headers.iter().filter(|header| header.enabled) {
        scan(&header.key, VariableLocation::Header(header.key.clone()));
        scan(&header.value, VariableLocation::Header(header.key.clone()));
    }
    match &request.auth {
        Auth::None => {}
        Auth::Basic { username, password } => {
            scan(username, VariableLocation::Auth);
            scan(password, VariableLocation::Auth);
        }
        Auth::Bearer { token } => scan(token, VariableLocation::Auth),
        Auth::ApiKey { name, value, .. } => {
            scan(name, VariableLocation::Auth);
            scan(value, VariableLocation::Auth);
        }
    }
    match &request.body {
        Body::Json(text) | Body::Text(text) | Body::Xml(text) => {
            scan(text, VariableLocation::Body);
        }
        Body::Form(fields) | Body::Multipart(fields) => {
            for field in fields.iter().filter(|field| field.enabled) {
                scan(&field.key, VariableLocation::Body);
                scan(&field.value, VariableLocation::Body);
            }
        }
        Body::None => {}
    }

    found
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use brasa_core::{KeyValue, Method};
    use pretty_assertions::assert_eq;

    fn base_request(url: &str) -> Request {
        Request {
            method: Method::Get,
            url: url.to_string(),
            ..Request::default()
        }
    }

    fn environment(vars: &[(&str, &str)]) -> Environment {
        Environment {
            name: "test".to_string(),
            variables: vars
                .iter()
                .map(|(key, value)| KeyValue::new(*key, *value))
                .collect(),
        }
    }

    #[test]
    fn a_defined_variable_resolves() {
        let request = base_request("http://example.com/{{id}}");
        let environment = environment(&[("id", "42")]);
        let preview = preview(&request, &environment, &SessionEnv::new());
        assert_eq!(preview.resolved.url, "http://example.com/42");
        assert!(preview.warnings.is_empty());
        assert!(preview.unknown_variables.is_empty());
    }

    #[test]
    fn an_undefined_variable_in_the_url_is_reported() {
        let request = base_request("http://example.com/{{missing}}");
        let preview = preview(&request, &Environment::default(), &SessionEnv::new());
        assert_eq!(
            preview.unknown_variables,
            vec![UnknownVariable {
                name: "missing".to_string(),
                location: VariableLocation::Url,
            }]
        );
    }

    #[test]
    fn an_undefined_variable_in_the_query_is_reported() {
        let mut request = base_request("http://example.com");
        request.query.push(KeyValue::new("page", "{{missing}}"));
        let preview = preview(&request, &Environment::default(), &SessionEnv::new());
        assert_eq!(
            preview.unknown_variables,
            vec![UnknownVariable {
                name: "missing".to_string(),
                location: VariableLocation::Query,
            }]
        );
    }

    #[test]
    fn an_undefined_variable_in_a_header_is_reported_with_its_key() {
        let mut request = base_request("http://example.com");
        request
            .headers
            .push(KeyValue::new("X-Token", "{{missing}}"));
        let preview = preview(&request, &Environment::default(), &SessionEnv::new());
        assert_eq!(
            preview.unknown_variables,
            vec![UnknownVariable {
                name: "missing".to_string(),
                location: VariableLocation::Header("X-Token".to_string()),
            }]
        );
    }

    #[test]
    fn an_undefined_variable_in_the_body_is_reported() {
        let mut request = base_request("http://example.com");
        request.body = Body::Json(r#"{"id": "{{missing}}"}"#.to_string());
        let preview = preview(&request, &Environment::default(), &SessionEnv::new());
        assert_eq!(
            preview.unknown_variables,
            vec![UnknownVariable {
                name: "missing".to_string(),
                location: VariableLocation::Body,
            }]
        );
    }

    #[test]
    fn an_undefined_variable_in_the_auth_is_reported() {
        let mut request = base_request("https://example.com");
        request.auth = Auth::Bearer {
            token: "{{token}}".to_string(),
        };
        let preview = preview(&request, &Environment::default(), &SessionEnv::new());
        assert_eq!(
            preview.unknown_variables,
            vec![UnknownVariable {
                name: "token".to_string(),
                location: VariableLocation::Auth,
            }]
        );
    }

    #[test]
    fn function_calls_are_never_reported_as_unknown_variables() {
        let request = base_request("http://example.com/{{ uuid() }}");
        let preview = preview(&request, &Environment::default(), &SessionEnv::new());
        assert!(preview.unknown_variables.is_empty());
    }

    #[test]
    fn session_overrides_win_over_the_environment() {
        let request = base_request("http://example.com/{{token}}");
        let environment = environment(&[("token", "from-environment")]);
        let mut session_env = SessionEnv::new();
        session_env.set("token", "from-session");
        let preview = preview(&request, &environment, &session_env);
        assert_eq!(preview.resolved.url, "http://example.com/from-session");
        assert!(preview.unknown_variables.is_empty());
    }

    #[test]
    fn unknown_variables_are_deduplicated_by_name_and_location() {
        let request = base_request("http://example.com/{{missing}}/{{missing}}");
        let preview = preview(&request, &Environment::default(), &SessionEnv::new());
        assert_eq!(preview.unknown_variables.len(), 1);
    }

    #[test]
    fn preview_never_runs_a_script_and_never_opens_a_socket() {
        let mut request = base_request("http://example.com");
        request.pre_script = "throw new Error('boom')".to_string();
        request.post_script = "throw new Error('boom')".to_string();
        // Neither script runs: preview succeeds and does not touch the network.
        let preview = preview(&request, &Environment::default(), &SessionEnv::new());
        assert_eq!(preview.resolved.url, "http://example.com");
        assert!(preview.warnings.is_empty());
    }
}
