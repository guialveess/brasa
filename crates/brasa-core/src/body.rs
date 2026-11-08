//! The body of a request.

use crate::key_value::KeyValue;

/// The body of a [`crate::Request`], as edited before interpolation.
///
/// Mirrors the `body <type>` section of the `.brasa` format: `json`, `text` and `xml` are raw
/// text, `form` is a list of urlencoded fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Body {
    /// No body.
    #[default]
    None,
    /// A JSON body, kept as raw text so it can hold `{{ }}` placeholders that are not valid
    /// JSON on their own.
    Json(String),
    /// A plain text body.
    Text(String),
    /// An XML body.
    Xml(String),
    /// A `application/x-www-form-urlencoded` body.
    Form(Vec<KeyValue>),
    /// A `multipart/form-data` body. A field whose value starts with `@` sends the file at that
    /// path (see [`multipart_value`]).
    Multipart(Vec<KeyValue>),
}

/// What the value of a [`Body::Multipart`] field sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultipartValue<'a> {
    /// This text.
    Text(&'a str),
    /// The contents of the file at this path.
    File(&'a str),
}

/// Reads the value of a [`Body::Multipart`] field: `@path` is a file and `@@text` is the text
/// `@text`, anything else is itself.
pub fn multipart_value(value: &str) -> MultipartValue<'_> {
    match value.strip_prefix('@') {
        Some(rest) if rest.starts_with('@') => MultipartValue::Text(rest),
        Some(path) => MultipartValue::File(path),
        None => MultipartValue::Text(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn a_multipart_value_is_text_or_a_file() {
        assert_eq!(multipart_value("plain"), MultipartValue::Text("plain"));
        assert_eq!(multipart_value("@a/b.png"), MultipartValue::File("a/b.png"));
        assert_eq!(multipart_value("@@handle"), MultipartValue::Text("@handle"));
    }

    #[test]
    fn default_is_none() {
        assert_eq!(Body::default(), Body::None);
    }
}
