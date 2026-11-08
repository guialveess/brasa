//! From the shapes of [`super::model`] to the [`ImportPlan`] a Postman import produces, so the
//! wizard's plan, preview and report are shared. The mapping is described in
//! `docs/bruno-import.md`.

use brasa_core::{ApiKeyLocation, Auth, Body, KeyValue, Method, Request};

use super::model::{Defaults, RawAuth, RawBody, RawFolder, RawRequest};
use super::script::{
    NOT_CONVERTED_HEADER, assertions_to_tests, convert_script, post_response_vars, pre_request_vars,
};
use crate::postman::{ImportFolder, PostmanOptions, RequestIssue};

/// The variable that holds the access token of a request with OAuth 2.0 authorization, the same
/// as a Postman import's.
pub(super) const OAUTH2_TOKEN_VARIABLE: &str = "oauth2AccessToken";

/// What mapping a collection collects on the way.
pub(super) struct Mapping<'a> {
    pub(super) options: &'a PostmanOptions,
    pub(super) warnings: Vec<String>,
    /// Variables of the collection and its folders, for every environment.
    pub(super) variables: Vec<KeyValue>,
    /// Whether a request uses OAuth 2.0, whose token goes to `.local.env`.
    pub(super) oauth2: bool,
}

/// What a request inherits from its folders and the collection, the nearest last.
#[derive(Clone, Default)]
pub(super) struct Inherited {
    headers: Vec<KeyValue>,
    auth: RawAuth,
}

impl Inherited {
    /// What the requests inside a folder with `defaults` inherit.
    pub(super) fn with(&self, defaults: &Defaults) -> Inherited {
        let mut headers = self.headers.clone();
        for header in &defaults.headers {
            headers.retain(|known| !known.key.eq_ignore_ascii_case(&header.key));
            headers.push(header.clone());
        }
        Inherited {
            headers,
            auth: defaults.auth.clone().unwrap_or_else(|| self.auth.clone()),
        }
    }
}

/// Collects what a folder (or the collection) gives that Brasa keeps elsewhere: its variables
/// go to every environment, its scripts are reported.
pub(super) fn take_defaults(defaults: &Defaults, what: &str, mapping: &mut Mapping) {
    for variable in defaults.vars.iter().filter(|variable| variable.enabled) {
        mapping.variables.retain(|known| known.key != variable.key);
        mapping.variables.push(variable.clone());
    }
    for script in &defaults.scripts {
        mapping.warnings.push(format!(
            "{what}: a {script} is not imported at that level, only per request"
        ));
    }
}

/// Maps `folder` and everything inside it.
pub(super) fn map_folder(
    folder: &RawFolder,
    inherited: &Inherited,
    path: &str,
    mapping: &mut Mapping,
) -> ImportFolder {
    let inherited = inherited.with(&folder.defaults);
    let mut out = ImportFolder {
        name: folder.name.clone(),
        folders: Vec::new(),
        requests: Vec::new(),
        issues: Vec::new(),
    };
    let mut requests: Vec<&RawRequest> = folder.requests.iter().collect();
    requests.sort_by(|a, b| a.seq.total_cmp(&b.seq).then_with(|| a.name.cmp(&b.name)));
    for request in requests {
        if !matches!(request.kind.as_str(), "http" | "graphql") {
            mapping.warnings.push(format!(
                "{path}{}: a {} request is not imported, Brasa sends HTTP requests",
                request.name, request.kind
            ));
            continue;
        }
        let (mapped, issues) = map_request(request, &inherited, mapping);
        out.requests.push((request.name.clone(), mapped));
        out.issues.push(issues);
    }
    let mut folders: Vec<&RawFolder> = folder.folders.iter().collect();
    folders.sort_by(|a, b| a.seq.total_cmp(&b.seq).then_with(|| a.name.cmp(&b.name)));
    for child in folders {
        take_defaults(
            &child.defaults,
            &format!("folder \"{}\"", child.name),
            mapping,
        );
        let child_path = format!("{path}{}/", child.name);
        let mapped = map_folder(child, &inherited, &child_path, mapping);
        if mapping.options.folders {
            out.folders.push(mapped);
        } else {
            flatten_into(&mut out, mapped);
        }
    }
    out
}

/// Moves every request of `folder`, at any depth, into `target`.
fn flatten_into(target: &mut ImportFolder, folder: ImportFolder) {
    target.requests.extend(folder.requests);
    target.issues.extend(folder.issues);
    for child in folder.folders {
        flatten_into(target, child);
    }
}

