//! The autocompletion of the JavaScript `::: pre` and `::: post` editors: what to offer for the
//! text before the cursor. Plain Rust over the script API schema (`brasa_script::api`), unit
//! tested without a window; the view draws the menu and applies the chosen item.
//!
//! What it offers, by where the cursor is:
//!
//! - In code, a global (`req`, `vars`, ...) after an identifier prefix, or the members of a
//!   member chain after a dot (`req.headers.`, `expect(res.status).not.`). A chain may hold
//!   calls with balanced arguments and `[0]` / `["key"]` indexes. `res` only exists in a post
//!   script. An empty prefix offers nothing unless the user asked explicitly (Ctrl+Space).
//! - In a string that is the first argument of `vars.get`, `vars.set`, `env.get`, `env.set` or
//!   `env.unset`: the variable names. In `req.headers.get/set/remove` and `res.headers.get`: the
//!   common header names.
//! - After `res.json()` (or a `const body = res.json();` alias), in a post script: the keys of
//!   the last response body, from a [`JsonShape`] the caller builds once per response.
//! - Nowhere else: inside comments, in any other string or in a template literal's text.
//!
//! What an inserted function looks like: a function with arguments inserts `name(`, one without
//! inserts `name()`, and either inserts just `name` when the text after the cursor already
//! starts with `(`. The parameters (`(name, value)`) are in the item's `detail`.
//!
//! Known limits, all on the safe side (a wrong guess offers nothing or too little): regular
//! expression literals are not recognized, so a quote or `//` inside one can confuse the
//! comment and string detection; a `)` or `]` inside a string inside call arguments is skipped
//! with a simple quote matcher; aliases are found by a text scan (`const`, `let` or `var`
//! followed by a name, `=` and a chain), without scopes or destructuring, and the last
//! declaration before the cursor wins.

use std::collections::HashSet;
use std::ops::Range;

use brasa_script::api::{self, ApiEntry, MemberKind, Returns};

use super::completion::rank;
use super::script_heuristics::vars_set_names;
use super::variable_hint::{Hint, MASK};

/// Deepest level of a response body a [`JsonShape`] keeps.
pub const MAX_JSON_DEPTH: usize = 8;

/// Most values a [`JsonShape`] visits, so building it stays cheap for any body.
pub const MAX_JSON_NODES: usize = 5000;

/// Most elements of an array that are merged into the shape of its elements.
pub const MAX_JSON_ARRAY_ELEMENTS: usize = 20;

/// Largest body, in bytes, [`JsonShape::from_body`] scans. A bigger one gets no key completion.
/// It matches `brasa_http::DEFAULT_MAX_RESPONSE_SIZE`, the scan is streaming so it costs time,
/// not memory.
pub const MAX_JSON_BODY_BYTES: usize = 20_000_000;

/// Builds a [`JsonShape`] straight from a JSON reader with the rules of `JsonShape::build`.
/// What is out of the caps is still read, with `IgnoredAny`, so the document stays valid.
struct Stream<'a> {
    depth: usize,
    budget: &'a mut usize,
}

impl Stream<'_> {
    /// A scalar: a leaf that spends one node of the budget while there is any.
    fn scalar(self, ty: JsonType) -> JsonShape {
        if *self.budget > 0 && self.depth < MAX_JSON_DEPTH {
            *self.budget -= 1;
        }
        JsonShape::leaf(ty)
    }
}

impl<'de> serde::de::DeserializeSeed<'de> for Stream<'_> {
    type Value = JsonShape;

    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<JsonShape, D::Error> {
        de.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for Stream<'_> {
    type Value = JsonShape;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<JsonShape, E> {
        Ok(self.scalar(JsonType::Null))
    }

    fn visit_bool<E>(self, _: bool) -> Result<JsonShape, E> {
        Ok(self.scalar(JsonType::Boolean))
    }

    fn visit_i64<E>(self, _: i64) -> Result<JsonShape, E> {
        Ok(self.scalar(JsonType::Number))
    }

    fn visit_u64<E>(self, _: u64) -> Result<JsonShape, E> {
        Ok(self.scalar(JsonType::Number))
    }

    fn visit_f64<E>(self, _: f64) -> Result<JsonShape, E> {
        Ok(self.scalar(JsonType::Number))
    }

    fn visit_str<E>(self, _: &str) -> Result<JsonShape, E> {
        Ok(self.scalar(JsonType::String))
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<JsonShape, A::Error> {
        use serde::de::IgnoredAny;
        let mut shape = JsonShape::leaf(JsonType::Array);
        if *self.budget == 0 || self.depth >= MAX_JSON_DEPTH {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            return Ok(shape);
        }
        *self.budget -= 1;
        let mut taken = 0;
        while taken < MAX_JSON_ARRAY_ELEMENTS && *self.budget > 0 {
            let seed = Stream {
                depth: self.depth + 1,
                budget: &mut *self.budget,
            };
            let Some(built) = seq.next_element_seed(seed)? else {
                return Ok(shape);
            };
            taken += 1;
            shape.element = Some(Box::new(match shape.element.take() {
                Some(merged) => (*merged).merge(built),
                None => built,
            }));
        }
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(shape)
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<JsonShape, A::Error> {
        use serde::de::IgnoredAny;
        let mut shape = JsonShape::leaf(JsonType::Object);
        if *self.budget == 0 || self.depth >= MAX_JSON_DEPTH {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            return Ok(shape);
        }
        *self.budget -= 1;
        while *self.budget > 0 {
            let Some(key) = map.next_key::<String>()? else {
                break;
            };
            let seed = Stream {
                depth: self.depth + 1,
                budget: &mut *self.budget,
            };
            let built = map.next_value_seed(seed)?;
            // A repeated key keeps its last value, like `serde_json::Map`.
            match shape.fields.iter_mut().find(|(name, _)| *name == key) {
                Some(field) => field.1 = built,
                None => shape.fields.push((key, built)),
            }
        }
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        shape.fields.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(shape)
    }
}

/// The kind of script being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    /// The `::: pre` section: no `res`, `req` is writable.
    Pre,
    /// The `::: post` section: `res` exists and `req` is read-only.
    Post,
}

/// What an item is, for its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A global or member holding other members, such as `req`.
    Object,
    /// A function.
    Function,
    /// A plain value, such as `res.status`.
    Property,
    /// A variable name inside a `vars.*` or `env.*` call.
    Variable,
    /// A header name inside a `*.headers.*` call.
    Header,
    /// A key of the last response body's JSON.
    JsonKey,
}

/// One row of the script completion menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptCompletion {
    /// What the row shows.
    pub label: String,
    /// What the row is.
    pub kind: ItemKind,
    /// A function's parameters, a property's type, a variable's value (masked for a secret) or a
    /// JSON key's type.
    pub detail: Option<String>,
    /// A one or two sentence description, for the item's tooltip.
    pub documentation: Option<String>,
    /// The text that replaces `replace` when the row is chosen.
    pub insert: String,
    /// The byte range of the script text that `insert` replaces.
    pub replace: Range<usize>,
}

/// The JSON type of a value in a [`JsonShape`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonType {
    /// `null`.
    Null,
    /// `true` or `false`.
    Boolean,
    /// A number.
    Number,
    /// A string.
    String,
    /// An array.
    Array,
    /// An object.
    Object,
}

impl JsonType {
    /// The name shown in a completion's detail.
    pub fn name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Number => "number",
            Self::String => "string",
            Self::Array => "array",
            Self::Object => "object",
        }
    }
}

