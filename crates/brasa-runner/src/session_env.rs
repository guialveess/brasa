//! [`SessionEnv`], the in-memory environment overrides, the second variable layer of `docs/format.md`.

use brasa_core::{Environment, KeyValue};
use brasa_script::EnvChange;

/// The environment overrides a script creates at runtime with `env.set`/`env.unset`
/// (the second variable layer of `docs/format.md`),
/// kept in memory for the app session.
///
/// Kept only in memory for as long as the value lives: the MVP never writes these back to an
/// environment file. One `SessionEnv` is meant to be shared across every [`crate::Runner::run`]
/// call in a session, so a value set by one request's script (for example a login token) is
/// visible to the next request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionEnv {
    overrides: Vec<KeyValue>,
}

impl SessionEnv {
    /// A session environment with no overrides yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current overrides, in the shape [`brasa_core::VarScope::session_env`] expects.
    #[must_use]
    pub fn as_slice(&self) -> &[KeyValue] {
        &self.overrides
    }

    /// Sets `key` to `value`, replacing any existing override with the same key.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        match self.overrides.iter_mut().find(|entry| entry.key == key) {
            Some(entry) => entry.value = value,
            None => self.overrides.push(KeyValue::new(key, value)),
        }
    }

    /// Removes an override, if one exists for `key`. Does nothing otherwise.
    pub fn unset(&mut self, key: &str) {
        self.overrides.retain(|entry| entry.key != key);
    }

    /// Applies a batch of [`EnvChange`]s from a script outcome, in order.
    pub fn apply(&mut self, changes: &[EnvChange]) {
        for change in changes {
            match change {
                EnvChange::Set { key, value } => self.set(key.clone(), value.clone()),
                EnvChange::Unset { key } => self.unset(key),
            }
        }
    }

    /// `environment`'s variables with every session override applied on top (an override wins
    /// over an environment variable with the same key), in the shape a script's `env.get` sees
    /// (the active environment merged with any session overrides, see `docs/scripting.md`).
    #[must_use]
    pub fn merged_with(&self, environment: &Environment) -> Vec<KeyValue> {
        let mut merged = environment.variables.clone();
        for over in &self.overrides {
            match merged.iter_mut().find(|entry| entry.key == over.key) {
                Some(entry) => entry.value = over.value.clone(),
                None => merged.push(over.clone()),
            }
        }
        merged
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn set_adds_a_new_override() {
        let mut session = SessionEnv::new();
        session.set("token", "abc");
        assert_eq!(session.as_slice(), &[KeyValue::new("token", "abc")]);
    }

    #[test]
    fn set_replaces_an_existing_override() {
        let mut session = SessionEnv::new();
        session.set("token", "abc");
        session.set("token", "def");
        assert_eq!(session.as_slice(), &[KeyValue::new("token", "def")]);
    }

    #[test]
    fn unset_removes_an_override() {
        let mut session = SessionEnv::new();
        session.set("token", "abc");
        session.unset("token");
        assert!(session.as_slice().is_empty());
    }

    #[test]
    fn unset_of_a_missing_key_is_a_no_op() {
        let mut session = SessionEnv::new();
        session.unset("missing");
        assert!(session.as_slice().is_empty());
    }

    #[test]
    fn apply_replays_a_batch_of_changes_in_order() {
        let mut session = SessionEnv::new();
        session.apply(&[
            EnvChange::Set {
                key: "a".to_string(),
                value: "1".to_string(),
            },
            EnvChange::Set {
                key: "b".to_string(),
                value: "2".to_string(),
            },
            EnvChange::Unset {
                key: "a".to_string(),
            },
        ]);
        assert_eq!(session.as_slice(), &[KeyValue::new("b", "2")]);
    }

    #[test]
    fn merged_with_overlays_session_overrides_on_the_environment() {
        let environment = Environment {
            name: "dev".to_string(),
            variables: vec![
                KeyValue::new("baseUrl", "https://dev.example.com"),
                KeyValue::new("token", "stale"),
            ],
        };
        let mut session = SessionEnv::new();
        session.set("token", "fresh");

        let merged = session.merged_with(&environment);
        assert_eq!(
            merged,
            vec![
                KeyValue::new("baseUrl", "https://dev.example.com"),
                KeyValue::new("token", "fresh"),
            ]
        );
    }

    #[test]
    fn merged_with_appends_a_session_only_key() {
        let environment = Environment::default();
        let mut session = SessionEnv::new();
        session.set("sessionOnly", "value");

        assert_eq!(
            session.merged_with(&environment),
            vec![KeyValue::new("sessionOnly", "value")]
        );
    }
}
