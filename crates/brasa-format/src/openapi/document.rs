//! The spec documents of one import and the `$ref` resolution between them.
//!
//! A spec is read into a [`serde_json::Value`] tree (YAML included), then every external `$ref`
//! (`other.yaml#/components/schemas/X`) is loaded up front through the caller's [`RefLoader`], so
//! that walking the spec afterwards is plain immutable navigation over [`Node`]s. A `$ref` that
//! cannot be followed becomes a warning and the referencing value is treated as missing.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::Value;

use super::error::OpenApiError;

/// The most documents one import loads, the main spec included. A guard against a spec whose
/// external references never end.
const MAX_DOCUMENTS: usize = 64;

/// The most `$ref` hops followed to resolve one value, a guard against reference cycles.
const MAX_REF_HOPS: usize = 32;

/// Loads the documents an OpenAPI spec references with an external `$ref`. Implemented by the
/// caller, which knows whether the spec came from a file, a URL or pasted text: this crate never
/// does IO itself.
pub trait RefLoader {
    /// Loads `reference` (the part of a `$ref` before `#`, a relative or absolute path or URL),
    /// relative to `base`, the location of the document the `$ref` is in. Returns the canonical
    /// location of the loaded document (used as `base` for its own references, and to load each
    /// document once) and its text, or a message saying why it could not be loaded.
    fn load(&self, base: &str, reference: &str) -> Result<(String, String), String>;
}

/// A [`RefLoader`] for pasted text, which has no location to resolve references against: every
/// external reference fails with a warning.
pub struct NoExternalRefs;

impl RefLoader for NoExternalRefs {
    fn load(&self, _base: &str, reference: &str) -> Result<(String, String), String> {
        Err(format!("cannot load {reference}, the spec has no location"))
    }
}

/// Parses spec text as JSON when it looks like JSON, and as YAML otherwise. YAML merge keys
/// (`<<: *anchor`) are applied.
pub(crate) fn parse_text(text: &str) -> Result<Value, OpenApiError> {
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed)
            .map_err(|error| OpenApiError::Syntax(error.to_string()));
    }
    let mut yaml: serde_norway::Value =
        serde_norway::from_str(trimmed).map_err(|error| OpenApiError::Syntax(error.to_string()))?;
    yaml.apply_merge()
        .map_err(|error| OpenApiError::Syntax(error.to_string()))?;
    serde_json::to_value(yaml).map_err(|error| OpenApiError::Syntax(error.to_string()))
}

/// Every document of one import, by canonical location, plus how each external reference maps
/// to one of them.
pub(crate) struct Documents {
    /// The location of the main spec document.
    pub(crate) main: String,
    documents: HashMap<String, Value>,
    /// `(base, reference)` to the canonical location the loader returned for it.
    links: HashMap<(String, String), String>,
    /// Problems found while resolving, deduplicated, in the order found.
    warnings: RefCell<Vec<String>>,
}

impl Documents {
    /// Takes the parsed main document at `location` and loads every document it references,
    /// directly or indirectly, through `loader`. A document that fails to load or parse is a
    /// warning, not an error.
    pub(crate) fn load(main: Value, location: &str, loader: &dyn RefLoader) -> Self {
        let mut documents = Documents {
            main: location.to_string(),
            documents: HashMap::new(),
            links: HashMap::new(),
            warnings: RefCell::new(Vec::new()),
        };
        let mut pending = vec![location.to_string()];
        documents.documents.insert(location.to_string(), main);
        while let Some(base) = pending.pop() {
            let mut references = Vec::new();
            if let Some(value) = documents.documents.get(&base) {
                collect_external_refs(value, &mut references);
            }
            for reference in references {
                let key = (base.clone(), reference.clone());
                if documents.links.contains_key(&key) {
                    continue;
                }
                if documents.documents.len() >= MAX_DOCUMENTS {
                    documents.warn(format!(
                        "too many referenced documents, skipped {reference}"
                    ));
                    continue;
                }
                match loader.load(&base, &reference) {
                    Ok((canonical, text)) => {
                        if !documents.documents.contains_key(&canonical) {
                            match parse_text(&text) {
                                Ok(value) => {
                                    documents.documents.insert(canonical.clone(), value);
                                    pending.push(canonical.clone());
                                }
                                Err(error) => {
                                    documents.warn(format!("cannot read {reference}: {error}"));
                                    continue;
                                }
                            }
                        }
                        documents.links.insert(key, canonical);
                    }
                    Err(error) => documents.warn(format!("cannot load {reference}: {error}")),
                }
            }
        }
        documents
    }

