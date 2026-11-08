//! Example values for request bodies: the spec's own example when it has one, a value built from
//! the schema otherwise.

use serde_json::{Map, Value};

use super::document::Node;

/// How deep a generated example nests objects and arrays before stopping with an empty one.
const MAX_DEPTH: usize = 6;

/// The most properties a generated object gets, so a huge schema still gives a readable body.
const MAX_PROPERTIES: usize = 40;

/// The example of a media type object (OpenAPI 3) or a body parameter (Swagger 2): its `example`,
/// the first of its `examples`, or one built from its `schema`.
pub(crate) fn media_example(media: Node<'_>) -> Option<Value> {
    if let Some(example) = media.get("example") {
        return Some(example.value().clone());
    }
    if let Some(examples) = media.get("examples") {
        let first = examples
            .entries()
            .into_iter()
            .find_map(|(_, example)| example.get("value"));
        if let Some(value) = first {
            return Some(value.value().clone());
        }
    }
    let schema = media.raw("schema")?;
    Some(schema_example(schema, &mut Vec::new(), 0))
}

/// The example value of a property or parameter schema, `None` when the schema gives nothing more
/// specific than its type.
pub(crate) fn explicit_example(schema: Node<'_>) -> Option<Value> {
    let schema = schema.resolve()?;
    for key in ["example", "x-example", "default", "const"] {
        if let Some(value) = schema.get(key) {
            return Some(value.value().clone());
        }
    }
    if let Some(first) = schema
        .get("examples")
        .and_then(|items| items.items().into_iter().next())
    {
        return Some(first.value().clone());
    }
    schema
        .get("enum")
        .and_then(|values| values.items().into_iter().next())
        .map(|value| value.value().clone())
}

/// Builds an example value from a schema. `stack` holds the `$ref`s being expanded, so a
/// recursive schema stops instead of repeating itself.
pub(crate) fn schema_example(raw: Node<'_>, stack: &mut Vec<String>, depth: usize) -> Value {
    let reference = raw.reference().map(str::to_string);
    if let Some(reference) = &reference {
        if stack.contains(reference) {
            return Value::Null;
        }
        stack.push(reference.clone());
    }
    let value = match raw.resolve() {
        Some(schema) => build(schema, stack, depth),
        None => Value::Null,
    };
    if reference.is_some() {
        stack.pop();
    }
    value
}

fn build(schema: Node<'_>, stack: &mut Vec<String>, depth: usize) -> Value {
    if let Some(value) = explicit_example(schema) {
        return value;
    }
    if let Some(all_of) = schema.raw("allOf") {
        let mut merged = Map::new();
        let mut other = None;
        for part in all_of.raw_items() {
            match schema_example(part, stack, depth) {
                Value::Object(map) => merged.extend(map),
                value => other = other.or(Some(value)),
            }
        }
        // `allOf` may also sit next to its own `properties`.
        if let Value::Object(own) = object(schema, stack, depth) {
            merged.extend(own);
        }
        return if merged.is_empty() {
            other.unwrap_or(Value::Object(merged))
        } else {
            Value::Object(merged)
        };
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(first) = schema
            .raw(key)
            .and_then(|items| items.raw_items().into_iter().next())
        {
            return schema_example(first, stack, depth);
        }
    }
    match schema_type(schema).as_deref() {
        Some("object") => object(schema, stack, depth),
        Some("array") => {
            if depth >= MAX_DEPTH {
                return Value::Array(Vec::new());
            }
            match schema.raw("items") {
                Some(items) => match schema_example(items, stack, depth + 1) {
                    Value::Null => Value::Array(Vec::new()),
                    item => Value::Array(vec![item]),
                },
                None => Value::Array(Vec::new()),
            }
        }
        Some("string") => Value::String(string_example(schema.str("format"))),
        Some("integer") => number_bound(schema).unwrap_or(Value::from(0)),
        Some("number") => number_bound(schema).unwrap_or(Value::from(0)),
        Some("boolean") => Value::Bool(true),
        _ => Value::Null,
    }
}

