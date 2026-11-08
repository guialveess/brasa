//! The Console tab of the response (#194, design 1a of "Console & Workspace History"): the
//! timeline of the last send, in order. Script log lines, the network exchange and the errors
//! that stopped a stage.
//!
//! [`ConsoleTimeline::build`] runs once per response, on the background thread that ran the
//! request, so nothing here costs anything in render. The network entry's detail (an ordered tree
//! and the raw HTTP text) is built there too. [`ConsoleView`] holds what the user changes (the
//! filter, the open tree nodes) and rebuilds the visible tree rows only when that changes.
//!
//! Kept free of `gpui` types so every rule is unit tested directly.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use brasa_core::{ConsoleLevel, Method, ResolvedBody, ResolvedRequest, Response, ScriptStage};
use brasa_runner::{FailedStage, RunResult};
use brasa_script::ScriptError;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

use super::response_render::reason_phrase;

/// Bodies larger than this are not parsed into the tree nor copied whole into the raw text: the
/// Body tab already shows them, and a tree of a 15 MB JSON would cost far more than it helps.
pub const MAX_DETAIL_BODY_BYTES: usize = 128 * 1024;

/// The most characters of a string a tree row shows. The row truncates to its width anyway, this
/// only keeps a 128 KB text body from being laid out whole on every frame.
const MAX_LEAF_CHARS: usize = 500;

/// Where an entry of the timeline came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySource {
    /// The `::: pre` script.
    Pre,
    /// The `::: post` script.
    Post,
    /// The HTTP exchange itself.
    Network,
}

impl From<ScriptStage> for EntrySource {
    fn from(stage: ScriptStage) -> Self {
        match stage {
            ScriptStage::Pre => EntrySource::Pre,
            ScriptStage::Post => EntrySource::Post,
        }
    }
}

/// One row of the Console tab.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsoleEntry {
    /// A `console.*` call.
    Log {
        time: Option<SystemTime>,
        source: EntrySource,
        level: ConsoleLevel,
        text: String,
        line: Option<u32>,
    },
    /// The request that was sent, and its response when one arrived.
    Network {
        time: Option<SystemTime>,
        method: Method,
        url: String,
        status: Option<u16>,
        duration: Option<Duration>,
        size: Option<usize>,
    },
    /// The error that stopped a stage: a script exception, a script limit or a failed send.
    Error {
        time: Option<SystemTime>,
        source: EntrySource,
        message: String,
        line: Option<u32>,
    },
}

impl ConsoleEntry {
    /// Whether the entry counts as an error: an [`ConsoleEntry::Error`] or a `console.error`.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            ConsoleEntry::Error { .. }
                | ConsoleEntry::Log {
                    level: ConsoleLevel::Error,
                    ..
                }
        )
    }

    /// The text the filter field searches.
    fn search_text(&self) -> &str {
        match self {
            ConsoleEntry::Log { text, .. } => text,
            ConsoleEntry::Network { url, .. } => url,
            ConsoleEntry::Error { message, .. } => message,
        }
    }
}

/// The chips of the Console's filter bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConsoleFilter {
    #[default]
    All,
    Network,
    Logs,
    Errors,
}

impl ConsoleFilter {
    /// Every chip, in the order the bar shows them.
    pub const ALL: [ConsoleFilter; 4] = [
        ConsoleFilter::All,
        ConsoleFilter::Network,
        ConsoleFilter::Logs,
        ConsoleFilter::Errors,
    ];

    /// Whether `entry` shows under this chip.
    pub fn accepts(self, entry: &ConsoleEntry) -> bool {
        match self {
            ConsoleFilter::All => true,
            ConsoleFilter::Network => matches!(entry, ConsoleEntry::Network { .. }),
            ConsoleFilter::Logs => matches!(entry, ConsoleEntry::Log { .. }),
            ConsoleFilter::Errors => entry.is_error(),
        }
    }
}

/// A JSON-like value with its keys in their original order, for the network entry's tree.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<TreeValue>),
    Object(Vec<(String, TreeValue)>),
}

impl TreeValue {
    fn object(entries: Vec<(&str, TreeValue)>) -> Self {
        TreeValue::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    /// The children of a non-empty object or array. Empty ones are leaves (`{}`, `[]`).
    fn children(&self) -> Option<Vec<(String, &TreeValue)>> {
        match self {
            TreeValue::Array(items) if items.is_empty() => None,
            TreeValue::Object(entries) if entries.is_empty() => None,
            TreeValue::Array(items) => Some(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| (index.to_string(), item))
                    .collect(),
            ),
            TreeValue::Object(entries) => Some(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), value))
                    .collect(),
            ),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for TreeValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TreeVisitor;

        impl<'de> Visitor<'de> for TreeVisitor {
            type Value = TreeValue;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("any JSON value")
            }
            fn visit_unit<E>(self) -> Result<TreeValue, E> {
                Ok(TreeValue::Null)
            }
            fn visit_bool<E>(self, value: bool) -> Result<TreeValue, E> {
                Ok(TreeValue::Bool(value))
            }
            fn visit_i64<E>(self, value: i64) -> Result<TreeValue, E> {
                Ok(TreeValue::Number(value.to_string()))
            }
            fn visit_u64<E>(self, value: u64) -> Result<TreeValue, E> {
                Ok(TreeValue::Number(value.to_string()))
            }
            fn visit_f64<E>(self, value: f64) -> Result<TreeValue, E> {
                Ok(TreeValue::Number(value.to_string()))
            }
            fn visit_str<E>(self, value: &str) -> Result<TreeValue, E> {
                Ok(TreeValue::String(value.to_string()))
            }
            fn visit_string<E>(self, value: String) -> Result<TreeValue, E> {
                Ok(TreeValue::String(value))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<TreeValue, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(TreeValue::Array(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<TreeValue, A::Error> {
                let mut entries = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, TreeValue>()? {
                    entries.push((key, value));
                }
                Ok(TreeValue::Object(entries))
            }
        }

        deserializer.deserialize_any(TreeVisitor)
    }
}

