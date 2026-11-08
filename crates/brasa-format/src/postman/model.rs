//! Raw serde models of the Postman Collection v2.1 JSON shape (v2.0 files parse the same, since
//! this only reads the fields we map).
//!
//! These types exist purely to be deserialized with `serde_json`, nothing else. The mapping into
//! [`crate::postman::ImportPlan`] and friends happens in `collection.rs` and `environment.rs`,
//! which never expose these raw types.

use serde::Deserialize;
use serde_json::Value;

/// The root of a `.postman_collection.json` export.
#[derive(Debug, Deserialize)]
pub(super) struct RawCollection {
    pub(super) info: RawInfo,
    #[serde(default)]
    pub(super) item: Vec<RawItem>,
    #[serde(default)]
    pub(super) auth: Option<RawAuth>,
    #[serde(default)]
    pub(super) variable: Vec<RawVariable>,
    /// Collection-level `prerequest`/`test` scripts. Never imported (only per-request
    /// scripts are), but their presence is reported as a warning.
    #[serde(default)]
    pub(super) event: Option<Vec<RawEvent>>,
}

/// The `info` object of a collection.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawInfo {
    #[serde(default)]
    pub(super) name: String,
    /// Identifies the collection across exports, to tell a later import of it.
    #[serde(default, rename = "_postman_id")]
    pub(super) postman_id: String,
    /// The URL of the schema, which names the format version.
    #[serde(default)]
    pub(super) schema: String,
}

/// One entry of an `item` array: either a folder (has its own `item` array) or a request (has a
/// `request` object). A malformed entry with neither is simply skipped by the mapping code.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawItem {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) item: Option<Vec<RawItem>>,
    #[serde(default)]
    pub(super) request: Option<RawRequest>,
    #[serde(default)]
    pub(super) event: Option<Vec<RawEvent>>,
    #[serde(default)]
    pub(super) auth: Option<RawAuth>,
    /// The saved examples of a request.
    #[serde(default)]
    pub(super) response: Vec<RawSavedResponse>,
}

/// A saved example of a request.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawSavedResponse {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) code: Option<u32>,
    #[serde(default)]
    pub(super) body: Option<String>,
}

/// The `request` object of a request item.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawRequest {
    #[serde(default)]
    pub(super) method: Option<String>,
    #[serde(default)]
    pub(super) header: Option<Vec<RawHeader>>,
    #[serde(default)]
    pub(super) url: Option<RawUrl>,
    #[serde(default)]
    pub(super) body: Option<RawBody>,
    #[serde(default)]
    pub(super) auth: Option<RawAuth>,
    #[serde(default)]
    pub(super) description: Option<RawDescription>,
}

/// A single request header.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawHeader {
    pub(super) key: String,
    #[serde(default)]
    pub(super) value: Option<Value>,
    #[serde(default)]
    pub(super) disabled: bool,
}

/// A `url`, either the plain string form used by some exports, or the detailed object form
/// (`{ "raw": ..., "query": [...] }`) that Postman itself writes.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawUrl {
    /// The whole URL as a single string, query string included.
    Raw(String),
    /// The structured form, with `raw` and a separate, already split `query` array.
    Detailed(RawUrlDetailed),
}

/// The detailed object form of [`RawUrl`].
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawUrlDetailed {
    #[serde(default)]
    pub(super) raw: Option<String>,
    #[serde(default)]
    pub(super) query: Vec<RawQueryParam>,
}

/// One entry of a `url.query` array.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawQueryParam {
    #[serde(default)]
    pub(super) key: Option<String>,
    #[serde(default)]
    pub(super) value: Option<Value>,
    #[serde(default)]
    pub(super) disabled: bool,
}

/// The `body` object of a request.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawBody {
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default)]
    pub(super) raw: Option<String>,
    #[serde(default)]
    pub(super) options: Option<RawBodyOptions>,
    #[serde(default)]
    pub(super) urlencoded: Vec<RawFormParam>,
    #[serde(default)]
    pub(super) formdata: Vec<RawFormDataParam>,
}