/// The keys of a JSON document, without its values: a small tree built once per response and
/// used to complete `res.json().` chains. It is bounded by [`MAX_JSON_DEPTH`],
/// [`MAX_JSON_NODES`] and [`MAX_JSON_ARRAY_ELEMENTS`], whatever the size of the body. The
/// elements of an array are merged into one shape, so every key found in any of the first few is
/// offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonShape {
    ty: JsonType,
    fields: Vec<(String, JsonShape)>,
    element: Option<Box<JsonShape>>,
}

impl JsonShape {
    fn leaf(ty: JsonType) -> Self {
        Self {
            ty,
            fields: Vec::new(),
            element: None,
        }
    }

    /// The shape of `value`. [`Self::from_body`] gives the same without building a `Value`.
    #[cfg(test)]
    pub fn from_value(value: &serde_json::Value) -> Self {
        let mut budget = MAX_JSON_NODES;
        Self::build(value, 0, &mut budget)
    }

    /// The shape of the JSON text `body`, `None` when it is not valid JSON or larger than
    /// [`MAX_JSON_BODY_BYTES`]. The shape is built while the text is read, so no `Value` tree
    /// (about 30 times the size of the body) is ever allocated. It is the same shape
    /// [`Self::from_value`] gives, except that with a node budget that runs out in the middle of
    /// an object the keys are spent in document order instead of sorted order.
    pub fn from_body(body: &str) -> Option<Self> {
        if body.len() > MAX_JSON_BODY_BYTES {
            return None;
        }
        use serde::de::DeserializeSeed;
        let mut budget = MAX_JSON_NODES;
        let mut de = serde_json::Deserializer::from_str(body);
        let shape = Stream {
            depth: 0,
            budget: &mut budget,
        }
        .deserialize(&mut de)
        .ok()?;
        de.end().ok()?;
        Some(shape)
    }

    #[cfg(test)]
    fn build(value: &serde_json::Value, depth: usize, budget: &mut usize) -> Self {
        use serde_json::Value;
        let ty = match value {
            Value::Null => JsonType::Null,
            Value::Bool(_) => JsonType::Boolean,
            Value::Number(_) => JsonType::Number,
            Value::String(_) => JsonType::String,
            Value::Array(_) => JsonType::Array,
            Value::Object(_) => JsonType::Object,
        };
        let mut shape = Self::leaf(ty);
        if *budget == 0 || depth >= MAX_JSON_DEPTH {
            return shape;
        }
        *budget -= 1;
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    if *budget == 0 {
                        break;
                    }
                    shape
                        .fields
                        .push((key.clone(), Self::build(child, depth + 1, budget)));
                }
            }
            Value::Array(items) => {
                for item in items.iter().take(MAX_JSON_ARRAY_ELEMENTS) {
                    if *budget == 0 {
                        break;
                    }
                    let built = Self::build(item, depth + 1, budget);
                    shape.element = Some(Box::new(match shape.element.take() {
                        Some(merged) => (*merged).merge(built),
                        None => built,
                    }));
                }
            }
            _ => {}
        }
        shape.fields.sort_by(|a, b| a.0.cmp(&b.0));
        shape
    }

    /// Merges `other` into `self`: a null or any other plain value gives way to an object or an
    /// array (they have something to complete), two objects join their keys, two arrays join
    /// their elements, and otherwise the first shape stays.
    fn merge(mut self, other: Self) -> Self {
        let structured = |ty| matches!(ty, JsonType::Object | JsonType::Array);
        if self.ty == JsonType::Null || (!structured(self.ty) && structured(other.ty)) {
            return other;
        }
        if self.ty != other.ty {
            return self;
        }
        for (key, shape) in other.fields {
            match self.fields.iter().position(|(name, _)| *name == key) {
                Some(index) => {
                    let existing =
                        std::mem::replace(&mut self.fields[index].1, Self::leaf(JsonType::Null));
                    self.fields[index].1 = existing.merge(shape);
                }
                None => self.fields.push((key, shape)),
            }
        }
        self.element = match (self.element.take(), other.element) {
            (Some(a), Some(b)) => Some(Box::new((*a).merge(*b))),
            (a, b) => a.or(b),
        };
        self.fields.sort_by(|a, b| a.0.cmp(&b.0));
        self
    }

    /// The JSON type of this value.
    pub fn ty(&self) -> JsonType {
        self.ty
    }

    /// The keys of an object with the shape of their values, sorted by name. Empty for any
    /// other type.
    pub fn fields(&self) -> &[(String, JsonShape)] {
        &self.fields
    }

    /// The shape of the key `name` of an object.
    pub fn field(&self, name: &str) -> Option<&JsonShape> {
        self.fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, shape)| shape)
    }

    /// The merged shape of an array's elements.
    pub fn element(&self) -> Option<&JsonShape> {
        self.element.as_deref()
    }

    /// How many values the shape holds, itself included.
    #[cfg(test)]
    pub fn node_count(&self) -> usize {
        1 + self
            .fields
            .iter()
            .map(|(_, shape)| shape.node_count())
            .sum::<usize>()
            + self.element.as_ref().map_or(0, |shape| shape.node_count())
    }
}

/// Everything besides the text that decides what is offered.
pub struct ScriptContext<'a> {
    /// Whether the script is a pre or a post script.
    pub kind: ScriptKind,
    /// The variables the editor knows, as the autocompletion of `{{ }}` markers lists them
    /// (`variable_hint::completions`). Their values are already masked when secret.
    pub variables: &'a [Hint],
    /// The header names offered inside `req.headers.*` and `res.headers.get`.
    pub header_names: &'a [&'static str],
    /// The shape of the last response body, `None` when there is none or it is not JSON.
    pub response: Option<&'a JsonShape>,
}

/// Whether typing `new_text` should open or refresh the menu: it ends in an identifier
/// character, `.`, or a quote (`"`, `'` or `` ` ``).
pub fn is_trigger(new_text: &str) -> bool {
    new_text
        .chars()
        .next_back()
        .is_some_and(|c| is_ident_char(c) || matches!(c, '.' | '"' | '\'' | '`'))
}

/// The rows to offer for the cursor at byte offset `cursor` of `text`, best first: names that
/// start with what is typed, then names that contain it, each in the schema's order. `explicit`
/// is true when the user asked for the menu (Ctrl+Space) rather than typed: then an empty prefix
/// lists every global. Empty when `cursor` is out of range or inside a character.
pub fn completions(
    text: &str,
    cursor: usize,
    context: &ScriptContext,
    explicit: bool,
) -> Vec<ScriptCompletion> {
    if !text.is_char_boundary(cursor) {
        return Vec::new();
    }
    let scan = scan(&text[..cursor]);
    match scan.position {
        Position::Comment | Position::Template => Vec::new(),
        Position::Str { quote, start } => string_completions(text, cursor, quote, start, context),
        Position::Code => code_completions(text, cursor, &scan.comments, context, explicit),
    }
}

// --- lexer ---------------------------------------------------------------------------------

/// Where the end of a text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    /// In code, including the `${ }` of a template literal.
    Code,
    /// In a line or block comment.
    Comment,
    /// In the text of a template literal.
    Template,
    /// In a `'` or `"` string opened at byte `start`.
    Str { quote: char, start: usize },
}

/// The result of lexing the text before the cursor.
struct Scan {
    position: Position,
    /// The byte ranges of the comments, the one the cursor is in included.
    comments: Vec<Range<usize>>,
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    /// Code, with the depth of its own `{ }`. Level 0 inside `${ }` ends at the `}`.
    Code(usize),
    /// The text of a template literal.
    Template,
}