    /// The root node of the main document.
    pub(crate) fn root(&self) -> Node<'_> {
        Node {
            documents: self,
            doc: &self.main,
            value: self.documents.get(&self.main).unwrap_or(&Value::Null),
        }
    }

    /// Records a warning once.
    pub(crate) fn warn(&self, message: String) {
        let mut warnings = self.warnings.borrow_mut();
        if !warnings.contains(&message) {
            warnings.push(message);
        }
    }

    /// Takes the warnings collected so far.
    pub(crate) fn take_warnings(&self) -> Vec<String> {
        std::mem::take(&mut self.warnings.borrow_mut())
    }

    /// Follows one `$ref` written in document `doc`.
    fn follow<'a>(&'a self, doc: &'a str, reference: &str) -> Option<Node<'a>> {
        let (document, fragment) = reference.split_once('#').unwrap_or((reference, ""));
        let target: &str = if document.is_empty() {
            doc
        } else {
            self.links
                .get(&(doc.to_string(), document.to_string()))
                .map(String::as_str)?
        };
        let (target, root) = self.documents.get_key_value(target)?;
        let value = pointer(root, fragment)?;
        Some(Node {
            documents: self,
            doc: target,
            value,
        })
    }
}

/// Collects every external `$ref` document part (the text before `#`) found in `value`.
fn collect_external_refs(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("$ref") {
                let document = reference.split('#').next().unwrap_or_default();
                if !document.is_empty() && !out.iter().any(|known| known == document) {
                    out.push(document.to_string());
                }
            }
            map.values()
                .for_each(|child| collect_external_refs(child, out));
        }
        Value::Array(items) => items
            .iter()
            .for_each(|child| collect_external_refs(child, out)),
        _ => {}
    }
}