/// How a tree row's value is colored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueTone {
    String,
    Number,
    /// `true`, `false` and `null`.
    Constant,
    /// The summary of a branch (`{ ... }`, `Array(3)`).
    Summary,
}

/// One visible row of the network entry's tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    /// The dotted path of the node, its identity for opening and closing it.
    pub path: String,
    pub key: String,
    /// 0 for the top level.
    pub depth: usize,
    /// `None` for a leaf; whether the branch is open otherwise.
    pub open: Option<bool>,
    pub text: String,
    pub tone: ValueTone,
}

/// The detail of the network entry: the tree and the raw HTTP text of both sides.
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkDetail {
    pub tree: TreeValue,
    pub raw_request: Arc<str>,
    pub raw_response: Option<Arc<str>>,
}

/// Everything the Console tab shows for one response, built once.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleTimeline {
    pub entries: Vec<ConsoleEntry>,
    pub error_count: usize,
    pub network: Option<NetworkDetail>,
}

impl ConsoleTimeline {
    /// Builds the timeline of `result`. `environment` is the name of the environment it ran
    /// with, if any.
    pub fn build(result: &RunResult, environment: Option<&str>) -> Self {
        let mut entries = Vec::new();
        let logs = |stage: ScriptStage| {
            result
                .console
                .iter()
                .filter(move |line| line.stage == stage)
                .map(|line| ConsoleEntry::Log {
                    time: line.time,
                    source: line.stage.into(),
                    level: line.level,
                    text: line.text.clone(),
                    line: line.line,
                })
        };
        let failure =
            |source: EntrySource, message: String, line: Option<u32>| ConsoleEntry::Error {
                time: Some(result.finished_at),
                source,
                message,
                line,
            };

        entries.extend(logs(ScriptStage::Pre));
        if let Some(FailedStage::Pre(error)) = &result.failed_stage {
            let (message, line) = script_error_parts(error);
            entries.push(failure(EntrySource::Pre, message, line));
        }
        if let Some(request) = &result.resolved_request
            && result.sent_at.is_some()
        {
            let response = result.response.as_ref();
            entries.push(ConsoleEntry::Network {
                time: result.sent_at,
                method: request.method.clone(),
                url: request.url.clone(),
                status: response.map(|response| response.status),
                duration: response.map(|response| response.time),
                size: response.map(|response| response.size),
            });
        }
        if let Some(FailedStage::Send(error)) = &result.failed_stage {
            entries.push(failure(EntrySource::Network, error.to_string(), None));
        }
        entries.extend(logs(ScriptStage::Post));
        if let Some(FailedStage::Post(error)) = &result.failed_stage {
            let (message, line) = script_error_parts(error);
            entries.push(failure(EntrySource::Post, message, line));
        }

        let network = result
            .resolved_request
            .as_ref()
            .filter(|_| result.sent_at.is_some())
            .map(|request| NetworkDetail {
                tree: network_tree(request, result.response.as_ref(), environment),
                raw_request: Arc::from(raw_request(request)),
                raw_response: result
                    .response
                    .as_ref()
                    .map(|response| Arc::from(raw_response(response))),
            });
        let error_count = entries.iter().filter(|entry| entry.is_error()).count();
        ConsoleTimeline {
            entries,
            error_count,
            network,
        }
    }
}

/// The message and script line of a script error. A runtime error's line comes from its stack.
fn script_error_parts(error: &ScriptError) -> (String, Option<u32>) {
    match error {
        ScriptError::Syntax { message, line } => (format!("SyntaxError: {message}"), *line),
        ScriptError::Runtime { message, stack } => (
            message.clone(),
            stack.as_deref().and_then(script_line_from_stack),
        ),
        other => (other.to_string(), None),
    }
}

/// The first `pre-script:N` or `post-script:N` line of a QuickJS stack trace.
fn script_line_from_stack(stack: &str) -> Option<u32> {
    ["pre-script:", "post-script:"]
        .iter()
        .filter_map(|marker| {
            let start = stack.find(marker)? + marker.len();
            let digits: String = stack[start..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            Some((start, digits.parse().ok()?))
        })
        .min_by_key(|(start, _)| *start)
        .map(|(_, line)| line)
}

/// A body as a tree value: the parsed JSON when it is JSON, its text otherwise, a size note when
/// it is too big or not text, and `null` when there is none.
fn body_value(body: &[u8]) -> TreeValue {
    if body.is_empty() {
        return TreeValue::Null;
    }
    if body.len() > MAX_DETAIL_BODY_BYTES {
        return TreeValue::String(format!("({} bytes, see the Body tab)", body.len()));
    }
    if let Ok(value) = serde_json::from_slice::<TreeValue>(body) {
        return value;
    }
    match std::str::from_utf8(body) {
        Ok(text) => TreeValue::String(text.to_string()),
        Err(_) => TreeValue::String(format!("({} bytes of binary data)", body.len())),
    }
}

fn headers_value<'a>(headers: impl Iterator<Item = (&'a str, &'a str)>) -> TreeValue {
    TreeValue::Object(
        headers
            .map(|(name, value)| (name.to_string(), TreeValue::String(value.to_string())))
            .collect(),
    )
}

