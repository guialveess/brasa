//! Fixture-based round-trip tests for the `.brasa` format: `serialize(parse(text)) == text` for
//! every canonically formatted fixture file in `tests/fixtures`.
#![allow(clippy::expect_used)]

use brasa_format::{parse, serialize};
use pretty_assertions::assert_eq;

/// Parses `text`, then asserts that serializing the result reproduces it exactly.
fn assert_round_trips(text: &str) {
    let request = parse(text).expect("fixture must parse");
    assert_eq!(serialize(&request), text);
}

#[test]
fn full_example_fixture_round_trips() {
    assert_round_trips(include_str!("fixtures/full.brasa"));
}

#[test]
fn minimal_fixture_round_trips() {
    assert_round_trips(include_str!("fixtures/minimal.brasa"));
}

#[test]
fn headers_only_fixture_round_trips() {
    assert_round_trips(include_str!("fixtures/headers_only.brasa"));
}

#[test]
fn disabled_entries_fixture_round_trips() {
    assert_round_trips(include_str!("fixtures/disabled_entries.brasa"));
}

#[test]
fn body_form_fixture_round_trips() {
    assert_round_trips(include_str!("fixtures/body_form.brasa"));
}

#[test]
fn escaped_markers_fixture_round_trips() {
    assert_round_trips(include_str!("fixtures/escaped_markers.brasa"));
}

#[test]
fn unicode_fixture_round_trips() {
    assert_round_trips(include_str!("fixtures/unicode.brasa"));
}

#[test]
fn custom_method_fixture_round_trips() {
    assert_round_trips(include_str!("fixtures/custom_method.brasa"));
}
