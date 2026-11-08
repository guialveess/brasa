//! Converting a raw Postman JSON `value` field into plain text.
//!
//! Postman's own exports always use plain strings, but the JSON schema allows any scalar (and,
//! technically, arrays and objects too) wherever a `value` is expected. [`value_to_text`] is the
//! single place that turns whatever was found into the plain string a Brasa [`KeyValue`] or
//! [`crate::env`] entry needs.
//!
//! [`KeyValue`]: brasa_core::KeyValue

use serde_json::Value;

/// Converts a Postman `value` field into plain text.
///
/// A string is used as is. A number or boolean is converted to its usual text form (`"true"`,
/// `"42"`). `null`, or the field being absent entirely, becomes an empty string. An array or
/// object has no natural text form: its compact JSON text is used instead, and a warning naming
/// `context` is recorded in `warnings`, so the user knows the value was not a plain scalar.
pub(super) fn value_to_text(
    value: Option<&Value>,
    context: &str,
    warnings: &mut Vec<String>,
) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(boolean)) => boolean.to_string(),
        Some(other @ (Value::Array(_) | Value::Object(_))) => {
            warnings.push(format!(
                "{context}: value is not a plain string, number or boolean, using its JSON text"
            ));
            other.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn string_is_used_as_is() {
        let mut warnings = Vec::new();
        let value = Value::String("hello".to_string());
        assert_eq!(value_to_text(Some(&value), "ctx", &mut warnings), "hello");
        assert!(warnings.is_empty());
    }

    #[test]
    fn number_and_boolean_are_stringified() {
        let mut warnings = Vec::new();
        assert_eq!(
            value_to_text(Some(&Value::from(42)), "ctx", &mut warnings),
            "42"
        );
        assert_eq!(
            value_to_text(Some(&Value::from(true)), "ctx", &mut warnings),
            "true"
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn null_and_absent_are_empty() {
        let mut warnings = Vec::new();
        assert_eq!(value_to_text(Some(&Value::Null), "ctx", &mut warnings), "");
        assert_eq!(value_to_text(None, "ctx", &mut warnings), "");
        assert!(warnings.is_empty());
    }

    #[test]
    fn array_or_object_uses_json_text_and_warns() {
        let mut warnings = Vec::new();
        let value = Value::from(vec![1, 2]);
        assert_eq!(value_to_text(Some(&value), "ctx", &mut warnings), "[1,2]");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("ctx"));
    }
}