/// The tree of the network entry: the request, the response, the timings and the environment.
fn network_tree(
    request: &ResolvedRequest,
    response: Option<&Response>,
    environment: Option<&str>,
) -> TreeValue {
    let request_body = match &request.body {
        ResolvedBody::None => TreeValue::Null,
        ResolvedBody::Bytes(bytes) => body_value(bytes),
    };
    let request_value = TreeValue::object(vec![
        ("method", TreeValue::String(request.method.to_string())),
        ("url", TreeValue::String(request.url.clone())),
        (
            "headers",
            headers_value(
                request
                    .headers
                    .iter()
                    .map(|field| (field.name.as_str(), field.value.as_str())),
            ),
        ),
        ("body", request_body),
    ]);
    let mut top = vec![("request", request_value)];
    if let Some(response) = response {
        let mut fields = vec![("status", TreeValue::Number(response.status.to_string()))];
        if let Some(phrase) = reason_phrase(response.status) {
            fields.push(("statusText", TreeValue::String(phrase.to_string())));
        }
        fields.push((
            "headers",
            headers_value(
                response
                    .headers
                    .iter()
                    .map(|field| (field.name.as_str(), field.value.as_str())),
            ),
        ));
        fields.push(("body", body_value(&response.body)));
        top.push(("response", TreeValue::object(fields)));
        top.push((
            "timings",
            TreeValue::object(vec![(
                "total",
                TreeValue::Number(response.time.as_millis().to_string()),
            )]),
        ));
    }
    top.push((
        "environment",
        environment.map_or(TreeValue::Null, |name| TreeValue::String(name.to_string())),
    ));
    TreeValue::object(top)
}

/// The request as HTTP/1.1 text: request line, `Host`, headers and body. `ureq` only speaks
/// HTTP/1.1, so the version is the one used on the wire. Headers the client adds by itself are
/// not known here and not shown.
fn raw_request(request: &ResolvedRequest) -> String {
    let (host, target) = split_url(&request.url);
    let mut text = format!("{} {target} HTTP/1.1\n", request.method);
    let has_host = request
        .headers
        .iter()
        .any(|field| field.name.eq_ignore_ascii_case("host"));
    if !has_host && !host.is_empty() {
        text.push_str(&format!("Host: {host}\n"));
    }
    for field in &request.headers {
        text.push_str(&format!("{}: {}\n", field.name, field.value));
    }
    if let ResolvedBody::Bytes(bytes) = &request.body
        && !bytes.is_empty()
    {
        text.push('\n');
        text.push_str(&raw_body(bytes));
    }
    text.trim_end().to_string()
}

/// The response as HTTP/1.1 text: status line, headers and body.
fn raw_response(response: &Response) -> String {
    let mut text = format!("HTTP/1.1 {}", response.status);
    if let Some(phrase) = reason_phrase(response.status) {
        text.push(' ');
        text.push_str(phrase);
    }
    text.push('\n');
    for field in &response.headers {
        text.push_str(&format!("{}: {}\n", field.name, field.value));
    }
    if !response.body.is_empty() {
        text.push('\n');
        text.push_str(&raw_body(&response.body));
    }
    text.trim_end().to_string()
}

/// A body for the raw text, cut at [`MAX_DETAIL_BODY_BYTES`].
fn raw_body(body: &[u8]) -> String {
    let cut = &body[..body.len().min(MAX_DETAIL_BODY_BYTES)];
    let mut text = String::from_utf8_lossy(cut).into_owned();
    if body.len() > MAX_DETAIL_BODY_BYTES {
        text.push_str(&format!(
            "\n... ({} more bytes, see the Body tab)",
            body.len() - MAX_DETAIL_BODY_BYTES
        ));
    }
    text
}

/// Splits a URL into its host (with port) and its request target (path and query).
fn split_url(url: &str) -> (&str, &str) {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let (authority, target) = match rest.find(['/', '?']) {
        Some(index) => rest.split_at(index),
        None => (rest, ""),
    };
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let target = if target.is_empty() {
        "/"
    } else if target.starts_with('?') {
        // A query without a path still asks for `/`.
        return (host, target);
    } else {
        target
    };
    (host, target)
}

/// The summary of a closed branch: `Array(3)` or `{ a: 1, b: "x", c: {…}, … }`.
fn summary(value: &TreeValue) -> String {
    match value {
        TreeValue::Array(items) if items.is_empty() => "[]".to_string(),
        TreeValue::Object(entries) if entries.is_empty() => "{}".to_string(),
        TreeValue::Array(items) => format!("Array({})", items.len()),
        TreeValue::Object(entries) => {
            let parts: Vec<String> = entries
                .iter()
                .take(3)
                .map(|(key, value)| format!("{key}: {}", short(value)))
                .collect();
            let more = if entries.len() > 3 { ", …" } else { "" };
            format!("{{ {}{more} }}", parts.join(", "))
        }
        leaf => leaf_text(leaf).0,
    }
}

