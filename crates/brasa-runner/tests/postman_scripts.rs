//! Scripts converted from Postman run for real: through `Runner` and QuickJS, against a local
//! server, so a rewrite that names something the sandbox does not have fails here.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use brasa_core::{Environment, KeyValue};
use brasa_format::postman::{PostmanOptions, parse_collection};
use brasa_http::SendOptions;
use brasa_http::test_support::{StubResponse, TestServer};
use brasa_runner::{Runner, SessionEnv};
use brasa_script::QuickJsEngine;

const COLLECTION: &str = r#"{
  "info": {"name": "x"},
  "item": [{
    "name": "Everything",
    "request": {"method": "POST", "url": "{{baseUrl}}/anything", "header": [],
      "body": {"mode": "raw", "raw": "{}"}},
    "event": [
      {"listen": "prerequest", "script": {"exec": [
        "pm.environment.set('stamp', 'one');",
        "pm.variables.set('local', 'two');",
        "pm.request.headers.add({ key: 'X-Stamp', value: pm.environment.get('stamp') });",
        "const method = pm.request.method;",
        "const url = pm.request.url;"
      ]}},
      {"listen": "test", "script": {"exec": [
        "pm.test('status and timing', function () {",
        "  pm.response.to.have.status(200);",
        "  pm.response.to.be.ok;",
        "  pm.expect(pm.response.code).to.equal(200);",
        "  pm.expect(pm.response.responseTime).to.be.above(-1);",
        "  pm.expect(pm.response.text()).to.include('list');",
        "});",
        "pm.test('body', () => {",
        "  const body = pm.response.json();",
        "  pm.expect(body).to.have.property('ok');",
        "  pm.expect(body.ok).to.be.true;",
        "  pm.expect(body.list).to.include(1);",
        "  pm.expect(body.list).to.eql([1, 2]);",
        "  pm.expect(body.missing).to.not.exist;",
        "  pm.expect(pm.response.headers.get('content-type')).to.include('json');",
        "});",
        "pm.test('variables', () => {",
        "  pm.collectionVariables.set('kept', 'yes');",
        "  pm.globals.set('global', 'yes');",
        "  pm.expect(pm.environment.get('kept')).to.equal('yes');",
        "  pm.expect(pm.variables.get('kept')).to.equal('yes');",
        "  pm.environment.unset('kept');",
        "});"
      ]}}
    ]
  }]
}"#;

#[test]
fn converted_scripts_run_in_the_sandbox() {
    let server = TestServer::start();
    let _handle = server.serve(|_| {
        StubResponse::new(200)
            .with_header("Content-Type", "application/json")
            .with_body(r#"{"ok":true,"list":[1,2]}"#)
    });
    let plan = parse_collection(COLLECTION, &PostmanOptions::default()).expect("a collection");
    let request = &plan.root.requests[0].1;
    assert!(
        !request.pre_script.contains("pm."),
        "{}",
        request.pre_script
    );
    assert!(
        !request.post_script.contains("pm."),
        "{}",
        request.post_script
    );

    let environment = Environment {
        name: "test".to_string(),
        variables: vec![KeyValue::new("baseUrl", server.url())],
    };
    let mut session = SessionEnv::new();
    let runner = Runner::new(Arc::new(QuickJsEngine), SendOptions::default());
    let result = runner.run(request, &environment, &mut session);

    assert!(result.failed_stage.is_none(), "{:?}", result.failed_stage);
    assert_eq!(result.tests.len(), 3);
    for test in &result.tests {
        assert!(test.passed, "{test:?}");
    }
}
