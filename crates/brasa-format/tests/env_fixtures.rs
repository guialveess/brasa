//! Fixture-based tests for `.env` environment file parsing.
#![allow(clippy::expect_used)]

use brasa_core::KeyValue;
use brasa_format::env;
use pretty_assertions::assert_eq;

#[test]
fn basic_fixture_parses() {
    let variables = env::parse(include_str!("fixtures/basic.env")).expect("valid fixture");
    assert_eq!(
        variables,
        vec![
            KeyValue::new("BASE_URL", "https://api.test"),
            KeyValue::new("TOKEN", "abc123"),
        ]
    );
}

#[test]
fn fixture_with_comments_ignores_them() {
    let variables = env::parse(include_str!("fixtures/with_comments.env")).expect("valid fixture");
    assert_eq!(
        variables,
        vec![
            KeyValue::new("BASE_URL", "https://api.test"),
            KeyValue::new("TOKEN", "abc123"),
        ]
    );
}

#[test]
fn local_env_fixture_parses() {
    let variables = env::parse(include_str!("fixtures/secrets.local.env")).expect("valid fixture");
    assert_eq!(
        variables,
        vec![KeyValue::new("API_KEY", "super-secret-value")]
    );
}