/// A value inside a summary: nested branches as `[…]` and `{…}`.
fn short(value: &TreeValue) -> String {
    match value {
        TreeValue::Array(items) if items.is_empty() => "[]".to_string(),
        TreeValue::Object(entries) if entries.is_empty() => "{}".to_string(),
        TreeValue::Array(_) => "[…]".to_string(),
        TreeValue::Object(_) => "{…}".to_string(),
        leaf => leaf_text(leaf).0,
    }
}

fn leaf_text(value: &TreeValue) -> (String, ValueTone) {
    match value {
        TreeValue::Null => ("null".to_string(), ValueTone::Constant),
        TreeValue::Bool(value) => (value.to_string(), ValueTone::Constant),
        TreeValue::Number(number) => (number.clone(), ValueTone::Number),
        TreeValue::String(text) => (quoted(text), ValueTone::String),
        branch => (summary(branch), ValueTone::Summary),
    }
}

/// A string as a JSON literal, so line breaks and tabs show as `\n` and `\t` and a row stays
/// one line (a text body is usually several), cut at [`MAX_LEAF_CHARS`].
fn quoted(text: &str) -> String {
    let cut = text.char_indices().nth(MAX_LEAF_CHARS).map(|(at, _)| at);
    let shown = cut.map_or(text, |at| &text[..at]);
    let literal = serde_json::to_string(shown).unwrap_or_else(|_| format!("\"{shown}\""));
    match cut {
        Some(_) => format!("{}…\"", &literal[..literal.len() - 1]),
        None => literal,
    }
}

/// The visible rows of `tree`: the top level always, and the children of every open branch.
pub fn tree_rows(tree: &TreeValue, open: &HashSet<String>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    if let Some(children) = tree.children() {
        push_rows(&children, "", 0, open, &mut rows);
    }
    rows
}

fn push_rows(
    children: &[(String, &TreeValue)],
    parent: &str,
    depth: usize,
    open: &HashSet<String>,
    rows: &mut Vec<TreeRow>,
) {
    for (key, value) in children {
        let path = if parent.is_empty() {
            key.clone()
        } else {
            format!("{parent}.{key}")
        };
        match value.children() {
            Some(grandchildren) => {
                let is_open = open.contains(&path);
                let text = match (is_open, value) {
                    (true, TreeValue::Array(items)) => format!("[{}]", items.len()),
                    (true, _) => String::new(),
                    (false, _) => summary(value),
                };
                rows.push(TreeRow {
                    path: path.clone(),
                    key: key.clone(),
                    depth,
                    open: Some(is_open),
                    text,
                    tone: ValueTone::Summary,
                });
                if is_open {
                    push_rows(&grandchildren, &path, depth + 1, open, rows);
                }
            }
            None => {
                let (text, tone) = leaf_text(value);
                rows.push(TreeRow {
                    path,
                    key: key.clone(),
                    depth,
                    open: None,
                    text,
                    tone,
                });
            }
        }
    }
}

/// The path of every branch of `tree`, for "Expand all".
pub fn branch_paths(tree: &TreeValue) -> HashSet<String> {
    fn walk(value: &TreeValue, path: &str, out: &mut HashSet<String>) {
        if let Some(children) = value.children() {
            if !path.is_empty() {
                out.insert(path.to_string());
            }
            for (key, child) in children {
                let child_path = if path.is_empty() {
                    key
                } else {
                    format!("{path}.{key}")
                };
                walk(child, &child_path, out);
            }
        }
    }
    let mut out = HashSet::new();
    walk(tree, "", &mut out);
    out
}

/// The tree as pretty JSON, for "Copy".
pub fn tree_as_json(tree: &TreeValue) -> String {
    fn to_json(value: &TreeValue) -> serde_json::Value {
        match value {
            TreeValue::Null => serde_json::Value::Null,
            TreeValue::Bool(value) => serde_json::Value::Bool(*value),
            TreeValue::Number(number) => number
                .parse::<serde_json::Number>()
                .map(serde_json::Value::Number)
                .unwrap_or_else(|_| serde_json::Value::String(number.clone())),
            TreeValue::String(text) => serde_json::Value::String(text.clone()),
            TreeValue::Array(items) => {
                serde_json::Value::Array(items.iter().map(to_json).collect())
            }
            // serde_json's map sorts keys, so objects are written by hand below to keep order.
            TreeValue::Object(_) => serde_json::Value::Null,
        }
    }
    fn write(value: &TreeValue, indent: usize, out: &mut String) {
        let pad = "  ".repeat(indent + 1);
        match value {
            TreeValue::Object(entries) if !entries.is_empty() => {
                out.push_str("{\n");
                for (index, (key, child)) in entries.iter().enumerate() {
                    out.push_str(&pad);
                    out.push_str(&serde_json::Value::String(key.clone()).to_string());
                    out.push_str(": ");
                    write(child, indent + 1, out);
                    if index + 1 < entries.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(indent));
                out.push('}');
            }
            TreeValue::Object(_) => out.push_str("{}"),
            TreeValue::Array(items) if !items.is_empty() => {
                out.push_str("[\n");
                for (index, child) in items.iter().enumerate() {
                    out.push_str(&pad);
                    write(child, indent + 1, out);
                    if index + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(indent));
                out.push(']');
            }
            leaf => out.push_str(&to_json(leaf).to_string()),
        }
    }
    let mut out = String::new();
    write(tree, 0, &mut out);
    out
}

