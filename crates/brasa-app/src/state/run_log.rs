//! The log lines written for a request sent from a tab (GitHub #36): one Info summary and one
//! Debug detail line, built here as plain strings so their privacy rules are unit tested. Neither
//! ever includes a query string, a body, a variable value or a script's own error message, which
//! may echo any of those.

use std::time::Duration;

use brasa_core::{TemplateWarning, log_safe};
use brasa_runner::{FailedStage, RunResult};

use super::response_render::format_size;

/// The Info line: method, URL without query or credentials, and the status, time and size, or
/// which stage failed. `elapsed` is the whole run, scripts included.
pub fn summary(result: &RunResult, elapsed: Duration) -> String {
    let target = match &result.resolved_request {
        Some(request) => format!("{} {}", request.method, log_safe::url_for_log(&request.url)),
        None => "request".to_string(),
    };
    let outcome = match (&result.response, &result.failed_stage) {
        (_, Some(FailedStage::Pre(_))) => "not sent: the pre-request script failed".to_string(),
        (_, Some(FailedStage::Send(error))) => format!("failed: {error}"),
        (Some(response), failed) => {
            let mut outcome = format!(
                "{} in {} ms, {}",
                response.status,
                response.time.as_millis(),
                format_size(response.size)
            );
            if response.tls_warning.is_some() {
                outcome.push_str(", TLS certificate not verified");
            }
            if matches!(failed, Some(FailedStage::Post(_))) {
                outcome.push_str(", the post-response script failed");
            }
            outcome
        }
        (None, _) => "no response".to_string(),
    };
    format!("{target} -> {outcome} (total {} ms)", elapsed.as_millis())
}

/// The Debug line: test results, console line count and the names of unresolved variables.
pub fn details(result: &RunResult) -> String {
    let passed = result.tests.iter().filter(|test| test.passed).count();
    let failed = result.tests.len() - passed;
    let mut unknown: Vec<&str> = Vec::new();
    let mut function_errors = 0;
    for warning in &result.warnings {
        match warning {
            TemplateWarning::UnknownVariable(name) => {
                if !unknown.contains(&name.as_str()) {
                    unknown.push(name);
                }
            }
            TemplateWarning::Function(_) => function_errors += 1,
        }
    }
    let mut line = format!(
        "tests: {passed} passed, {failed} failed; console: {} lines",
        result.console.len()
    );
    if !unknown.is_empty() {
        line.push_str(&format!("; unknown variables: {}", unknown.join(", ")));
    }
    if function_errors > 0 {
        line.push_str(&format!("; template function errors: {function_errors}"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use brasa_core::{Method, ResolvedBody, ResolvedRequest, Response, TestResult};
    use pretty_assertions::assert_eq;

    fn resolved(url: &str) -> ResolvedRequest {
        ResolvedRequest {
            method: Method::Get,
            url: url.to_string(),
            headers: Vec::new(),
            body: ResolvedBody::None,
        }
    }

    fn response(status: u16) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: b"secret body".to_vec(),
            time: Duration::from_millis(42),
            size: 2048,
            tls_warning: None,
        }
    }

    fn result(url: &str, response: Option<Response>) -> RunResult {
        RunResult {
            resolved_request: Some(resolved(url)),
            response,
            tests: Vec::new(),
            console: Vec::new(),
            warnings: Vec::new(),
            failed_stage: None,
            started_at: std::time::UNIX_EPOCH,
            sent_at: None,
            finished_at: std::time::UNIX_EPOCH,
        }
    }

    #[test]
    fn summary_hides_the_query_and_credentials() {
        let line = summary(
            &result(
                "https://me:pw@api.example.com/items?token=abc",
                Some(response(200)),
            ),
            Duration::from_millis(50),
        );
        assert_eq!(
            line,
            "GET https://api.example.com/items -> 200 in 42 ms, 2.0 KB (total 50 ms)"
        );
    }

    #[test]
    fn summary_reports_a_send_failure() {
        let mut run = result("http://localhost:1/", None);
        run.failed_stage = Some(FailedStage::Send(brasa_runner::HttpError::Timeout));
        assert_eq!(
            summary(&run, Duration::from_millis(5)),
            "GET http://localhost:1/ -> failed: the request timed out (total 5 ms)"
        );
    }

    #[test]
    fn details_lists_unknown_variable_names_once_and_never_values() {
        let mut run = result("http://localhost/", Some(response(200)));
        run.tests = vec![
            TestResult {
                name: "ok".to_string(),
                passed: true,
                message: None,
            },
            TestResult {
                name: "ko".to_string(),
                passed: false,
                message: Some("expected 1".to_string()),
            },
        ];
        run.warnings = vec![
            TemplateWarning::UnknownVariable("token".to_string()),
            TemplateWarning::UnknownVariable("token".to_string()),
            TemplateWarning::UnknownVariable("host".to_string()),
        ];
        assert_eq!(
            details(&run),
            "tests: 1 passed, 1 failed; console: 0 lines; unknown variables: token, host"
        );
    }
}