/// The URL with its `:name` path parameters filled in, and without its query string when the
/// parameters are listed apart (Bruno keeps both, Brasa would send them twice).
fn request_url(request: &RawRequest) -> String {
    let mut url = request.url.clone();
    if !request.query.is_empty()
        && let Some(at) = url.find('?')
    {
        url.truncate(at);
    }
    for param in request.path.iter().filter(|param| param.enabled) {
        let placeholder = format!(":{}", param.key);
        let mut result = String::with_capacity(url.len());
        let mut rest = url.as_str();
        while let Some(at) = rest.find(&placeholder) {
            let after = &rest[at + placeholder.len()..];
            let whole = !after.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-');
            result.push_str(&rest[..at]);
            if whole {
                result.push_str(&param.value);
            } else {
                result.push_str(&placeholder);
            }
            rest = after;
        }
        result.push_str(rest);
        url = result;
    }
    url
}

/// The body of a GraphQL request: the JSON object a GraphQL server expects in a POST.
fn graphql_body(query: &str, variables: &str) -> String {
    let variables = variables.trim();
    let variables = if variables.is_empty() {
        "{}"
    } else {
        variables
    };
    let query = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".to_string());
    format!("{{\n  \"query\": {query},\n  \"variables\": {variables}\n}}")
}

fn map_body(body: &RawBody, issues: &mut Vec<RequestIssue>) -> Body {
    match body {
        RawBody::None => Body::None,
        RawBody::Json(text) => Body::Json(text.clone()),
        RawBody::Text(text) => Body::Text(text.clone()),
        RawBody::Xml(text) => Body::Xml(text.clone()),
        RawBody::Form(fields) => Body::Form(fields.clone()),
        RawBody::GraphQl { query, variables } => Body::Json(graphql_body(query, variables)),
        RawBody::Multipart(fields) => {
            let mut out = Vec::new();
            for entry in fields {
                let mut field = entry.field.clone();
                if let Some(files) = &entry.files {
                    if !issues.contains(&RequestIssue::FormDataFile) {
                        issues.push(RequestIssue::FormDataFile);
                    }
                    if files.len() > 1 && !issues.contains(&RequestIssue::FormDataManyFiles) {
                        issues.push(RequestIssue::FormDataManyFiles);
                    }
                    field.value = format!("@{}", files.first().map_or("", String::as_str));
                }
                out.push(field);
            }
            Body::Multipart(out)
        }
        RawBody::Other(kind) => {
            issues.push(RequestIssue::BodyUnsupported(kind.clone()));
            Body::None
        }
    }
}

fn map_auth(auth: &RawAuth, mapping: &mut Mapping, issues: &mut Vec<RequestIssue>) -> Auth {
    match auth {
        RawAuth::None | RawAuth::Inherit => Auth::None,
        RawAuth::Basic { username, password } => Auth::Basic {
            username: username.clone(),
            password: password.clone(),
        },
        RawAuth::Bearer { token } => Auth::Bearer {
            token: token.clone(),
        },
        RawAuth::ApiKey { key, value, query } => Auth::ApiKey {
            name: key.clone(),
            value: value.clone(),
            location: if *query {
                ApiKeyLocation::Query
            } else {
                ApiKeyLocation::Header
            },
        },
        RawAuth::OAuth2 => {
            mapping.oauth2 = true;
            issues.push(RequestIssue::OAuth2Token);
            Auth::Bearer {
                token: format!("{{{{{OAUTH2_TOKEN_VARIABLE}}}}}"),
            }
        }
        RawAuth::Other(kind) => {
            issues.push(RequestIssue::AuthUnsupported(kind.clone()));
            Auth::None
        }
    }
}

/// `source` commented out under the header of an import that does not convert scripts.
fn not_converted(source: &str) -> String {
    let lines: Vec<String> = source.lines().map(|line| format!("// {line}")).collect();
    format!("{NOT_CONVERTED_HEADER}\n{}", lines.join("\n"))
}