/// What a tree row's Copy writes (#197).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyPart {
    /// The value: a string as its text (unquoted, whole, with its line breaks), any other leaf
    /// as written, a branch as pretty JSON.
    Value,
    /// The key, such as `x-sample-profile`, or the index in an array.
    Key,
    /// The dotted path, such as `response.headers.x-sample-profile`.
    Path,
}

/// The node at `path` (as [`TreeRow::path`] names it) and its key.
fn find_node<'a>(tree: &'a TreeValue, path: &str) -> Option<(String, &'a TreeValue)> {
    fn walk<'a>(value: &'a TreeValue, parent: &str, path: &str) -> Option<(String, &'a TreeValue)> {
        for (key, child) in value.children()? {
            let child_path = if parent.is_empty() {
                key.clone()
            } else {
                format!("{parent}.{key}")
            };
            if child_path == path {
                return Some((key, child));
            }
            // Keys may contain dots (`a.b`), so more than one child can be a prefix.
            if path.starts_with(&format!("{child_path}."))
                && let Some(found) = walk(child, &child_path, path)
            {
                return Some(found);
            }
        }
        None
    }
    walk(tree, "", path)
}

/// The text Copy writes for the row at `path`, `None` when there is no such row.
pub fn copy_text(tree: &TreeValue, path: &str, part: CopyPart) -> Option<String> {
    let (key, value) = find_node(tree, path)?;
    Some(match part {
        CopyPart::Key => key,
        CopyPart::Path => path.to_string(),
        CopyPart::Value => match value {
            TreeValue::String(text) => text.clone(),
            TreeValue::Null => "null".to_string(),
            TreeValue::Bool(value) => value.to_string(),
            TreeValue::Number(number) => number.clone(),
            branch => tree_as_json(branch),
        },
    })
}

/// A key that moves the tree's selection (#197).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeKey {
    /// The row above.
    Up,
    /// The row below.
    Down,
    /// Closes an open branch, or goes to the parent.
    Left,
    /// Opens a closed branch, or goes to its first child.
    Right,
    /// The first row.
    Home,
    /// The last row.
    End,
}

/// A time as the local `HH:MM:SS.mmm` clock.
pub fn format_clock(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time)
        .format("%H:%M:%S%.3f")
        .to_string()
}

/// What the user changed in the Console of one response. Reset when a new response arrives.
#[derive(Debug, Default)]
pub struct ConsoleView {
    /// The `(tab id, response generation)` this state belongs to.
    key: Option<(String, u64)>,
    pub filter: ConsoleFilter,
    /// The text filter, lowercased.
    query: String,
    /// Whether "Clear" hid the entries of this response.
    pub cleared: bool,
    pub network_open: bool,
    pub raw: bool,
    open: HashSet<String>,
    /// The path of the selected tree row (#197), for Copy and the arrow keys. When a branch
    /// closes over it, its closest visible ancestor becomes selected.
    pub selected: Option<String>,
    /// The visible tree rows, rebuilt only when the open set or the response changes.
    rows: Arc<Vec<TreeRow>>,
    rows_built: bool,
}

impl ConsoleView {
    /// Starts over when `(tab_id, generation)` is a different response than the last one.
    /// The filter chip and the text filter are kept, as in a browser's console.
    pub fn sync(&mut self, tab_id: &str, generation: u64) {
        let same = self
            .key
            .as_ref()
            .is_some_and(|(tab, gen_)| tab == tab_id && *gen_ == generation);
        if !same {
            self.key = Some((tab_id.to_string(), generation));
            self.cleared = false;
            self.network_open = false;
            self.raw = false;
            self.open.clear();
            self.selected = None;
            self.rows = Arc::default();
            self.rows_built = false;
        }
    }