/// Resolves a JSON pointer fragment (`/components/schemas/Pet`), percent-decoded and with the
/// `~1` and `~0` escapes, inside `root`. An empty fragment is the whole document.
fn pointer<'a>(root: &'a Value, fragment: &str) -> Option<&'a Value> {
    let fragment = percent_decode(fragment);
    if fragment.is_empty() {
        return Some(root);
    }
    let mut current = root;
    for token in fragment.strip_prefix('/')?.split('/') {
        let token = token.replace("~1", "/").replace("~0", "~");
        current = match current {
            Value::Object(map) => map.get(&token)?,
            Value::Array(items) => items.get(token.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Decodes `%XX` escapes, leaving anything malformed as written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(hex) = text.get(index + 1..index + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A value inside one of the import's documents. Navigating with [`Node::get`] or iterating
/// follows `$ref`s transparently, so callers never see one.
#[derive(Clone, Copy)]
pub(crate) struct Node<'a> {
    documents: &'a Documents,
    doc: &'a str,
    value: &'a Value,
}

impl<'a> Node<'a> {
    /// This node with its `$ref` chain followed. A reference that cannot be followed is a
    /// warning, and the result is `None`.
    pub(crate) fn resolve(self) -> Option<Node<'a>> {
        let mut current = self;
        for _ in 0..MAX_REF_HOPS {
            let Some(Value::String(reference)) = current.value.get("$ref") else {
                return Some(current);
            };
            match self.documents.follow(current.doc, reference) {
                Some(next) => current = next,
                None => {
                    self.documents
                        .warn(format!("unresolved reference {reference}"));
                    return None;
                }
            }
        }
        self.documents.warn("reference cycle".to_string());
        None
    }

    /// The raw JSON value of this node, without following a `$ref` on it.
    pub(crate) fn value(&self) -> &'a Value {
        self.value
    }

    /// The `$ref` written on this node, if any.
    pub(crate) fn reference(&self) -> Option<&'a str> {
        self.value.get("$ref").and_then(Value::as_str)
    }

    /// The member `key` of this object, with its references followed.
    pub(crate) fn get(&self, key: &str) -> Option<Node<'a>> {
        let value = self.value.get(key)?;
        self.child(value).resolve()
    }

    /// The member `key` as a string.
    pub(crate) fn str(&self, key: &str) -> Option<&'a str> {
        self.get(key)?.value.as_str()
    }

    /// The member `key` as a boolean, `false` when absent.
    pub(crate) fn flag(&self, key: &str) -> bool {
        self.get(key)
            .and_then(|node| node.value.as_bool())
            .unwrap_or(false)
    }

    /// The `(key, node)` entries of this object, in document order, with references followed.
    /// Entries whose reference cannot be followed are skipped.
    pub(crate) fn entries(&self) -> Vec<(&'a str, Node<'a>)> {
        match self.value {
            Value::Object(map) => map
                .iter()
                .filter_map(|(key, value)| Some((key.as_str(), self.child(value).resolve()?)))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The items of this array, with references followed.
    pub(crate) fn items(&self) -> Vec<Node<'a>> {
        match self.value {
            Value::Array(items) => items
                .iter()
                .filter_map(|value| self.child(value).resolve())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The items of this array without following references, for the few places that must
    /// look at the `$ref` itself (schema cycles).
    pub(crate) fn raw_items(&self) -> Vec<Node<'a>> {
        match self.value {
            Value::Array(items) => items.iter().map(|value| self.child(value)).collect(),
            _ => Vec::new(),
        }
    }

    /// The member `key` without following a reference on it.
    pub(crate) fn raw(&self, key: &str) -> Option<Node<'a>> {
        Some(self.child(self.value.get(key)?))
    }

    /// The `(key, node)` entries of this object without following references.
    pub(crate) fn raw_entries(&self) -> Vec<(&'a str, Node<'a>)> {
        match self.value {
            Value::Object(map) => map
                .iter()
                .map(|(key, value)| (key.as_str(), self.child(value)))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Records a warning on the import.
    pub(crate) fn warn(&self, message: String) {
        self.documents.warn(message);
    }

    fn child(&self, value: &'a Value) -> Node<'a> {
        Node {
            documents: self.documents,
            doc: self.doc,
            value,
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    struct MapLoader(Vec<(&'static str, &'static str)>);

    impl RefLoader for MapLoader {
        fn load(&self, _base: &str, reference: &str) -> Result<(String, String), String> {
            self.0
                .iter()
                .find(|(name, _)| *name == reference)
                .map(|(name, text)| (name.to_string(), text.to_string()))
                .ok_or_else(|| "not found".to_string())
        }
    }

    #[test]
    fn yaml_integer_keys_become_strings_and_merges_apply() {
        let value = parse_text(
            "responses:\n  200: {description: ok}\nb: &b {x: 1}\nc:\n  <<: *b\n  y: 2\n",
        )
        .expect("valid YAML");
        assert_eq!(value["responses"]["200"]["description"], "ok");
        assert_eq!(value["c"]["x"], 1);
        assert_eq!(value["c"]["y"], 2);
    }

    #[test]
    fn json_text_parses_as_json() {
        let value = parse_text("\u{feff}  {\"openapi\": \"3.1.0\"}").expect("valid JSON");
        assert_eq!(value["openapi"], "3.1.0");
    }

    #[test]
    fn syntax_errors_are_reported() {
        assert!(matches!(
            parse_text("a: [1, 2"),
            Err(OpenApiError::Syntax(_))
        ));
    }

    #[test]
    fn local_and_external_references_resolve() {
        let main = parse_text(
            "a: {$ref: '#/defs/b'}\ndefs:\n  b: {$ref: 'other.yaml#/c~1d'}\nmissing: {$ref: '#/nope'}\n",
        )
        .expect("valid");
        let loader = MapLoader(vec![(
            "other.yaml",
            "c/d: {value: 42, next: {$ref: '#/e'}}\ne: 7\n",
        )]);
        let documents = Documents::load(main, "main.yaml", &loader);
        let root = documents.root();
        let a = root.get("a").expect("resolves");
        assert_eq!(a.value()["value"], 42);
        // A reference inside the external document is relative to that document.
        assert_eq!(
            a.get("next").expect("resolves").value(),
            &serde_json::json!(7)
        );
        assert!(root.get("missing").is_none());
        assert_eq!(
            documents.take_warnings(),
            vec!["unresolved reference #/nope".to_string()]
        );
    }

    #[test]
    fn reference_cycles_end_with_a_warning() {
        let main = parse_text("a: {$ref: '#/b'}\nb: {$ref: '#/a'}\n").expect("valid");
        let documents = Documents::load(main, "main", &NoExternalRefs);
        assert!(documents.root().get("a").is_none());
        assert_eq!(
            documents.take_warnings(),
            vec!["reference cycle".to_string()]
        );
    }

    #[test]
    fn external_references_without_a_loader_are_warnings() {
        let main = parse_text("a: {$ref: 'x.yaml#/y'}\n").expect("valid");
        let documents = Documents::load(main, "", &NoExternalRefs);
        assert!(documents.root().get("a").is_none());
        let warnings = documents.take_warnings();
        assert!(warnings[0].starts_with("cannot load x.yaml"));
    }

    #[test]
    fn percent_escapes_in_pointers_decode() {
        let main = parse_text("paths:\n  /a/{id}: {x: 1}\nr: {$ref: '#/paths/~1a~1%7Bid%7D'}\n")
            .expect("valid");
        let documents = Documents::load(main, "m", &NoExternalRefs);
        assert_eq!(documents.root().get("r").expect("resolves").value()["x"], 1);
    }
}