/// The `body.options` object, we only need the raw body language.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawBodyOptions {
    #[serde(default)]
    pub(super) raw: Option<RawRawOptions>,
}

/// The `body.options.raw` object.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawRawOptions {
    #[serde(default)]
    pub(super) language: Option<String>,
}

/// One entry of a `body.urlencoded` array.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawFormParam {
    pub(super) key: String,
    #[serde(default)]
    pub(super) value: Option<Value>,
    #[serde(default)]
    pub(super) disabled: bool,
}

/// A field of a `formdata` body: a text field, or a file field with its path in `src`.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawFormDataParam {
    pub(super) key: String,
    #[serde(default)]
    pub(super) value: Option<Value>,
    /// `text` or `file`.
    #[serde(default, rename = "type")]
    pub(super) kind: Option<String>,
    /// The path of a file field: a string, or an array when several files were picked.
    #[serde(default)]
    pub(super) src: Option<Value>,
    #[serde(default)]
    pub(super) disabled: bool,
}

/// An `auth` object, at the collection, folder or request level.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawAuth {
    #[serde(default, rename = "type")]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) bearer: Vec<RawAuthParam>,
    #[serde(default)]
    pub(super) basic: Vec<RawAuthParam>,
    #[serde(default)]
    pub(super) apikey: Vec<RawAuthParam>,
    #[serde(default)]
    pub(super) oauth2: Vec<RawAuthParam>,
}

/// One `key`/`value` entry of an auth block (for example `{"key": "token", "value": "..."}`).
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawAuthParam {
    pub(super) key: String,
    #[serde(default)]
    pub(super) value: Option<Value>,
}

/// One entry of an `event` array (a `prerequest` or `test` script).
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawEvent {
    #[serde(default)]
    pub(super) listen: Option<String>,
    #[serde(default)]
    pub(super) script: Option<RawScript>,
}

/// The `script` object of an event.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawScript {
    #[serde(default)]
    pub(super) exec: Option<RawExec>,
}

/// The `script.exec` field, either a single string or an array of lines. Postman's own exports
/// always use the array form, but the string form is accepted too, for robustness.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawExec {
    /// One array entry per source line.
    Lines(Vec<String>),
    /// The whole script as one string, possibly with embedded newlines.
    Single(String),
}

impl RawExec {
    /// Splits this script into individual source lines, regardless of which form it was in.
    pub(super) fn lines(&self) -> Vec<String> {
        match self {
            RawExec::Lines(lines) => lines.clone(),
            RawExec::Single(text) => text.lines().map(str::to_string).collect(),
        }
    }
}

/// A `description`, either a plain string or the richer `{ "content": ..., "type": ... }` object
/// form. Only the text content is mapped.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawDescription {
    /// A plain text description.
    Text(String),
    /// The object form, we only read its `content`.
    Rich {
        /// The description text.
        #[serde(default)]
        content: Option<String>,
    },
}

impl RawDescription {
    /// The description text, regardless of which form it was in.
    pub(super) fn as_text(&self) -> String {
        match self {
            RawDescription::Text(text) => text.clone(),
            RawDescription::Rich { content } => content.clone().unwrap_or_default(),
        }
    }
}

/// One entry of the collection-level `variable` array.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawVariable {
    #[serde(default)]
    pub(super) key: Option<String>,
    #[serde(default)]
    pub(super) value: Option<Value>,
    #[serde(default, rename = "type")]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) disabled: bool,
}

/// The root of a `*.postman_environment.json` export.
#[derive(Debug, Deserialize, Default)]
pub(super) struct RawEnvironment {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) values: Vec<RawEnvironmentValue>,
}

/// One entry of an environment export's `values` array.
#[derive(Debug, Deserialize)]
pub(super) struct RawEnvironmentValue {
    pub(super) key: String,
    #[serde(default)]
    pub(super) value: Option<Value>,
    #[serde(default, rename = "type")]
    pub(super) kind: Option<String>,
    #[serde(default = "default_true")]
    pub(super) enabled: bool,
}

/// The default for [`RawEnvironmentValue::enabled`] when the field is absent: Postman treats a
/// value with no `enabled` field as enabled.
fn default_true() -> bool {
    true
}