    /// Sets the text filter.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.trim().to_lowercase();
    }

    /// The entries of `timeline` that the chip, the text filter and "Clear" let through.
    pub fn visible<'a>(&self, timeline: &'a ConsoleTimeline) -> Vec<&'a ConsoleEntry> {
        if self.cleared {
            return Vec::new();
        }
        timeline
            .entries
            .iter()
            .filter(|entry| self.filter.accepts(entry))
            .filter(|entry| {
                self.query.is_empty() || entry.search_text().to_lowercase().contains(&self.query)
            })
            .collect()
    }

    /// The visible rows of the network tree.
    pub fn rows(&mut self, detail: &NetworkDetail) -> Arc<Vec<TreeRow>> {
        if !self.rows_built {
            self.rows = Arc::new(tree_rows(&detail.tree, &self.open));
            self.rows_built = true;
            self.selected = self
                .selected_index(&self.rows)
                .map(|index| self.rows[index].path.clone());
        }
        self.rows.clone()
    }

    /// Selects the row at `path`.
    pub fn select(&mut self, path: &str) {
        self.selected = Some(path.to_string());
    }

    /// The index in `rows` of the selected row, or of its closest visible ancestor.
    fn selected_index(&self, rows: &[TreeRow]) -> Option<usize> {
        let selected = self.selected.as_deref()?;
        rows.iter()
            .enumerate()
            .filter(|(_, row)| {
                selected == row.path
                    || selected
                        .strip_prefix(row.path.as_str())
                        .is_some_and(|rest| rest.starts_with('.'))
            })
            .max_by_key(|(_, row)| row.path.len())
            .map(|(index, _)| index)
    }

    /// Moves the selection with `key`, opening or closing branches for Left and Right. With
    /// nothing selected, any key selects the first row. Returns whether anything changed.
    pub fn navigate(&mut self, key: TreeKey, detail: &NetworkDetail) -> bool {
        let rows = self.rows(detail);
        let Some(last) = rows.len().checked_sub(1) else {
            return false;
        };
        let Some(current) = self.selected_index(&rows) else {
            self.select(&rows[0].path);
            return true;
        };
        let row = &rows[current];
        let target = match key {
            TreeKey::Up => current.saturating_sub(1),
            TreeKey::Down => (current + 1).min(last),
            TreeKey::Home => 0,
            TreeKey::End => last,
            TreeKey::Right => match row.open {
                Some(false) => {
                    self.toggle(&row.path);
                    return true;
                }
                // An open branch has at least one child, right below it.
                Some(true) => current + 1,
                None => current,
            },
            TreeKey::Left => match row.open {
                Some(true) => {
                    self.toggle(&row.path);
                    return true;
                }
                _ => match row.depth.checked_sub(1) {
                    Some(parent) => (0..current)
                        .rev()
                        .find(|&index| rows[index].depth == parent)
                        .unwrap_or(current),
                    None => current,
                },
            },
        };
        if target == current {
            return false;
        }
        self.select(&rows[target].path);
        true
    }

    /// Opens or closes the branch at `path`.
    pub fn toggle(&mut self, path: &str) {
        if !self.open.remove(path) {
            self.open.insert(path.to_string());
        }
        self.rows_built = false;
    }

    /// Opens every branch, and switches to the tree.
    pub fn expand_all(&mut self, detail: &NetworkDetail) {
        self.open = branch_paths(&detail.tree);
        self.raw = false;
        self.rows_built = false;
    }

    /// Closes every branch.
    pub fn collapse_all(&mut self) {
        self.open.clear();
        self.rows_built = false;
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use brasa_core::{ConsoleLine, ResolvedField};
    use brasa_runner::HttpError;
    use pretty_assertions::assert_eq;

    fn line(stage: ScriptStage, level: ConsoleLevel, text: &str) -> ConsoleLine {
        ConsoleLine {
            level,
            text: text.to_string(),
            stage,
            time: None,
            line: Some(2),
        }
    }

    fn field(name: &str, value: &str) -> ResolvedField {
        ResolvedField {
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    fn request() -> ResolvedRequest {
        ResolvedRequest {
            method: Method::Get,
            url: "https://api.acme.dev/v1/users?page=2".to_string(),
            headers: vec![field("Accept", "application/json")],
            body: ResolvedBody::None,
        }
    }

    fn response(body: &str) -> Response {
        Response {
            status: 200,
            headers: vec![field("Content-Type", "application/json")],
            body: body.as_bytes().to_vec(),
            time: Duration::from_millis(182),
            size: body.len(),
            tls_warning: None,
        }
    }

    fn result(failed_stage: Option<FailedStage>, sent: bool) -> RunResult {
        RunResult {
            resolved_request: Some(request()),
            response: sent.then(|| response(r#"{"page":2,"data":[{"id":21,"ok":true}]}"#)),
            tests: Vec::new(),
            console: vec![
                line(ScriptStage::Post, ConsoleLevel::Log, "users on page: 20"),
                line(ScriptStage::Pre, ConsoleLevel::Info, "using dev"),
            ],
            warnings: Vec::new(),
            failed_stage,
            started_at: SystemTime::UNIX_EPOCH,
            sent_at: Some(SystemTime::UNIX_EPOCH),
            finished_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn the_timeline_puts_pre_logs_the_network_post_logs_and_the_error_in_order() {
        let error = ScriptError::Runtime {
            message: "TypeError: cannot read 'email'".to_string(),
            stack: Some("    at <anonymous> (post-script:7:3)\n".to_string()),
        };
        let timeline = ConsoleTimeline::build(&result(Some(FailedStage::Post(error)), true), None);

        let kinds: Vec<&str> = timeline
            .entries
            .iter()
            .map(|entry| match entry {
                ConsoleEntry::Log { text, .. } => text.as_str(),
                ConsoleEntry::Network { .. } => "network",
                ConsoleEntry::Error { .. } => "error",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["using dev", "network", "users on page: 20", "error"]
        );
        assert_eq!(timeline.error_count, 1);
        let ConsoleEntry::Error { line, source, .. } = &timeline.entries[3] else {
            panic!("an error entry");
        };
        assert_eq!((*line, *source), (Some(7), EntrySource::Post));
    }

    #[test]
    fn a_failed_send_has_a_network_entry_without_status_and_an_error() {
        let failed = Some(FailedStage::Send(HttpError::Timeout));
        let timeline = ConsoleTimeline::build(&result(failed, false), None);
        assert!(matches!(
            timeline.entries[1],
            ConsoleEntry::Network { status: None, .. }
        ));
        assert!(matches!(
            timeline.entries[2],
            ConsoleEntry::Error {
                source: EntrySource::Network,
                ..
            }
        ));
        assert!(
            timeline
                .network
                .as_ref()
                .expect("detail")
                .raw_response
                .is_none()
        );
    }

    #[test]
    fn filters_pick_their_kind_and_the_query_matches_any_case() {
        let timeline = ConsoleTimeline::build(&result(None, true), None);
        let mut view = ConsoleView::default();
        view.sync("t", 1);
        assert_eq!(view.visible(&timeline).len(), 3);
        view.filter = ConsoleFilter::Network;
        assert_eq!(view.visible(&timeline).len(), 1);
        view.filter = ConsoleFilter::Logs;
        view.set_query("USERS");
        assert_eq!(view.visible(&timeline).len(), 1);
        view.filter = ConsoleFilter::Errors;
        assert!(view.visible(&timeline).is_empty());
        view.filter = ConsoleFilter::All;
        view.set_query("");
        view.cleared = true;
        assert!(view.visible(&timeline).is_empty());
        view.sync("t", 2);
        assert_eq!(view.visible(&timeline).len(), 3);
    }

    #[test]
    fn the_tree_starts_with_the_top_level_closed_and_keeps_json_key_order() {
        let timeline = ConsoleTimeline::build(&result(None, true), Some("dev"));
        let detail = timeline.network.expect("detail");
        let mut view = ConsoleView::default();
        view.sync("t", 1);

        let keys: Vec<String> = view
            .rows(&detail)
            .iter()
            .map(|row| row.key.clone())
            .collect();
        assert_eq!(keys, vec!["request", "response", "timings", "environment"]);
        assert_eq!(view.rows(&detail)[3].text, "\"dev\"");

        view.toggle("response");
        view.toggle("response.body");
        let rows = view.rows(&detail);
        let body: Vec<(String, usize, String)> = rows
            .iter()
            .filter(|row| row.path.starts_with("response.body."))
            .map(|row| (row.key.clone(), row.depth, row.text.clone()))
            .collect();
        assert_eq!(
            body,
            vec![
                ("page".to_string(), 2, "2".to_string()),
                ("data".to_string(), 2, "Array(1)".to_string()),
            ]
        );

        view.expand_all(&detail);
        assert!(
            view.rows(&detail)
                .iter()
                .any(|row| row.path == "response.body.data.0.ok")
        );
        view.collapse_all();
        assert_eq!(view.rows(&detail).len(), 4);
    }

    #[test]
    fn summaries_show_three_keys_and_shorten_nested_values() {
        let value: TreeValue =
            serde_json::from_str(r#"{"a":1,"b":"x","c":{"d":null},"e":[1]}"#).expect("json");
        assert_eq!(summary(&value), r#"{ a: 1, b: "x", c: {…}, … }"#);
    }

    #[test]
    fn empty_objects_and_arrays_are_leaves() {
        let value: TreeValue = serde_json::from_str(r#"{"h":{},"l":[]}"#).expect("json");
        let rows = tree_rows(&value, &HashSet::new());
        let shown: Vec<(&str, Option<bool>, &str)> = rows
            .iter()
            .map(|row| (row.key.as_str(), row.open, row.text.as_str()))
            .collect();
        assert_eq!(shown, vec![("h", None, "{}"), ("l", None, "[]")]);
        assert!(branch_paths(&value).is_empty());
    }

    #[test]
    fn raw_text_has_the_request_line_host_headers_and_bodies() {
        let timeline = ConsoleTimeline::build(&result(None, true), None);
        let detail = timeline.network.expect("detail");
        assert_eq!(
            &*detail.raw_request,
            "GET /v1/users?page=2 HTTP/1.1\nHost: api.acme.dev\nAccept: application/json"
        );
        assert_eq!(
            detail.raw_response.as_deref(),
            Some(
                "HTTP/1.1 200 OK\nContent-Type: application/json\n\n\
                 {\"page\":2,\"data\":[{\"id\":21,\"ok\":true}]}"
            )
        );
    }

    #[test]
    fn split_url_handles_credentials_ports_and_bare_hosts() {
        assert_eq!(
            split_url("http://u:p@localhost:8080/a?b=1"),
            ("localhost:8080", "/a?b=1")
        );
        assert_eq!(split_url("https://example.com"), ("example.com", "/"));
        assert_eq!(
            split_url("https://example.com?x=1"),
            ("example.com", "?x=1")
        );
    }

    #[test]
    fn big_bodies_are_left_to_the_body_tab() {
        let big = "x".repeat(MAX_DETAIL_BODY_BYTES + 1);
        assert_eq!(
            body_value(big.as_bytes()),
            TreeValue::String(format!("({} bytes, see the Body tab)", big.len()))
        );
    }

    #[test]
    fn copy_writes_the_tree_as_json_in_order() {
        let value: TreeValue = serde_json::from_str(r#"{"z":[1,"a"],"a":{}}"#).expect("json");
        assert_eq!(
            tree_as_json(&value),
            "{\n  \"z\": [\n    1,\n    \"a\"\n  ],\n  \"a\": {}\n}"
        );
    }

    #[test]
    fn a_multiline_string_stays_on_one_row() {
        let value = TreeValue::String("chunk 1 of 4\nchunk 2 of 4\n\tend \"q\"".to_string());
        assert_eq!(
            leaf_text(&value).0,
            r#""chunk 1 of 4\nchunk 2 of 4\n\tend \"q\"""#
        );
        assert_eq!(quoted("añadir"), r#""añadir""#);

        let long = "x".repeat(MAX_LEAF_CHARS + 10);
        let shown = quoted(&long);
        assert_eq!(shown, format!("\"{}…\"", "x".repeat(MAX_LEAF_CHARS)));
        assert!(!shown.contains('\n'));
    }

    fn detail(json: &str) -> NetworkDetail {
        NetworkDetail {
            tree: serde_json::from_str(json).expect("json"),
            raw_request: Arc::from(""),
            raw_response: None,
        }
    }

    #[test]
    fn copy_writes_the_value_key_or_path_of_a_row() {
        let detail = detail(
            r#"{"response":{"headers":{"date":"Wed","x-sample-profile":"self-signed"},
            "body":"chunk 1\nchunk 2","status":200,"ok":true,"none":null,"a.b":{"c":1}}}"#,
        );
        let copy = |path: &str, part| copy_text(&detail.tree, path, part);
        assert_eq!(
            copy("response.headers.x-sample-profile", CopyPart::Value).as_deref(),
            Some("self-signed")
        );
        assert_eq!(
            copy("response.headers.x-sample-profile", CopyPart::Key).as_deref(),
            Some("x-sample-profile")
        );
        assert_eq!(
            copy("response.headers.date", CopyPart::Path).as_deref(),
            Some("response.headers.date")
        );
        assert_eq!(
            copy("response.body", CopyPart::Value).as_deref(),
            Some("chunk 1\nchunk 2")
        );
        assert_eq!(
            copy("response.status", CopyPart::Value).as_deref(),
            Some("200")
        );
        assert_eq!(
            copy("response.ok", CopyPart::Value).as_deref(),
            Some("true")
        );
        assert_eq!(
            copy("response.none", CopyPart::Value).as_deref(),
            Some("null")
        );
        assert_eq!(
            copy("response.headers", CopyPart::Value).as_deref(),
            Some("{\n  \"date\": \"Wed\",\n  \"x-sample-profile\": \"self-signed\"\n}")
        );
        // A key with a dot is found through its own path.
        assert_eq!(
            copy("response.a.b.c", CopyPart::Value).as_deref(),
            Some("1")
        );
        assert_eq!(copy("response.missing", CopyPart::Value), None);
    }

    #[test]
    fn arrow_keys_move_the_selection_and_open_or_close_branches() {
        let detail = detail(
            r#"{"request":{"method":"GET"},"response":{"status":200,"h":{"a":"1"}},"env":"dev"}"#,
        );
        let mut view = ConsoleView::default();
        view.sync("t", 1);
        let selected = |view: &ConsoleView| view.selected.clone().unwrap_or_default();

        // The first key selects the first row.
        assert!(view.navigate(TreeKey::Down, &detail));
        assert_eq!(selected(&view), "request");
        assert!(view.navigate(TreeKey::Down, &detail));
        assert_eq!(selected(&view), "response");
        // Right opens, then goes to the first child; Down walks into the open branch.
        assert!(view.navigate(TreeKey::Right, &detail));
        assert_eq!(selected(&view), "response");
        assert!(view.navigate(TreeKey::Right, &detail));
        assert_eq!(selected(&view), "response.status");
        assert!(view.navigate(TreeKey::Down, &detail));
        assert_eq!(selected(&view), "response.h");
        assert!(view.navigate(TreeKey::Right, &detail));
        assert!(view.navigate(TreeKey::Right, &detail));
        assert_eq!(selected(&view), "response.h.a");
        // A leaf has nothing to open.
        assert!(!view.navigate(TreeKey::Right, &detail));
        // Left goes to the parent, then closes it.
        assert!(view.navigate(TreeKey::Left, &detail));
        assert_eq!(selected(&view), "response.h");
        assert!(view.navigate(TreeKey::Left, &detail));
        assert_eq!(selected(&view), "response.h");
        assert!(view.navigate(TreeKey::End, &detail));
        assert_eq!(selected(&view), "env");
        assert!(!view.navigate(TreeKey::Down, &detail));
        assert!(view.navigate(TreeKey::Home, &detail));
        assert_eq!(selected(&view), "request");
        assert!(!view.navigate(TreeKey::Up, &detail));
    }

    #[test]
    fn closing_a_branch_selects_it_and_a_new_response_clears_the_selection() {
        let detail = detail(r#"{"response":{"headers":{"date":"Wed"}}}"#);
        let mut view = ConsoleView::default();
        view.sync("t", 1);
        view.expand_all(&detail);
        view.select("response.headers.date");
        view.toggle("response");
        view.rows(&detail);
        assert_eq!(view.selected.as_deref(), Some("response"));

        view.sync("t", 2);
        assert_eq!(view.selected, None);
    }
}