/// Lexes `text` (the script up to the cursor) just enough to know what its end is in.
fn scan(text: &str) -> Scan {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut comments = Vec::new();
    let mut stack = vec![Mode::Code(0)];
    let next = |i: usize| chars.get(i + 1).map(|&(_, c)| c);
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        let Some(&mode) = stack.last() else { break };
        match mode {
            Mode::Template => match c {
                '\\' => i += 1,
                '`' => {
                    stack.pop();
                }
                '$' if next(i) == Some('{') => {
                    stack.push(Mode::Code(0));
                    i += 1;
                }
                _ => {}
            },
            Mode::Code(braces) => match c {
                '/' if next(i) == Some('/') => {
                    let mut j = i + 2;
                    while j < chars.len() && chars[j].1 != '\n' {
                        j += 1;
                    }
                    let end = chars.get(j).map_or(text.len(), |&(p, _)| p);
                    comments.push(pos..end);
                    if j >= chars.len() {
                        return Scan {
                            position: Position::Comment,
                            comments,
                        };
                    }
                    i = j;
                    continue;
                }
                '/' if next(i) == Some('*') => {
                    let mut j = i + 2;
                    while j < chars.len()
                        && !(chars[j - 1].1 == '*' && chars[j].1 == '/' && j > i + 2)
                    {
                        j += 1;
                    }
                    if j >= chars.len() {
                        comments.push(pos..text.len());
                        return Scan {
                            position: Position::Comment,
                            comments,
                        };
                    }
                    comments.push(pos..chars[j].0 + 1);
                    i = j;
                }
                '\'' | '"' => {
                    let mut j = i + 1;
                    loop {
                        match chars.get(j).map(|&(_, ch)| ch) {
                            None => {
                                return Scan {
                                    position: Position::Str {
                                        quote: c,
                                        start: pos,
                                    },
                                    comments,
                                };
                            }
                            Some('\\') => j += 2,
                            Some(ch) if ch == c => break,
                            // An unterminated string ends with its line.
                            Some('\n') => {
                                j -= 1;
                                break;
                            }
                            Some(_) => j += 1,
                        }
                    }
                    i = j;
                }
                '`' => stack.push(Mode::Template),
                '{' => {
                    if let Some(top) = stack.last_mut() {
                        *top = Mode::Code(braces + 1);
                    }
                }
                '}' => {
                    if braces > 0 {
                        if let Some(top) = stack.last_mut() {
                            *top = Mode::Code(braces - 1);
                        }
                    } else if stack.len() > 1 {
                        stack.pop();
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    let position = match stack.last() {
        Some(Mode::Template) => Position::Template,
        _ => Position::Code,
    };
    Scan { position, comments }
}

// --- member chains -------------------------------------------------------------------------

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(is_ident_char)
}

/// One step of a member chain.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    /// `.name` or the first identifier.
    Name(String),
    /// `(...)`, whatever its arguments.
    Call,
    /// `["key"]` or `['key']`.
    Key(String),
    /// `[0]` or any other index that is not a plain string.
    Element,
}

/// The byte index where the identifier ending at `end` starts.
fn ident_start(text: &str, end: usize) -> usize {
    let mut start = end;
    for (index, c) in text[..end].char_indices().rev() {
        if !is_ident_char(c) {
            break;
        }
        start = index;
    }
    start
}

/// A `.` (or `?.`) right before byte `end`.
struct Dot {
    /// The byte index of the `.`.
    index: usize,
    /// Whether it is `?.`.
    optional: bool,
}

impl Dot {
    /// Where the expression the dot follows ends.
    fn receiver_end(&self) -> usize {
        if self.optional {
            self.index - 1
        } else {
            self.index
        }
    }
}

fn dot_before(text: &str, end: usize) -> Option<Dot> {
    let before = &text[..end];
    let index = before.strip_suffix('.')?.len();
    Some(Dot {
        index,
        optional: before[..index].ends_with('?'),
    })
}

/// The index of the opening bracket that matches the `close` bracket at byte `close`, reading
/// backwards and skipping quoted strings.
fn matching_open(text: &str, close: usize) -> Option<usize> {
    let close_char = text[close..].chars().next()?;
    let open_char = if close_char == ')' { '(' } else { '[' };
    let chars: Vec<(usize, char)> = text[..close].char_indices().collect();
    let mut depth = 1;
    let mut j = chars.len();
    while j > 0 {
        j -= 1;
        let (index, c) = chars[j];
        if matches!(c, '"' | '\'' | '`') {
            let mut k = j;
            loop {
                if k == 0 {
                    return None;
                }
                k -= 1;
                if chars[k].1 == c && !(k > 0 && chars[k - 1].1 == '\\') {
                    break;
                }
            }
            j = k;
        } else if c == close_char {
            depth += 1;
        } else if c == open_char {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

/// The index just after the bracket that matches the opening one at byte `open`, reading
/// forwards and skipping quoted strings.
fn matching_close(text: &str, open: usize) -> Option<usize> {
    let open_char = text[open..].chars().next()?;
    let close_char = if open_char == '(' { ')' } else { ']' };
    let mut depth = 0;
    let mut chars = text[open..].char_indices();
    while let Some((index, c)) = chars.next() {
        if matches!(c, '"' | '\'' | '`') {
            let mut escaped = false;
            for (_, inner) in chars.by_ref() {
                if escaped {
                    escaped = false;
                } else if inner == '\\' {
                    escaped = true;
                } else if inner == c {
                    break;
                }
            }
        } else if c == open_char {
            depth += 1;
        } else if c == close_char {
            depth -= 1;
            if depth == 0 {
                return Some(open + index + c.len_utf8());
            }
        }
    }
    None
}

/// Reads the member chain that ends right before byte `end`, such as `expect(res.status)` or
/// `res.json().items[0]`, backwards. Returns its steps in order and the byte index where it
/// starts, `None` when what precedes `end` is not a chain (an operator, a parenthesized group).
fn parse_chain(text: &str, mut end: usize) -> Option<(Vec<Seg>, usize)> {
    let mut reversed = Vec::new();
    loop {
        let last = text[..end].chars().next_back()?;
        match last {
            ')' => {
                let open = matching_open(text, end - 1)?;
                reversed.push(Seg::Call);
                end = open;
            }
            ']' => {
                let open = matching_open(text, end - 1)?;
                let inner = text[open + 1..end - 1].trim();
                let quoted = inner.len() >= 2
                    && ((inner.starts_with('"') && inner.ends_with('"'))
                        || (inner.starts_with('\'') && inner.ends_with('\'')));
                reversed.push(if quoted {
                    Seg::Key(inner[1..inner.len() - 1].to_string())
                } else {
                    Seg::Element
                });
                end = open;
            }
            c if is_ident_char(c) => {
                let start = ident_start(text, end);
                reversed.push(Seg::Name(text[start..end].to_string()));
                match dot_before(text, start) {
                    Some(dot) => end = dot.receiver_end(),
                    None => {
                        reversed.reverse();
                        return Some((reversed, start));
                    }
                }
            }
            _ => return None,
        }
    }
}

/// The byte index where the member chain starting at byte 0 of `text` ends.
fn chain_end(text: &str) -> usize {
    let first = text
        .char_indices()
        .find(|&(_, c)| !is_ident_char(c))
        .map_or(text.len(), |(index, _)| index);
    let mut end = first;
    loop {
        let rest = &text[end..];
        match rest.chars().next() {
            Some('(') | Some('[') => match matching_close(text, end) {
                Some(close) => end = close,
                None => return end,
            },
            Some('.') | Some('?') => {
                let after = rest.strip_prefix("?.").or_else(|| rest.strip_prefix('.'));
                let Some(after) = after else { return end };
                let name = after.len() - after.trim_start_matches(is_ident_char).len();
                if name == 0 {
                    return end;
                }
                end = text.len() - after.len() + name;
            }
            _ => return end,
        }
    }
}

// --- resolution ----------------------------------------------------------------------------

/// What a member chain evaluates to.
#[derive(Clone, Copy)]
enum Value<'a> {
    /// A schema entry, and whether a function has been called.
    Entry(&'static ApiEntry, bool),
    /// A value of the last response's JSON.
    Json(&'a JsonShape),
}

/// A local name declared with `const`, `let` or `var`, and what it resolved to.
type Alias<'a> = (String, Option<Value<'a>>);

fn step<'a>(value: Value<'a>, seg: &Seg, context: &ScriptContext<'a>) -> Option<Value<'a>> {
    match (value, seg) {
        (Value::Entry(entry, called), Seg::Name(name)) => {
            if (entry.kind == MemberKind::Function) != called {
                return None;
            }
            entry.member(name).map(|member| Value::Entry(member, false))
        }
        (Value::Entry(entry, false), Seg::Call) if entry.kind == MemberKind::Function => {
            if entry.returns == Returns::Json {
                context.response.map(Value::Json)
            } else {
                Some(Value::Entry(entry, true))
            }
        }
        (Value::Json(shape), Seg::Name(name)) | (Value::Json(shape), Seg::Key(name)) => {
            shape.field(name).map(Value::Json)
        }
        (Value::Json(shape), Seg::Element) => shape.element().map(Value::Json),
        _ => None,
    }
}

fn resolve<'a>(
    segs: &[Seg],
    aliases: &[Alias<'a>],
    context: &ScriptContext<'a>,
) -> Option<Value<'a>> {
    let (Seg::Name(first), rest) = segs.split_first()? else {
        return None;
    };
    let mut value = match aliases.iter().rev().find(|(name, _)| name == first) {
        Some((_, resolved)) => (*resolved)?,
        None => Value::Entry(api::global(first, context.kind == ScriptKind::Post)?, false),
    };
    for seg in rest {
        value = step(value, seg, context)?;
    }
    Some(value)
}

/// The text with every comment replaced by spaces, keeping byte offsets.
fn blank_comments(text: &str, comments: &[Range<usize>]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for comment in comments {
        out.push_str(&text[at..comment.start]);
        for c in text[comment.clone()].chars() {
            out.extend(std::iter::repeat_n(' ', c.len_utf8()));
        }
        at = comment.end;
    }
    out.push_str(&text[at..]);
    out
}

/// The names declared before the cursor with `const`, `let` or `var` and a chain after the `=`,
/// in order, each resolved with the ones before it.
fn find_aliases<'a>(
    before_cursor: &str,
    comments: &[Range<usize>],
    context: &ScriptContext<'a>,
) -> Vec<Alias<'a>> {
    let code = blank_comments(before_cursor, comments);
    let mut aliases: Vec<Alias<'a>> = Vec::new();
    let mut at = 0;
    while at < code.len() {
        let rest = &code[at..];
        let Some(offset) = rest.find(|c: char| is_ident_char(c)) else {
            break;
        };
        let word_start = at + offset;
        let word_end = word_start
            + code[word_start..]
                .find(|c: char| !is_ident_char(c))
                .unwrap_or(code.len() - word_start);
        at = word_end;
        let word = &code[word_start..word_end];
        let after_dot = dot_before(&code, word_start).is_some();
        if after_dot || !matches!(word, "const" | "let" | "var") {
            continue;
        }
        let after = code[word_end..].trim_start();
        let name_len = after.len() - after.trim_start_matches(is_ident_char).len();
        let name = &after[..name_len];
        if !is_identifier(name) {
            continue;
        }
        let Some(init) = after[name_len..].trim_start().strip_prefix('=') else {
            continue;
        };
        if init.starts_with(['=', '>']) {
            continue;
        }
        let init = init.trim_start();
        let value = parse_chain(init, chain_end(init))
            .filter(|(_, start)| *start == 0)
            .and_then(|(segs, _)| resolve(&segs, &aliases, context));
        aliases.push((name.to_string(), value));
    }
    aliases
}

// --- completions in code -------------------------------------------------------------------

fn api_item(entry: &ApiEntry, replace: Range<usize>, after_cursor: &str) -> ScriptCompletion {
    let insert = match entry.kind {
        MemberKind::Function if after_cursor.starts_with('(') => entry.name.to_string(),
        MemberKind::Function if entry.takes_no_arguments() => format!("{}()", entry.name),
        MemberKind::Function => format!("{}(", entry.name),
        _ => entry.name.to_string(),
    };
    ScriptCompletion {
        label: entry.name.to_string(),
        kind: match entry.kind {
            MemberKind::Object => ItemKind::Object,
            MemberKind::Function => ItemKind::Function,
            MemberKind::Property => ItemKind::Property,
        },
        // The parameters alone: the label already shows the name.
        detail: entry
            .signature
            .map(|signature| signature.strip_prefix(entry.name).unwrap_or(signature))
            .or(entry.ty)
            .or(matches!(entry.kind, MemberKind::Object).then_some("object"))
            .map(str::to_string),
        documentation: Some(entry.doc.to_string()),
        insert,
        replace,
    }
}

fn json_items(shape: &JsonShape, partial: &str, dot: &Dot, cursor: usize) -> Vec<ScriptCompletion> {
    let partial_start = cursor - partial.len();
    rank(shape.fields(), partial, |(key, _)| key)
        .into_iter()
        .map(|(key, child)| {
            let (insert, replace) = if is_identifier(key) {
                (key.clone(), partial_start..cursor)
            } else {
                // Not a valid name after a dot: replace the dot with an index.
                let escaped = key.replace('\\', "\\\\").replace('"', "\\\"");
                let dot_text = if dot.optional { "." } else { "" };
                (format!("{dot_text}[\"{escaped}\"]"), dot.index..cursor)
            };
            ScriptCompletion {
                label: key.clone(),
                kind: ItemKind::JsonKey,
                detail: Some(child.ty().name().to_string()),
                documentation: None,
                insert,
                replace,
            }
        })
        .collect()
}

fn code_completions(
    text: &str,
    cursor: usize,
    comments: &[Range<usize>],
    context: &ScriptContext,
    explicit: bool,
) -> Vec<ScriptCompletion> {
    let partial_start = ident_start(text, cursor);
    let partial = &text[partial_start..cursor];
    if partial.starts_with(|c: char| c.is_ascii_digit()) {
        return Vec::new();
    }
    let after_cursor = &text[cursor..];
    let replace = partial_start..cursor;
    let Some(dot) = dot_before(text, partial_start) else {
        if partial.is_empty() && !explicit {
            return Vec::new();
        }
        let post = context.kind == ScriptKind::Post;
        let available: Vec<&ApiEntry> = api::GLOBALS
            .iter()
            .filter(|entry| entry.availability.available_in(post))
            .collect();
        return rank(&available, partial, |entry| entry.name)
            .into_iter()
            .map(|entry| api_item(entry, replace.clone(), after_cursor))
            .collect();
    };
    let Some((segs, _)) = parse_chain(text, dot.receiver_end()) else {
        return Vec::new();
    };
    let aliases = find_aliases(&text[..cursor], comments, context);
    match resolve(&segs, &aliases, context) {
        Some(Value::Json(shape)) => json_items(shape, partial, &dot, cursor),
        Some(Value::Entry(entry, called)) => {
            if (entry.kind == MemberKind::Function) != called {
                return Vec::new();
            }
            rank(entry.members, partial, |member| member.name)
                .into_iter()
                .map(|member| api_item(member, replace.clone(), after_cursor))
                .collect()
        }
        None => Vec::new(),
    }
}

// --- completions in strings ----------------------------------------------------------------

/// What the first string argument of a call names.
enum Argument {
    Variable { with_script_names: bool },
    Header,
}

fn argument_of(callee: &[&str], kind: ScriptKind) -> Option<Argument> {
    match callee {
        ["vars", "get" | "set"] => Some(Argument::Variable {
            with_script_names: true,
        }),
        ["env", "get" | "set" | "unset"] => Some(Argument::Variable {
            with_script_names: false,
        }),
        ["req", "headers", "get" | "set" | "remove"] => Some(Argument::Header),
        ["res", "headers", "get"] if kind == ScriptKind::Post => Some(Argument::Header),
        _ => None,
    }
}

fn string_completions(
    text: &str,
    cursor: usize,
    quote: char,
    start: usize,
    context: &ScriptContext,
) -> Vec<ScriptCompletion> {
    let _ = quote;
    let typed = &text[start + 1..cursor];
    if typed.contains('\\') {
        return Vec::new();
    }
    let before = text[..start].trim_end();
    if !before.ends_with('(') {
        return Vec::new();
    }
    let Some((segs, _)) = parse_chain(text, before.len() - 1) else {
        return Vec::new();
    };
    let names: Option<Vec<&str>> = segs
        .iter()
        .map(|seg| match seg {
            Seg::Name(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let Some(argument) = names.and_then(|names| argument_of(&names, context.kind)) else {
        return Vec::new();
    };
    let replace = start + 1..cursor;
    let item = |label: String, kind, detail| ScriptCompletion {
        insert: label.clone(),
        label,
        kind,
        detail,
        documentation: None,
        replace: replace.clone(),
    };
    match argument {
        Argument::Header => {
            let all: Vec<&&str> = context.header_names.iter().collect();
            rank(&all, typed, |name| name)
                .into_iter()
                .map(|name| item((**name).to_string(), ItemKind::Header, None))
                .collect()
        }
        Argument::Variable { with_script_names } => {
            let mut all: Vec<(String, Option<String>)> = context
                .variables
                .iter()
                .map(|hint| (hint.name.clone(), variable_detail(hint)))
                .collect();
            if with_script_names {
                let known: HashSet<String> = all.iter().map(|(name, _)| name.clone()).collect();
                let mut own: Vec<String> = vars_set_names(&text[..start])
                    .into_iter()
                    .filter(|name| !known.contains(name))
                    .collect();
                own.sort_by_key(|name| (name.to_lowercase(), name.clone()));
                all.extend(own.into_iter().map(|name| (name, None)));
            }
            rank(&all, typed, |(name, _)| name)
                .into_iter()
                .map(|(name, detail)| item(name.clone(), ItemKind::Variable, detail.clone()))
                .collect()
        }
    }
}

/// What a variable row shows on the right: its value, or the mask for a secret.
fn variable_detail(hint: &Hint) -> Option<String> {
    if hint.masked {
        Some(MASK.to_string())
    } else {
        hint.value.clone()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn hint(name: &str, value: Option<&str>, masked: bool) -> Hint {
        Hint {
            name: name.to_string(),
            value: value.map(str::to_string),
            masked,
            origin: None,
        }
    }

    fn context(kind: ScriptKind) -> ScriptContext<'static> {
        ScriptContext {
            kind,
            variables: &[],
            header_names: &[],
            response: None,
        }
    }

    /// Splits `marked` at its `|`, the cursor.
    fn cursor_in(marked: &str) -> (String, usize) {
        let cursor = marked.find('|').expect("a | marks the cursor");
        (marked.replacen('|', "", 1), cursor)
    }

    fn run(marked: &str, context: &ScriptContext, explicit: bool) -> Vec<ScriptCompletion> {
        let (text, cursor) = cursor_in(marked);
        completions(&text, cursor, context, explicit)
    }

    fn labels(items: &[ScriptCompletion]) -> Vec<&str> {
        items.iter().map(|item| item.label.as_str()).collect()
    }

    fn pre(marked: &str) -> Vec<String> {
        let ctx = context(ScriptKind::Pre);
        let items = run(marked, &ctx, false);
        labels(&items).into_iter().map(str::to_string).collect()
    }

    fn post(marked: &str) -> Vec<String> {
        let ctx = context(ScriptKind::Post);
        let items = run(marked, &ctx, false);
        labels(&items).into_iter().map(str::to_string).collect()
    }

    fn shape() -> JsonShape {
        JsonShape::from_value(&json!({
            "id": 1,
            "token": "abc",
            "data": { "name": "x", "tags": ["a"], "a-b": true },
            "items": [ { "sku": "s", "price": 2 }, { "sku": "t", "stock": null } ],
        }))
    }

    fn post_with(shape: &JsonShape, marked: &str) -> Vec<ScriptCompletion> {
        let ctx = ScriptContext {
            response: Some(shape),
            ..context(ScriptKind::Post)
        };
        run(marked, &ctx, false)
    }

    #[test]
    fn a_global_prefix_lists_the_globals_by_rank() {
        assert_eq!(pre("re|"), ["req"]);
        assert_eq!(post("re|"), ["req", "res"]);
        assert_eq!(pre("x = ex|"), ["expect"]);
        assert_eq!(pre("e|"), ["env", "expect", "req", "test", "console"]);
    }

    #[test]
    fn an_empty_prefix_offers_nothing_unless_explicit() {
        assert!(pre("|").is_empty());
        assert!(pre("x = |").is_empty());
        let ctx = context(ScriptKind::Pre);
        let items = run("|", &ctx, true);
        assert_eq!(
            labels(&items),
            ["req", "vars", "env", "test", "expect", "console", "util"]
        );
        let ctx = context(ScriptKind::Post);
        assert_eq!(run("|", &ctx, true).len(), 8);
    }

    #[test]
    fn res_is_only_available_in_a_post_script() {
        assert!(pre("res.|").is_empty());
        assert_eq!(
            post("res.|"),
            ["status", "headers", "body", "json", "timeMs", "size"]
        );
        assert_eq!(post("res.t|"), ["timeMs", "status"]);
    }

    #[test]
    fn a_dot_lists_every_member_and_a_partial_filters_them() {
        assert_eq!(pre("req.|"), ["method", "url", "body", "headers"]);
        assert_eq!(pre("req.h|"), ["headers", "method"]);
        assert_eq!(pre("req.headers.|"), ["get", "set", "remove"]);
        assert_eq!(
            pre("util.|"),
            [
                "uuid",
                "now",
                "isoDate",
                "randomInt",
                "randomString",
                "base64Encode",
                "base64Decode",
                "urlEncode"
            ]
        );
        assert_eq!(pre("console.|"), ["log", "info", "warn", "error"]);
        assert_eq!(pre("vars.|"), ["get", "set"]);
        assert_eq!(pre("env.|"), ["get", "set", "unset"]);
    }

    #[test]
    fn optional_chaining_counts_as_a_dot() {
        assert_eq!(pre("req?.|"), ["method", "url", "body", "headers"]);
        assert_eq!(pre("req?.ur|"), ["url"]);
    }

    #[test]
    fn unknown_chains_offer_nothing() {
        assert!(pre("foo.|").is_empty());
        assert!(pre("req.nope.|").is_empty());
        assert!(pre("vars.get(1).|").is_empty());
        assert!(pre("expect.|").is_empty());
        assert!(pre("req.url.|").is_empty());
        assert!(pre("(a + b).|").is_empty());
        assert!(pre("1.|").is_empty());
        assert!(pre("req.headers.get.|").is_empty());
    }

    #[test]
    fn functions_insert_their_name_with_a_parenthesis() {
        let ctx = context(ScriptKind::Pre);
        let item = |marked: &str| run(marked, &ctx, false).remove(0);
        let with_args = item("vars.se|");
        assert_eq!(with_args.insert, "set(");
        assert_eq!(with_args.detail.as_deref(), Some("(name, value)"));
        assert_eq!(with_args.kind, ItemKind::Function);
        assert_eq!(item("util.uui|").insert, "uuid()");
        assert_eq!(item("req.ur|").insert, "url");
        assert_eq!(item("req.ur|").detail.as_deref(), Some("string"));
        assert_eq!(item("req.hea|").kind, ItemKind::Object);
        assert_eq!(item("vars.se|").documentation.is_some(), true);
        // A parenthesis already after the cursor is not doubled.
        assert_eq!(item("vars.se|()").insert, "set");
        assert_eq!(item("ex|(1)").insert, "expect");
    }

    #[test]
    fn the_replaced_range_is_the_partial_identifier() {
        let ctx = context(ScriptKind::Pre);
        let (text, cursor) = cursor_in("req.hea|");
        let items = completions(&text, cursor, &ctx, false);
        assert_eq!(items[0].replace, 4..7);
        let (text, cursor) = cursor_in("req.|");
        assert_eq!(completions(&text, cursor, &ctx, false)[0].replace, 4..4);
    }

    #[test]
    fn expect_chains_through_calls_and_not() {
        let matchers = [
            "toBe",
            "toEqual",
            "toBeTruthy",
            "toBeFalsy",
            "toContain",
            "toMatch",
            "toBeGreaterThan",
            "toBeLessThan",
            "toHaveProperty",
        ];
        let mut with_not = matchers.map(String::from).to_vec();
        with_not.push("not".to_string());
        assert_eq!(post("expect(res.status).|"), with_not);
        assert_eq!(post("expect(res.status).not.|"), matchers);
        assert_eq!(post("expect(res.status).not.toBeG|"), ["toBeGreaterThan"]);
        assert_eq!(post("expect(res.json().a).not.to|").len(), 9);
        assert_eq!(pre("expect(f(a, g(b))).toBeT|"), ["toBeTruthy"]);
        assert_eq!(pre("expect(\"a)\").toBeT|"), ["toBeTruthy"]);
        // Inside a test callback.
        assert_eq!(
            post("test(\"x\", () => {\n  expect(res.status).toBeL|\n});"),
            ["toBeLessThan"]
        );
    }

    #[test]
    fn nothing_is_offered_in_comments() {
        assert!(pre("// req.|").is_empty());
        assert!(pre("x();\n// re|").is_empty());
        assert!(pre("/* req.| */").is_empty());
        assert!(pre("/* a\n b req.|").is_empty());
        assert!(pre("/** doc re| */").is_empty());
    }

    #[test]
    fn completion_resumes_after_a_comment_ends() {
        assert_eq!(pre("// note\nre|"), ["req"]);
        assert_eq!(pre("/* note */ re|"), ["req"]);
        assert_eq!(pre("/**/ re|"), ["req"]);
        assert_eq!(pre("a = 1; // note\nreq.ur|"), ["url"]);
    }

    #[test]
    fn a_comment_marker_inside_a_string_is_not_a_comment() {
        assert_eq!(pre("x = \"http://a\"; re|"), ["req"]);
        assert_eq!(pre("x = '/*'; re|"), ["req"]);
    }

    #[test]
    fn template_literals_offer_code_only_inside_the_dollar_braces() {
        assert!(pre("`req.|`").is_empty());
        assert!(pre("x = `hello re|").is_empty());
        assert_eq!(pre("x = `hello ${re|}`"), ["req"]);
        assert_eq!(pre("x = `a ${ {b: 1}.x + req.ur| }`"), ["url"]);
        assert_eq!(pre("x = `a ${req.url} b`; re|"), ["req"]);
        assert!(pre("x = `a ${req.url} b re|").is_empty());
        assert!(pre("x = `\\` re|").is_empty());
    }

    #[test]
    fn a_string_anywhere_else_offers_nothing() {
        assert!(pre("x = \"re|\"").is_empty());
        assert!(pre("vars.set(\"a\", \"re|\")").is_empty());
        assert!(pre("foo(\"re|\")").is_empty());
        assert!(pre("req.url = 'req.|'").is_empty());
    }

    #[test]
    fn an_unterminated_string_ends_with_its_line() {
        assert_eq!(pre("x = 'oops\nre|"), ["req"]);
    }

    fn variables() -> Vec<Hint> {
        vec![
            hint("baseUrl", Some("https://x.test"), false),
            hint("token", Some(MASK), true),
            hint("requestId", None, false),
            hint("apiKey", Some("k"), false),
        ]
    }

    fn with_variables(kind: ScriptKind, marked: &str) -> Vec<ScriptCompletion> {
        let variables = variables();
        let ctx = ScriptContext {
            variables: &variables,
            header_names: &["Accept", "Authorization", "Content-Type"],
            ..context(kind)
        };
        run(marked, &ctx, false)
    }

    #[test]
    fn variable_names_are_offered_in_vars_and_env_calls() {
        for call in ["vars.get", "vars.set", "env.get", "env.set", "env.unset"] {
            let items = with_variables(ScriptKind::Pre, &format!("{call}(\"|"));
            assert_eq!(
                labels(&items),
                ["baseUrl", "token", "requestId", "apiKey"],
                "{call}"
            );
            assert!(items.iter().all(|item| item.kind == ItemKind::Variable));
        }
        let items = with_variables(ScriptKind::Pre, "vars.get('to|')");
        assert_eq!(labels(&items), ["token"]);
        let items = with_variables(ScriptKind::Pre, "vars.get( \"ur|");
        assert_eq!(labels(&items), ["baseUrl"]);
    }

    #[test]
    fn variable_matches_rank_by_prefix_then_substring() {
        let items = with_variables(ScriptKind::Pre, "vars.get(\"a|");
        assert_eq!(labels(&items), ["apiKey", "baseUrl"]);
    }

    #[test]
    fn the_replaced_range_covers_only_what_is_typed_in_the_quotes() {
        let marked = "vars.get(\"tok|en\")";
        let (text, cursor) = cursor_in(marked);
        let variables = variables();
        let ctx = ScriptContext {
            variables: &variables,
            ..context(ScriptKind::Pre)
        };
        let items = completions(&text, cursor, &ctx, false);
        assert_eq!(items[0].label, "token");
        assert_eq!(items[0].insert, "token");
        assert_eq!(items[0].replace, 10..13);
        assert_eq!(&text[items[0].replace.clone()], "tok");
    }

    #[test]
    fn the_detail_shows_the_value_and_never_reveals_a_secret() {
        let items = with_variables(ScriptKind::Pre, "env.get(\"|");
        let detail = |name: &str| {
            items
                .iter()
                .find(|item| item.label == name)
                .and_then(|item| item.detail.clone())
        };
        assert_eq!(detail("baseUrl").as_deref(), Some("https://x.test"));
        assert_eq!(detail("token").as_deref(), Some(MASK));
        assert_eq!(detail("requestId"), None);

        let secret = [hint("password", Some("hunter2"), true)];
        let ctx = ScriptContext {
            variables: &secret,
            ..context(ScriptKind::Pre)
        };
        let items = run("vars.get(\"|", &ctx, false);
        assert_eq!(items[0].detail.as_deref(), Some(MASK));
    }

    #[test]
    fn vars_calls_also_offer_the_names_the_script_already_set() {
        let marked = "vars.set(\"fromScript\", 1);\nvars.set('token', 2);\nvars.get(\"|";
        let items = with_variables(ScriptKind::Pre, marked);
        assert_eq!(
            labels(&items),
            ["baseUrl", "token", "requestId", "apiKey", "fromScript"]
        );
        let items = with_variables(ScriptKind::Pre, "vars.set(\"fromScript\", 1);\nenv.get(\"|");
        assert_eq!(labels(&items), ["baseUrl", "token", "requestId", "apiKey"]);
        // The name being typed is not offered back to itself.
        let items = with_variables(ScriptKind::Pre, "vars.set(\"zz|");
        assert!(items.is_empty());
    }

    #[test]
    fn header_names_are_offered_in_header_calls() {
        for call in ["req.headers.get", "req.headers.set", "req.headers.remove"] {
            let items = with_variables(ScriptKind::Pre, &format!("{call}(\"|"));
            assert_eq!(labels(&items), ["Accept", "Authorization", "Content-Type"]);
            assert!(items.iter().all(|item| item.kind == ItemKind::Header));
        }
        let items = with_variables(ScriptKind::Post, "res.headers.get(\"con|");
        assert_eq!(labels(&items), ["Content-Type"]);
        let items = with_variables(ScriptKind::Pre, "req.headers.get(\"c|");
        assert_eq!(labels(&items), ["Content-Type", "Accept"]);
    }

    #[test]
    fn res_headers_are_not_offered_in_a_pre_script() {
        assert!(with_variables(ScriptKind::Pre, "res.headers.get(\"|").is_empty());
    }

    #[test]
    fn only_the_first_argument_of_those_calls_is_completed() {
        assert!(with_variables(ScriptKind::Pre, "vars.set(\"a\", \"|").is_empty());
        assert!(with_variables(ScriptKind::Pre, "vars.get(x, \"|").is_empty());
        assert!(with_variables(ScriptKind::Pre, "vars.get(f(\"|").is_empty());
        assert!(with_variables(ScriptKind::Pre, "vars.nope(\"|").is_empty());
        assert!(with_variables(ScriptKind::Pre, "x.vars.get(\"|").is_empty());
    }

    #[test]
    fn a_closed_string_does_not_complete_names() {
        assert!(with_variables(ScriptKind::Pre, "vars.get(\"a\"|").is_empty());
    }

    #[test]
    fn byte_offsets_survive_multibyte_text_before_the_cursor() {
        let ctx = context(ScriptKind::Pre);
        let (text, cursor) = cursor_in("// ñandú\nconst é = \"ü\"; req.hea|");
        let items = completions(&text, cursor, &ctx, false);
        assert_eq!(labels(&items), ["headers"]);
        assert_eq!(&text[items[0].replace.clone()], "hea");

        let variables = [hint("tókén", Some("v"), false)];
        let ctx = ScriptContext {
            variables: &variables,
            ..context(ScriptKind::Pre)
        };
        let (text, cursor) = cursor_in("vars.get(\"tók|\")");
        let items = completions(&text, cursor, &ctx, false);
        assert_eq!(labels(&items), ["tókén"]);
        assert_eq!(&text[items[0].replace.clone()], "tók");

        // A cursor inside a character offers nothing instead of panicking.
        assert!(completions("é", 1, &ctx, true).is_empty());
        assert!(completions("a", 9, &ctx, true).is_empty());
    }

    #[test]
    fn json_keys_follow_res_json() {
        let shape = shape();
        let items = post_with(&shape, "res.json().|");
        assert_eq!(labels(&items), ["data", "id", "items", "token"]);
        assert!(items.iter().all(|item| item.kind == ItemKind::JsonKey));
        let detail = |name: &str| {
            items
                .iter()
                .find(|item| item.label == name)
                .and_then(|item| item.detail.clone())
        };
        assert_eq!(detail("id").as_deref(), Some("number"));
        assert_eq!(detail("token").as_deref(), Some("string"));
        assert_eq!(detail("data").as_deref(), Some("object"));
        assert_eq!(detail("items").as_deref(), Some("array"));
        assert_eq!(labels(&post_with(&shape, "res.json().da|")), ["data"]);
        assert_eq!(
            labels(&post_with(&shape, "res.json().data.|")),
            ["a-b", "name", "tags"]
        );
    }

    #[test]
    fn json_navigation_follows_indexes_and_keys() {
        let shape = shape();
        assert_eq!(
            labels(&post_with(&shape, "res.json().items[0].|")),
            ["price", "sku", "stock"]
        );
        assert_eq!(
            labels(&post_with(&shape, "res.json()[\"data\"].|")),
            ["a-b", "name", "tags"]
        );
        assert_eq!(
            labels(&post_with(&shape, "res.json().data['name'].|")),
            Vec::<&str>::new()
        );
        assert!(post_with(&shape, "res.json().id.|").is_empty());
        assert!(post_with(&shape, "res.json().nope.|").is_empty());
        assert!(post_with(&shape, "res.json().data[0].|").is_empty());
        assert_eq!(
            labels(&post_with(&shape, "expect(res.json().items[1].|")),
            ["price", "sku", "stock"]
        );
        assert_eq!(
            labels(&post_with(
                &shape,
                "expect(res.json().data).toHaveProperty(\"x\"); res.json().i|"
            )),
            ["id", "items"]
        );
    }

    #[test]
    fn an_array_of_mixed_values_offers_the_keys_of_its_objects() {
        let shape = JsonShape::from_value(&json!({ "deep": ["a", null, { "c": "d" }, 1] }));
        assert_eq!(labels(&post_with(&shape, "res.json().deep[0].|")), ["c"]);
    }

    #[test]
    fn json_keys_need_a_response_and_a_post_script() {
        assert!(post("res.json().|").is_empty());
        let shape = shape();
        let ctx = ScriptContext {
            response: Some(&shape),
            ..context(ScriptKind::Pre)
        };
        assert!(run("res.json().|", &ctx, false).is_empty());
    }

    #[test]
    fn a_key_that_is_not_a_name_becomes_an_index_replacing_the_dot() {
        let shape = shape();
        let marked = "res.json().data.a|";
        let (text, cursor) = cursor_in(marked);
        let ctx = ScriptContext {
            response: Some(&shape),
            ..context(ScriptKind::Post)
        };
        let items = completions(&text, cursor, &ctx, false);
        let weird = items.iter().find(|item| item.label == "a-b").unwrap();
        assert_eq!(weird.insert, "[\"a-b\"]");
        assert_eq!(&text[weird.replace.clone()], ".a");
        let plain = post_with(&shape, "res.json().data.na|");
        assert_eq!(plain[0].insert, "name");

        let (text, cursor) = cursor_in("res.json()?.data?.a|");
        let items = completions(&text, cursor, &ctx, false);
        let weird = items.iter().find(|item| item.label == "a-b").unwrap();
        assert_eq!(weird.insert, ".[\"a-b\"]");
        assert_eq!(&text[weird.replace.clone()], ".a");
    }

    #[test]
    fn an_alias_of_res_json_navigates_like_it() {
        let shape = shape();
        let keys = ["data", "id", "items", "token"];
        for declaration in ["const", "let", "var"] {
            let marked = format!("{declaration} body = res.json();\nbody.|");
            assert_eq!(labels(&post_with(&shape, &marked)), keys, "{declaration}");
        }
        assert_eq!(
            labels(&post_with(&shape, "const d = res.json().data;\nd.na|")),
            ["name"]
        );
        assert_eq!(
            labels(&post_with(
                &shape,
                "const first = res.json().items[0]; first.|"
            )),
            ["price", "sku", "stock"]
        );
        assert_eq!(
            labels(&post_with(
                &shape,
                "const b = res.json();\nconst d = b.data;\nd.|"
            )),
            ["a-b", "name", "tags"]
        );
        assert_eq!(
            labels(&post_with(&shape, "const b = res.json();\nb[\"data\"].na|")),
            ["name"]
        );
        assert_eq!(
            labels(&post_with(&shape, "const b = res.json()\nexpect(b.|")),
            keys
        );
    }

    #[test]
    fn the_last_alias_before_the_cursor_wins() {
        let shape = shape();
        let marked = "let x = res.json();\nx = 1;\nlet x = res.json().data;\nx.|";
        assert_eq!(labels(&post_with(&shape, marked)), ["a-b", "name", "tags"]);
        let marked = "let x = res.json();\nlet x = 5;\nx.|";
        assert!(post_with(&shape, marked).is_empty());
        // A declaration after the cursor does not count.
        let marked = "x.|\nlet x = res.json();";
        assert!(post_with(&shape, marked).is_empty());
    }

    #[test]
    fn an_alias_in_a_comment_or_of_something_else_is_ignored() {
        let shape = shape();
        assert!(post_with(&shape, "// const b = res.json();\nb.|").is_empty());
        assert!(post_with(&shape, "/* var b = res.json(); */ b.|").is_empty());
        assert!(post_with(&shape, "const b = other();\nb.|").is_empty());
        assert!(post_with(&shape, "const b == res.json();\nb.|").is_empty());
        assert!(post_with(&shape, "const b = (res.json());\nb.|").is_empty());
        assert!(post_with(&shape, "xconst b = res.json();\nb.|").is_empty());
    }

    #[test]
    fn an_alias_also_works_for_the_api_objects() {
        assert_eq!(pre("const h = req.headers;\nh.|"), ["get", "set", "remove"]);
        assert_eq!(
            pre("const check = expect(1);\ncheck.toBeT|"),
            ["toBeTruthy"]
        );
    }

    #[test]
    fn typing_triggers_on_names_dots_and_quotes() {
        for typed in ["a", "Z", "_", "$", "7", "é", ".", "\"", "'", "`"] {
            assert!(is_trigger(typed), "{typed}");
        }
        for typed in ["", " ", "\n", "(", ")", ";", "=", ",", "{", "a b "] {
            assert!(!is_trigger(typed), "{typed:?}");
        }
        assert!(is_trigger("req."));
    }

    #[test]
    fn a_shape_records_types_and_merges_array_elements() {
        let shape = JsonShape::from_value(&json!({
            "a": null, "b": true, "c": 1, "d": "s",
            "e": [ { "x": null, "y": 1 }, { "x": "now text", "z": [] } ],
            "f": [],
        }));
        assert_eq!(shape.ty(), JsonType::Object);
        let ty = |name: &str| shape.field(name).map(JsonShape::ty);
        assert_eq!(ty("a"), Some(JsonType::Null));
        assert_eq!(ty("b"), Some(JsonType::Boolean));
        assert_eq!(ty("c"), Some(JsonType::Number));
        assert_eq!(ty("d"), Some(JsonType::String));
        assert_eq!(ty("e"), Some(JsonType::Array));
        let element = shape.field("e").and_then(JsonShape::element).unwrap();
        let keys: Vec<&str> = element.fields().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["x", "y", "z"]);
        assert_eq!(
            element.field("x").map(JsonShape::ty),
            Some(JsonType::String)
        );
        assert!(shape.field("f").and_then(JsonShape::element).is_none());
        assert!(shape.field("nope").is_none());
    }

    #[test]
    fn a_shape_is_built_from_a_body_text() {
        assert!(JsonShape::from_body("{\"a\": 1}").is_some());
        assert!(JsonShape::from_body("[1, 2]").is_some());
        assert!(JsonShape::from_body("not json").is_none());
        assert!(JsonShape::from_body("").is_none());
        let huge = format!("[{}0]", "0,".repeat(MAX_JSON_BODY_BYTES / 2));
        assert!(JsonShape::from_body(&huge).is_none());
    }

    #[test]
    fn a_streamed_shape_equals_the_shape_of_the_parsed_value() {
        let same = |text: &str| {
            let value: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(
                JsonShape::from_body(text),
                Some(JsonShape::from_value(&value)),
                "{}",
                &text[..text.len().min(80)]
            );
        };
        same(r#"{"a": {"b": {"c": [1, 2]}, "d": null}, "e": "x", "f": true, "g": 1.5}"#);
        same(r#"["a", null, {"c": "d"}, 1]"#);
        same(r#"[null, [1], {"a": 1}, {"b": [{"x": null}, {"x": {"y": 1}}]}]"#);
        same(r#"{"a": 1, "a": {"b": 2}, "c": 3}"#);
        same("1");
        same("null");
        same("{}");
        same("[]");
        let long: Vec<String> = (0..100)
            .map(|i| format!("{{\"only{i}\": [{i}]}}"))
            .collect();
        same(&format!("[{}]", long.join(",")));
        same(&format!(
            "[{}{{\"late\": 1}}]",
            "1,".repeat(MAX_JSON_ARRAY_ELEMENTS + 5)
        ));
        let deep = format!("{}1{}", "{\"n\": [".repeat(30), "]}".repeat(30));
        same(&deep);
        // Keys in sorted order, so the budget is spent in the same order as `from_value`.
        let wide: Vec<String> = (0..20_000)
            .map(|i| format!("\"k{i:06}\": [{i}, {i}]"))
            .collect();
        same(&format!("{{{}}}", wide.join(",")));
        let rows: Vec<String> = (0..9_000)
            .map(|i| format!("{{\"a\": {i}, \"b\": {{\"c\": [1, 2, 3]}}}}"))
            .collect();
        same(&format!("[{}]", rows.join(",")));

        for bad in ["{\"a\": 1} x", "[1, 2", "{\"a\": }", "[1] [2]", "{\"a\" 1}"] {
            assert!(JsonShape::from_body(bad).is_none(), "{bad}");
        }
        let deep_bad = format!("{}1{}", "[".repeat(30), "]".repeat(29));
        assert!(JsonShape::from_body(&deep_bad).is_none());
        let past_budget = format!("[{}0, ", "{\"a\": 1},".repeat(MAX_JSON_NODES));
        assert!(JsonShape::from_body(&past_budget).is_none());
    }

    #[test]
    fn a_shape_is_bounded_in_depth() {
        let mut value = json!({ "leaf": 1 });
        for _ in 0..30 {
            value = json!({ "next": value });
        }
        let shape = JsonShape::from_value(&value);
        let mut depth = 0;
        let mut at = &shape;
        while let Some(next) = at.field("next") {
            depth += 1;
            at = next;
        }
        assert_eq!(depth, MAX_JSON_DEPTH);
    }

    #[test]
    fn a_shape_is_bounded_in_nodes_and_array_elements() {
        let many: Vec<serde_json::Value> = (0..200_000)
            .map(|i| json!({ "id": i, "name": "n", "nested": { "a": 1, "b": [1, 2, 3] } }))
            .collect();
        let shape = JsonShape::from_value(&serde_json::Value::Array(many));
        assert!(shape.node_count() <= MAX_JSON_NODES);
        let element = shape.element().unwrap();
        assert_eq!(element.fields().len(), 3);

        let wide: serde_json::Map<String, serde_json::Value> =
            (0..100_000).map(|i| (format!("k{i}"), json!(i))).collect();
        let shape = JsonShape::from_value(&serde_json::Value::Object(wide));
        assert!(shape.node_count() <= MAX_JSON_NODES);
        assert!(shape.fields().len() < MAX_JSON_NODES);

        let long: Vec<serde_json::Value> =
            (0..100).map(|i| json!({ format!("only{i}"): i })).collect();
        let shape = JsonShape::from_value(&serde_json::Value::Array(long));
        assert_eq!(
            shape.element().unwrap().fields().len(),
            MAX_JSON_ARRAY_ELEMENTS
        );
    }
}
