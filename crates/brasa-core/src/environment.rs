//! A named set of variables loaded from `environments/<name>.env`.

use crate::key_value::KeyValue;

/// A named environment: the variables loaded from `environments/<name>.env`, merged with the
/// optional `<name>.local.env`. Loading and merging happen in `brasa-workspace`, this type just
/// holds the result.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Environment {
    /// The environment name, the file name without `.env`.
    pub name: String,
    /// The variables, in file order. `enabled` is always `true`, the `.env` format has no
    /// concept of a disabled entry.
    pub variables: Vec<KeyValue>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn default_is_empty() {
        let environment = Environment::default();
        assert_eq!(environment.name, "");
        assert!(environment.variables.is_empty());
    }
}