/// Joins the non empty `parts` with a blank line.
fn join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| part.trim_end())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Builds the pre and post scripts of a request, in the order Bruno runs its parts: request
/// variables, then the pre-request script; post-response variables, the post-response script,
/// the assertions, then the tests.
fn map_scripts(
    request: &RawRequest,
    mapping: &Mapping,
    issues: &mut Vec<RequestIssue>,
) -> (String, String) {
    let has_any = !request.pre_script.trim().is_empty()
        || !request.post_script.trim().is_empty()
        || !request.tests.trim().is_empty()
        || request.asserts.iter().any(|entry| entry.enabled)
        || request.vars_pre.iter().any(|entry| entry.enabled)
        || request.vars_post.iter().any(|entry| entry.enabled);
    if !has_any {
        return (String::new(), String::new());
    }
    if !mapping.options.scripts {
        issues.push(RequestIssue::ScriptNotConverted);
        let pre = join(std::slice::from_ref(&request.pre_script));
        let post = join(&[request.post_script.clone(), request.tests.clone()]);
        let wrap = |text: String| {
            if text.is_empty() {
                text
            } else {
                not_converted(&text)
            }
        };
        return (wrap(pre), wrap(post));
    }
    let (mut total, mut converted, mut send_request) = (0, 0, false);
    let mut script = |source: &str| {
        if source.trim().is_empty() {
            return String::new();
        }
        let result = convert_script(source);
        total += result.total;
        converted += result.converted;
        send_request |= result.send_request;
        result.text
    };
    let pre_script = script(&request.pre_script);
    let post_script = script(&request.post_script);
    let tests = script(&request.tests);
    let pre_vars = pre_request_vars(&request.vars_pre);
    let post_vars = post_response_vars(&request.vars_post);
    let asserts = assertions_to_tests(&request.asserts);
    for generated in [&pre_vars, &post_vars, &asserts] {
        total += generated.total;
        converted += generated.converted;
    }
    let pre = join(&[pre_vars.text, pre_script]);
    let post = join(&[post_vars.text, post_script, asserts.text, tests]);
    issues.push(if converted == total {
        RequestIssue::ScriptConverted
    } else {
        RequestIssue::ScriptPartial { converted, total }
    });
    if send_request {
        issues.push(RequestIssue::SendRequest);
    }
    (pre, post)
}

/// Maps one request, with what it inherits.
fn map_request(
    request: &RawRequest,
    inherited: &Inherited,
    mapping: &mut Mapping,
) -> (Request, Vec<RequestIssue>) {
    let mut issues = Vec::new();
    let mut headers = inherited.headers.clone();
    for header in &request.headers {
        headers.retain(|known| !known.key.eq_ignore_ascii_case(&header.key));
        headers.push(header.clone());
    }
    let auth = match &request.auth {
        RawAuth::Inherit => inherited.auth.clone(),
        own => own.clone(),
    };
    let auth = if mapping.options.auth {
        map_auth(&auth, mapping, &mut issues)
    } else {
        Auth::None
    };
    let mut body = map_body(&request.body, &mut issues);
    if request.kind == "graphql"
        && !headers
            .iter()
            .any(|header| header.key.eq_ignore_ascii_case("content-type"))
    {
        headers.push(KeyValue::new("Content-Type", "application/json"));
    }
    if matches!(body, Body::None) && request.kind == "graphql" {
        body = Body::Json(graphql_body("", ""));
    }
    let (pre_script, post_script) = map_scripts(request, mapping, &mut issues);
    let mapped = Request {
        method: request.method.parse().unwrap_or(Method::Get),
        url: request_url(request),
        headers,
        query: request.query.clone(),
        auth,
        body,
        pre_script,
        pre_enabled: true,
        post_script,
        post_enabled: true,
        docs: if mapping.options.docs {
            request.docs.clone()
        } else {
            String::new()
        },
    };
    (mapped, issues)
}

#[cfg(test)]
mod tests {
    use super::super::model::MultipartEntry;
    use super::*;
    use pretty_assertions::assert_eq;

