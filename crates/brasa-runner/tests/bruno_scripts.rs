//! Scripts, assertions and request variables converted from Bruno run for real: through `Runner`
//! and QuickJS, against a local server, so a rewrite that names something the sandbox does not
//! have, or an assertion that compiles to the wrong check, fails here.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use brasa_core::{Environment, KeyValue};
use brasa_format::bruno::{BrunoFile, parse_bruno_collection};
use brasa_format::postman::{PostmanOptions, RequestIssue};
use brasa_http::SendOptions;
use brasa_http::test_support::{StubResponse, TestServer};
use brasa_runner::{Runner, SessionEnv};
use brasa_script::QuickJsEngine;

const REQUEST: &str = r#"meta {
  name: Everything
  type: http
  seq: 1
}

post {
  url: {{baseUrl}}/anything/:id?page=1
  body: json
  auth: none
}

params:query {
  page: 1
}

params:path {
  id: 7
}

body:json {
  {"a": 1}
}

vars:pre-request {
  local: two
}

vars:post-response {
  token: $res.body.token
  @order: $res.body.count
}

assert {
  $res.status: eq 200
  $res.body.ok: isTruthy
  $res.body.name: eq "Ann"
  $res.body.count: gte 3
  $res.body.count: between 1, 5
  $res.body.list: isArray
  $res.body.list: length 2
  $res.body.name: startsWith A
  $res.body.name: in Ann, Bob
  $res.body.missing: isUndefined
  $res.headers.content-type: contains json
  $res.body.local: eq {{local}}
}

script:pre-request {
  bru.setEnvVar('stamp', 'one');
  bru.setVar('runtime', 'yes');
  req.setHeader('X-Stamp', bru.getEnvVar('stamp'));
  const method = req.getMethod();
  const url = req.getUrl();
  const body = req.getBody();
  req.setBody({ a: body.a + 1, method: method, local: bru.getRequestVar('local') });
}

script:post-response {
  bru.setVar('status', res.getStatus());
}

tests {
  test('status and timing', function () {
    expect(res.status).to.equal(200);
    expect(res.getStatus()).to.equal(200);
    expect(res.getResponseTime()).to.be.above(-1);
    expect(res.getHeader('content-type')).to.include('json');
  });

  test('body', () => {
    const data = res.getBody();
    expect(data).to.have.property('ok');
    expect(data.ok).to.be.true;
    expect(res.body.list).to.eql([1, 2]);
    expect(res.body.missing).to.not.exist;
    expect(res.body.name).to.not.equal('Bob');
  });

  test('variables', () => {
    expect(bru.getEnvVar('stamp')).to.equal('one');
    expect(bru.getVar('runtime')).to.equal('yes');
    // Brasa's variables are text: Bruno's typed runtime variables come back as strings.
    expect(bru.getVar('status')).to.equal('200');
  });
}
"#;

#[test]
fn converted_bruno_scripts_run_in_the_sandbox() {
    let server = TestServer::start();
    let _handle = server.serve(|_| {
        StubResponse::new(200)
            .with_header("Content-Type", "application/json")
            .with_body(
                r#"{"ok":true,"list":[1,2],"token":"abc","name":"Ann","count":3,"local":"two"}"#,
            )
    });
    let files = [
        BrunoFile {
            path: "bruno.json".to_string(),
            text: r#"{"name": "x"}"#.to_string(),
        },
        BrunoFile {
            path: "Everything.bru".to_string(),
            text: REQUEST.to_string(),
        },
    ];
    let import =
        parse_bruno_collection(&files, "x", &PostmanOptions::default()).expect("a collection");
    let (_, request) = &import.plan.root.requests[0];
    assert_eq!(
        import.plan.root.issues[0],
        vec![RequestIssue::ScriptConverted],
        "{}\n{}",
        request.pre_script,
        request.post_script
    );
    for script in [&request.pre_script, &request.post_script] {
        assert!(!script.contains("bru."), "{script}");
        assert!(!script.contains("Imported from Bruno"), "{script}");
    }

    let environment = Environment {
        name: "test".to_string(),
        variables: vec![KeyValue::new("baseUrl", server.url())],
    };
    let mut session = SessionEnv::new();
    let runner = Runner::new(Arc::new(QuickJsEngine), SendOptions::default());
    let result = runner.run(request, &environment, &mut session);

    assert!(result.failed_stage.is_none(), "{:?}", result.failed_stage);
    assert_eq!(result.tests.len(), 15, "{:?}", result.tests);
    for test in &result.tests {
        assert!(test.passed, "{test:?}");
    }
    let resolved = result.resolved_request.expect("sent");
    assert!(
        resolved.url.ends_with("/anything/7?page=1"),
        "{}",
        resolved.url
    );
    let session: Vec<(String, String)> = session
        .as_slice()
        .iter()
        .map(|entry| (entry.key.clone(), entry.value.clone()))
        .collect();
    assert!(
        session.contains(&("token".to_string(), "abc".to_string())),
        "{session:?}"
    );
}
