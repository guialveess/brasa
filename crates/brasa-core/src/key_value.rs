//! A name-value pair used for headers, query parameters, form fields and variables.

/// A name-value pair with an enabled flag.
///
/// Used for request headers, query parameters and form fields, all of which can be disabled in a
/// `.brasa` file with a leading `#`. It is also reused for environment variables, where `enabled`
/// is always `true`: the `.env` format has no concept of a disabled entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValue {
    /// The entry name (header name, query key, form field name or variable name).
    pub key: String,
    /// The entry value, before variable interpolation.
    pub value: String,
    /// Whether the entry is active. A disabled entry is kept (so it round-trips) but ignored
    /// when resolving a request or looking up a variable.
    pub enabled: bool,
}

impl KeyValue {
    /// Creates a new, enabled entry.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        KeyValue {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }
}

impl Default for KeyValue {
    /// An empty, enabled entry, a sensible starting point for a newly added row in the UI.
    fn default() -> Self {
        KeyValue {
            key: String::new(),
            value: String::new(),
            enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn new_is_enabled() {
        let entry = KeyValue::new("Content-Type", "application/json");
        assert_eq!(entry.key, "Content-Type");
        assert_eq!(entry.value, "application/json");
        assert!(entry.enabled);
    }

    #[test]
    fn default_is_empty_and_enabled() {
        let entry = KeyValue::default();
        assert_eq!(entry, KeyValue::new("", ""));
        assert!(entry.enabled);
    }
}