    fn mapping(options: &PostmanOptions) -> Mapping<'_> {
        Mapping {
            options,
            warnings: Vec::new(),
            variables: Vec::new(),
            oauth2: false,
        }
    }

    fn request(url: &str) -> RawRequest {
        RawRequest {
            name: "r".to_string(),
            kind: "http".to_string(),
            method: "GET".to_string(),
            url: url.to_string(),
            ..RawRequest::default()
        }
    }

    #[test]
    fn path_params_fill_the_url_and_listed_query_params_leave_it() {
        let mut raw = request("{{host}}/users/:id/posts/:idx?page=1&size=2");
        raw.path = vec![KeyValue::new("id", "{{userId}}"), KeyValue::new("idx", "3")];
        raw.query = vec![KeyValue::new("page", "1"), KeyValue::new("size", "2")];
        assert_eq!(request_url(&raw), "{{host}}/users/{{userId}}/posts/3");

        let alone = request("https://a.test/x?keep=1");
        assert_eq!(request_url(&alone), "https://a.test/x?keep=1");
    }

    #[test]
    fn inherited_headers_and_auth() {
        let options = PostmanOptions::default();
        let mut mapping = mapping(&options);
        let collection = Defaults {
            headers: vec![KeyValue::new("X-App", "1"), KeyValue::new("Accept", "a")],
            auth: Some(RawAuth::Bearer {
                token: "{{token}}".to_string(),
            }),
            ..Defaults::default()
        };
        let folder = Defaults {
            headers: vec![KeyValue::new("accept", "b")],
            ..Defaults::default()
        };
        let inherited = Inherited::default().with(&collection).with(&folder);
        let mut raw = request("/x");
        raw.auth = RawAuth::Inherit;
        raw.headers = vec![KeyValue::new("X-App", "2")];
        let (mapped, issues) = map_request(&raw, &inherited, &mut mapping);
        assert_eq!(
            mapped.headers,
            vec![KeyValue::new("accept", "b"), KeyValue::new("X-App", "2")]
        );
        assert_eq!(
            mapped.auth,
            Auth::Bearer {
                token: "{{token}}".to_string()
            }
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn graphql_becomes_a_json_post_body() {
        let options = PostmanOptions::default();
        let mut mapping = mapping(&options);
        let mut raw = request("/graphql");
        raw.kind = "graphql".to_string();
        raw.method = "POST".to_string();
        raw.body = RawBody::GraphQl {
            query: "{ user(id: 1) { name } }".to_string(),
            variables: String::new(),
        };
        let (mapped, _) = map_request(&raw, &Inherited::default(), &mut mapping);
        assert_eq!(
            mapped.body,
            Body::Json(
                "{\n  \"query\": \"{ user(id: 1) { name } }\",\n  \"variables\": {}\n}".to_string()
            )
        );
        assert_eq!(
            mapped.headers,
            vec![KeyValue::new("Content-Type", "application/json")]
        );
    }

    #[test]
    fn multipart_files_oauth2_and_unsupported_parts() {
        let options = PostmanOptions::default();
        let mut mapping = mapping(&options);
        let mut raw = request("/upload");
        raw.body = RawBody::Multipart(vec![MultipartEntry {
            field: KeyValue::new("doc", "@file(a.pdf|b.pdf)"),
            files: Some(vec!["a.pdf".to_string(), "b.pdf".to_string()]),
        }]);
        raw.auth = RawAuth::OAuth2;
        let (mapped, issues) = map_request(&raw, &Inherited::default(), &mut mapping);
        assert_eq!(
            mapped.body,
            Body::Multipart(vec![KeyValue::new("doc", "@a.pdf")])
        );
        assert_eq!(
            mapped.auth,
            Auth::Bearer {
                token: "{{oauth2AccessToken}}".to_string()
            }
        );
        assert!(mapping.oauth2);
        assert_eq!(
            issues,
            vec![
                RequestIssue::OAuth2Token,
                RequestIssue::FormDataFile,
                RequestIssue::FormDataManyFiles
            ]
        );

        let mut other = request("/s");
        other.body = RawBody::Other("sparql".to_string());
        other.auth = RawAuth::Other("awsv4".to_string());
        let (mapped, issues) = map_request(&other, &Inherited::default(), &mut mapping);
        assert_eq!((mapped.body, mapped.auth), (Body::None, Auth::None));
        assert_eq!(
            issues,
            vec![
                RequestIssue::AuthUnsupported("awsv4".to_string()),
                RequestIssue::BodyUnsupported("sparql".to_string())
            ]
        );
    }

    #[test]
    fn scripts_follow_bruno_order() {
        let options = PostmanOptions::default();
        let mut mapping = mapping(&options);
        let mut raw = request("/login");
        raw.vars_pre = vec![KeyValue::new("day", "1")];
        raw.pre_script = "bru.setVar('a', 1);".to_string();
        raw.vars_post = vec![KeyValue::new("token", "$res.body.token")];
        raw.post_script = "console.log(res.status);".to_string();
        raw.asserts = vec![KeyValue::new("$res.status", "eq 200")];
        raw.tests = "test('t', () => {\n  expect(res.status).to.equal(200);\n});".to_string();
        let (mapped, issues) = map_request(&raw, &Inherited::default(), &mut mapping);
        assert_eq!(
            mapped.pre_script,
            "vars.set(\"day\", \"1\");\n\nenv.set('a', 1);"
        );
        assert_eq!(
            mapped.post_script,
            "env.set(\"token\", res.json().token);\n\nconsole.log(res.status);\n\ntest(\"$res.status: eq 200\", () => {\n  expect(res.status).toBe(200);\n});\n\ntest('t', () => {\n  expect(res.status).toBe(200);\n});"
        );
        assert_eq!(issues, vec![RequestIssue::ScriptConverted]);

        let off = PostmanOptions {
            scripts: false,
            ..PostmanOptions::default()
        };
        let mut mapping = super::Mapping {
            options: &off,
            warnings: Vec::new(),
            variables: Vec::new(),
            oauth2: false,
        };
        let (mapped, issues) = map_request(&raw, &Inherited::default(), &mut mapping);
        assert!(mapped.pre_script.starts_with(NOT_CONVERTED_HEADER));
        assert!(mapped.pre_script.contains("// bru.setVar('a', 1);"));
        assert_eq!(issues, vec![RequestIssue::ScriptNotConverted]);
    }
}