/// The type of a schema: its `type` (the first that is not `"null"` for a 3.1 type list), or
/// `object`/`array` inferred from `properties`/`items`.
pub(crate) fn schema_type(schema: Node<'_>) -> Option<String> {
    match schema.get("type").map(|node| node.value()) {
        Some(Value::String(kind)) => return Some(kind.clone()),
        Some(Value::Array(kinds)) => {
            if let Some(kind) = kinds
                .iter()
                .filter_map(Value::as_str)
                .find(|kind| *kind != "null")
            {
                return Some(kind.to_string());
            }
        }
        _ => {}
    }
    if schema.raw("properties").is_some() {
        Some("object".to_string())
    } else if schema.raw("items").is_some() {
        Some("array".to_string())
    } else {
        None
    }
}

fn object(schema: Node<'_>, stack: &mut Vec<String>, depth: usize) -> Value {
    let mut map = Map::new();
    if depth >= MAX_DEPTH {
        return Value::Object(map);
    }
    if let Some(properties) = schema.raw("properties").and_then(Node::resolve) {
        for (name, property) in properties.raw_entries().into_iter().take(MAX_PROPERTIES) {
            if property
                .resolve()
                .is_some_and(|resolved| resolved.flag("readOnly"))
            {
                continue;
            }
            map.insert(name.to_string(), schema_example(property, stack, depth + 1));
        }
    }
    Value::Object(map)
}

fn number_bound(schema: Node<'_>) -> Option<Value> {
    schema.get("minimum").map(|node| node.value().clone())
}

/// A plausible string for a string `format`.
fn string_example(format: Option<&str>) -> String {
    match format {
        Some("date-time") => "2024-01-01T00:00:00Z",
        Some("date") => "2024-01-01",
        Some("time") => "00:00:00",
        Some("email") => "user@example.com",
        Some("uuid") => "00000000-0000-0000-0000-000000000000",
        Some("uri" | "url") => "https://example.com",
        Some("hostname") => "example.com",
        Some("ipv4") => "127.0.0.1",
        Some("ipv6") => "::1",
        Some("byte") => "",
        Some("password") => "",
        _ => "string",
    }
    .to_string()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::super::document::{Documents, NoExternalRefs, parse_text};
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn example_of(yaml: &str) -> Value {
        let documents = Documents::load(parse_text(yaml).expect("valid"), "m", &NoExternalRefs);
        let root = documents.root();
        media_example(root.raw("media").expect("media")).expect("an example")
    }

    #[test]
    fn explicit_examples_win() {
        assert_eq!(
            example_of("media: {example: {a: 1}, schema: {type: string}}"),
            json!({"a": 1})
        );
        assert_eq!(
            example_of("media: {examples: {first: {value: [1]}, second: {value: [2]}}}"),
            json!([1])
        );
    }

    #[test]
    fn objects_are_built_from_properties() {
        let yaml = r##"
media:
  schema: {$ref: '#/components/schemas/Customer'}
components:
  schemas:
    Customer:
      type: object
      properties:
        id: {type: string, readOnly: true}
        email: {type: string, format: email}
        age: {type: integer, minimum: 18}
        tier: {type: string, enum: [gold, silver]}
        tags: {type: array, items: {type: string}}
        address: {properties: {city: {type: string, example: Vigo}}}
        note: {type: [string, "null"]}
"##;
        assert_eq!(
            example_of(yaml),
            json!({
                "email": "user@example.com",
                "age": 18,
                "tier": "gold",
                "tags": ["string"],
                "address": {"city": "Vigo"},
                "note": "string"
            })
        );
    }

    #[test]
    fn all_of_merges_and_one_of_takes_the_first() {
        let yaml = r##"
media:
  schema:
    allOf:
      - {properties: {a: {type: boolean}}}
      - {properties: {b: {oneOf: [{type: integer}, {type: string}]}}}
"##;
        assert_eq!(example_of(yaml), json!({"a": true, "b": 0}));
    }

    #[test]
    fn recursive_schemas_stop() {
        let yaml = r##"
media: {schema: {$ref: '#/s/Node'}}
s:
  Node:
    properties:
      name: {type: string}
      children: {type: array, items: {$ref: '#/s/Node'}}
"##;
        assert_eq!(example_of(yaml), json!({"name": "string", "children": []}));
    }
}
