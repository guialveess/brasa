//! A static description of every global and member the script sandbox exposes (`req`, `res`,
//! `vars`, `env`, `test`, `expect`, `console`, `util`), for tooling such as the autocompletion of
//! the script editors. It is data only: nothing here runs a script.
//!
//! The schema must stay in step with `prelude.js` and the native `util` bindings. The tests of
//! this module run the real sandbox and compare the own property names of every documented
//! object with the schema, so a global or member added on either side without the other fails
//! `cargo test`. The descriptions follow `docs/scripting.md`.
//!
//! Reading the tree: an [`ApiEntry`] of kind [`MemberKind::Object`] or [`MemberKind::Property`]
//! has its `members` right after a dot (`req.headers.`). A [`MemberKind::Function`] has them
//! only after a call (`expect(x).`), since they are the members of its result.

/// Where a global exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// In `::: pre` and `::: post` scripts.
    Both,
    /// Only in `::: pre` scripts.
    PreOnly,
    /// Only in `::: post` scripts (there is no response before the request is sent).
    PostOnly,
}

impl Availability {
    /// Whether the entry exists in a post script (`post` true) or in a pre script (`post`
    /// false).
    pub fn available_in(self, post: bool) -> bool {
        match self {
            Self::Both => true,
            Self::PreOnly => !post,
            Self::PostOnly => post,
        }
    }
}

/// What an entry is, which decides how an editor presents and inserts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    /// A value that has members of its own, such as `req.headers`.
    Object,
    /// A callable, such as `vars.set(name, value)`.
    Function,
    /// A plain value, such as `res.status`.
    Property,
}

/// What the call of a function evaluates to, when that is more than its `members` say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Returns {
    /// Nothing worth completing: a string, a number, `undefined`, or an object described by the
    /// entry's `members`.
    Unknown,
    /// The parsed JSON of the response body (`res.json()`). Its keys are only known from the
    /// actual response, so tooling resolves them from the last body.
    Json,
}

/// One global or member of the sandbox.
#[derive(Debug, Clone, Copy)]
pub struct ApiEntry {
    /// The name as written in a script.
    pub name: &'static str,
    /// Whether it is an object, a function or a plain property.
    pub kind: MemberKind,
    /// For a function, how it is called, such as `set(name, value)`; `None` otherwise.
    pub signature: Option<&'static str>,
    /// For a property, the type of its value, such as `number`; `None` otherwise.
    pub ty: Option<&'static str>,
    /// One or two English sentences describing it.
    pub doc: &'static str,
    /// Where it exists. Only meaningful for a global; members are listed with the availability
    /// of their parent.
    pub availability: Availability,
    /// The members after a dot (after a call, for a function). Empty when there are none.
    pub members: &'static [ApiEntry],
    /// What calling a function evaluates to.
    pub returns: Returns,
}

impl ApiEntry {
    const fn new(name: &'static str, kind: MemberKind, doc: &'static str) -> Self {
        Self {
            name,
            kind,
            signature: None,
            ty: None,
            doc,
            availability: Availability::Both,
            members: &[],
            returns: Returns::Unknown,
        }
    }

    const fn object(name: &'static str, doc: &'static str, members: &'static [ApiEntry]) -> Self {
        let mut entry = Self::new(name, MemberKind::Object, doc);
        entry.members = members;
        entry
    }

    const fn function(name: &'static str, signature: &'static str, doc: &'static str) -> Self {
        let mut entry = Self::new(name, MemberKind::Function, doc);
        entry.signature = Some(signature);
        entry
    }

    const fn property(name: &'static str, ty: &'static str, doc: &'static str) -> Self {
        let mut entry = Self::new(name, MemberKind::Property, doc);
        entry.ty = Some(ty);
        entry
    }

    const fn post_only(mut self) -> Self {
        self.availability = Availability::PostOnly;
        self
    }

    const fn result_members(mut self, members: &'static [ApiEntry]) -> Self {
        self.members = members;
        self
    }

    const fn returning_json(mut self) -> Self {
        self.returns = Returns::Json;
        self
    }

    /// The member called `name`, `None` when there is none.
    pub fn member(&self, name: &str) -> Option<&'static ApiEntry> {
        self.members.iter().find(|member| member.name == name)
    }

    /// Whether calling this function takes no argument, judged from its signature.
    pub fn takes_no_arguments(&self) -> bool {
        self.signature
            .is_some_and(|signature| signature.ends_with("()"))
    }
}

/// The global called `name` that exists in a post script (`post` true) or a pre script.
pub fn global(name: &str, post: bool) -> Option<&'static ApiEntry> {
    GLOBALS
        .iter()
        .find(|entry| entry.name == name && entry.availability.available_in(post))
}

const REQ_HEADERS_MEMBERS: &[ApiEntry] = &[
    ApiEntry::function(
        "get",
        "get(name)",
        "Returns the value of the header, matched case insensitively, or undefined.",
    ),
    ApiEntry::function(
        "set",
        "set(name, value)",
        "Sets a header, keeping the spelling of an existing one. Throws in a post script.",
    ),
    ApiEntry::function(
        "remove",
        "remove(name)",
        "Removes the header, matched case insensitively. Throws in a post script.",
    ),
];

const REQ_MEMBERS: &[ApiEntry] = &[
    ApiEntry::property(
        "method",
        "string",
        "The HTTP method. Writable in a pre script, read-only in a post script.",
    ),
    ApiEntry::property(
        "url",
        "string",
        "The URL. Writable in a pre script, read-only in a post script.",
    ),
    ApiEntry::property(
        "body",
        "string",
        "The body as text, empty for a request without one. Writable in a pre script, read-only \
         in a post script.",
    ),
    ApiEntry::object(
        "headers",
        "The enabled request headers. Read-only in a post script.",
        REQ_HEADERS_MEMBERS,
    ),
];

const RES_HEADERS_MEMBERS: &[ApiEntry] = &[ApiEntry::function(
    "get",
    "get(name)",
    "Returns the value of the response header, matched case insensitively, or undefined.",
)];

const RES_MEMBERS: &[ApiEntry] = &[
    ApiEntry::property("status", "number", "The HTTP status code of the response."),
    ApiEntry::object(
        "headers",
        "The response headers, read-only.",
        RES_HEADERS_MEMBERS,
    ),
    ApiEntry::property(
        "body",
        "string",
        "The response body as text, decoded as UTF-8.",
    ),
    ApiEntry::function(
        "json",
        "json()",
        "Parses the response body as JSON. Throws if it is not valid JSON.",
    )
    .returning_json(),
    ApiEntry::property(
        "timeMs",
        "number",
        "The total time of the request in milliseconds.",
    ),
    ApiEntry::property("size", "number", "The size of the response body in bytes."),
];

const VARS_MEMBERS: &[ApiEntry] = &[
    ApiEntry::function(
        "get",
        "get(name)",
        "Returns the request variable called name, or undefined.",
    ),
    ApiEntry::function(
        "set",
        "set(name, value)",
        "Sets a request variable, converting the value to a string. It lives for this run only.",
    ),
];

const ENV_MEMBERS: &[ApiEntry] = &[
    ApiEntry::function(
        "get",
        "get(name)",
        "Returns the value of the environment variable, or undefined.",
    ),
    ApiEntry::function(
        "set",
        "set(name, value)",
        "Overrides an environment variable for the rest of the app session, never written to the \
         .env file.",
    ),
    ApiEntry::function(
        "unset",
        "unset(name)",
        "Removes an environment variable for the rest of the app session.",
    ),
];

const CONSOLE_MEMBERS: &[ApiEntry] = &[
    ApiEntry::function(
        "log",
        "log(...values)",
        "Adds a line to the Console tab, joining the values with a space.",
    ),
    ApiEntry::function(
        "info",
        "info(...values)",
        "Adds an info line to the Console tab.",
    ),
    ApiEntry::function(
        "warn",
        "warn(...values)",
        "Adds a warning line to the Console tab.",
    ),
    ApiEntry::function(
        "error",
        "error(...values)",
        "Adds an error line to the Console tab.",
    ),
];

const UTIL_MEMBERS: &[ApiEntry] = &[
    ApiEntry::function("uuid", "uuid()", "Returns a random UUID v4 as a string."),
    ApiEntry::function(
        "now",
        "now()",
        "Returns the current Unix time in milliseconds as a number.",
    ),
    ApiEntry::function(
        "isoDate",
        "isoDate()",
        "Returns the current UTC date and time in ISO 8601 format.",
    ),
    ApiEntry::function(
        "randomInt",
        "randomInt(min, max)",
        "Returns a random integer between min and max as a number.",
    ),
    ApiEntry::function(
        "randomString",
        "randomString(length)",
        "Returns a random alphanumeric string of the given length, at most 1,048,576 characters.",
    ),
    ApiEntry::function(
        "base64Encode",
        "base64Encode(text)",
        "Returns the Base64 encoding of the text.",
    ),
    ApiEntry::function(
        "base64Decode",
        "base64Decode(text)",
        "Returns the text a Base64 string encodes.",
    ),
    ApiEntry::function(
        "urlEncode",
        "urlEncode(text)",
        "Returns the text percent-encoded for use in a URL.",
    ),
];

const TO_BE: ApiEntry = ApiEntry::function(
    "toBe",
    "toBe(expected)",
    "Passes when the value is strictly equal (===) to expected.",
);
const TO_EQUAL: ApiEntry = ApiEntry::function(
    "toEqual",
    "toEqual(expected)",
    "Passes when the value is deeply equal to expected, compared through JSON.stringify.",
);
const TO_BE_TRUTHY: ApiEntry = ApiEntry::function(
    "toBeTruthy",
    "toBeTruthy()",
    "Passes when the value is truthy.",
);
const TO_BE_FALSY: ApiEntry = ApiEntry::function(
    "toBeFalsy",
    "toBeFalsy()",
    "Passes when the value is falsy.",
);
const TO_CONTAIN: ApiEntry = ApiEntry::function(
    "toContain",
    "toContain(item)",
    "Passes when the string or array value contains item. Throws for any other value.",
);
const TO_MATCH: ApiEntry = ApiEntry::function(
    "toMatch",
    "toMatch(pattern)",
    "Passes when the value, as a string, matches the RegExp or the pattern string.",
);
const TO_BE_GREATER_THAN: ApiEntry = ApiEntry::function(
    "toBeGreaterThan",
    "toBeGreaterThan(other)",
    "Passes when the value is greater than other.",
);
const TO_BE_LESS_THAN: ApiEntry = ApiEntry::function(
    "toBeLessThan",
    "toBeLessThan(other)",
    "Passes when the value is less than other.",
);
const TO_HAVE_PROPERTY: ApiEntry = ApiEntry::function(
    "toHaveProperty",
    "toHaveProperty(name)",
    "Passes when the value has an own property called name.",
);

/// The matchers behind `.not`: the same nine, without a further `not`.
const NEGATED_MATCHERS: &[ApiEntry] = &[
    TO_BE,
    TO_EQUAL,
    TO_BE_TRUTHY,
    TO_BE_FALSY,
    TO_CONTAIN,
    TO_MATCH,
    TO_BE_GREATER_THAN,
    TO_BE_LESS_THAN,
    TO_HAVE_PROPERTY,
];

const EXPECT_MEMBERS: &[ApiEntry] = &[
    TO_BE,
    TO_EQUAL,
    TO_BE_TRUTHY,
    TO_BE_FALSY,
    TO_CONTAIN,
    TO_MATCH,
    TO_BE_GREATER_THAN,
    TO_BE_LESS_THAN,
    TO_HAVE_PROPERTY,
    ApiEntry::object(
        "not",
        "Inverts the matcher that follows: it passes exactly when the plain one would fail.",
        NEGATED_MATCHERS,
    ),
];

/// Every global of the sandbox, in the order tooling lists them.
pub static GLOBALS: &[ApiEntry] = &[
    ApiEntry::object(
        "req",
        "The request about to be sent in a pre script, or as it was sent in a post script, where \
         it is read-only.",
        REQ_MEMBERS,
    ),
    ApiEntry::object(
        "res",
        "The response that was received. It only exists in a post script.",
        RES_MEMBERS,
    )
    .post_only(),
    ApiEntry::object(
        "vars",
        "Variables of this run, set by the pre script. They have the highest priority when a \
         {{ }} marker is resolved.",
        VARS_MEMBERS,
    ),
    ApiEntry::object(
        "env",
        "Session overrides of the active environment, kept until the app is closed.",
        ENV_MEMBERS,
    ),
    ApiEntry::function(
        "test",
        "test(name, fn)",
        "Runs fn right away and records a passed or failed test. A failure does not stop the \
         script.",
    ),
    ApiEntry::function(
        "expect",
        "expect(value)",
        "Starts an assertion on value; chain a matcher such as toBe(...) to check it.",
    )
    .result_members(EXPECT_MEMBERS),
    ApiEntry::object(
        "console",
        "Writes lines to the Console tab of the response.",
        CONSOLE_MEMBERS,
    ),
    ApiEntry::object(
        "util",
        "The built-in functions, the same ones available inside {{ }} markers.",
        UTIL_MEMBERS,
    ),
];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::quickjs_engine::{baseline_global_names, with_sandbox};
    use pretty_assertions::assert_eq;
    use rquickjs::Ctx;
    use std::collections::BTreeSet;

    fn names_of(ctx: &Ctx<'_>, expression: &str) -> BTreeSet<String> {
        let json: String = ctx
            .eval(format!(
                "JSON.stringify(Object.getOwnPropertyNames({expression}))"
            ))
            .unwrap_or_else(|error| panic!("could not enumerate `{expression}`: {error}"));
        serde_json::from_str::<Vec<String>>(&json)
            .unwrap()
            .into_iter()
            .collect()
    }

    fn assert_same(what: &str, actual: BTreeSet<String>, expected: BTreeSet<String>) {
        let missing: Vec<_> = expected.difference(&actual).collect();
        let extra: Vec<_> = actual.difference(&expected).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "api schema out of step with the sandbox for {what}: in the schema but not in the \
             sandbox: {missing:?}; in the sandbox but not in the schema: {extra:?}. Update \
             crates/brasa-script/src/api.rs."
        );
    }

    /// Members the sandbox has on purpose that the schema leaves out because offering them would
    /// only be noise: `res.headers` shares its implementation with `req.headers`, so it has
    /// `set` and `remove`, which always throw, and `.not` can be chained again.
    const UNLISTED: &[(&str, &str)] = &[
        ("res.headers", "set"),
        ("res.headers", "remove"),
        ("expect(1).not", "not"),
    ];

    /// Compares the members of `entry`, reached with `expression`, then recurses.
    fn check_entry(ctx: &Ctx<'_>, expression: &str, entry: &ApiEntry) {
        let type_of: String = ctx.eval(format!("typeof ({expression})")).unwrap();
        match entry.kind {
            MemberKind::Function => assert_eq!(type_of, "function", "{expression}"),
            MemberKind::Object => assert_eq!(type_of, "object", "{expression}"),
            MemberKind::Property => assert_ne!(type_of, "function", "{expression}"),
        }
        if entry.members.is_empty() {
            return;
        }
        let target = if entry.kind == MemberKind::Function {
            format!("{expression}(1)")
        } else {
            expression.to_string()
        };
        let expected = entry.members.iter().map(|m| m.name.to_string()).collect();
        let actual = names_of(ctx, &target)
            .into_iter()
            .filter(|name| !UNLISTED.contains(&(target.as_str(), name.as_str())))
            .collect();
        assert_same(&target, actual, expected);
        for member in entry.members {
            check_entry(ctx, &format!("{target}.{}", member.name), member);
        }
    }

    fn check_context(post: bool) {
        let baseline = baseline_global_names();
        with_sandbox(post, |ctx| {
            let actual: BTreeSet<String> = names_of(ctx, "globalThis")
                .into_iter()
                .filter(|name| !baseline.contains(name) && !name.starts_with("__"))
                .collect();
            let expected: BTreeSet<String> = GLOBALS
                .iter()
                .filter(|entry| entry.availability.available_in(post))
                .map(|entry| entry.name.to_string())
                .collect();
            let kind = if post { "post" } else { "pre" };
            assert_same(&format!("the globals of a {kind} script"), actual, expected);
            for entry in GLOBALS
                .iter()
                .filter(|entry| entry.availability.available_in(post))
            {
                check_entry(ctx, entry.name, entry);
            }
        });
    }

    #[test]
    fn the_schema_matches_the_pre_sandbox() {
        check_context(false);
    }

    #[test]
    fn the_schema_matches_the_post_sandbox() {
        check_context(true);
    }

    #[test]
    fn res_exists_only_in_a_post_script() {
        assert!(global("res", true).is_some());
        assert!(global("res", false).is_none());
        assert!(global("req", false).is_some());
    }

    #[test]
    fn json_is_the_only_function_returning_json() {
        let res = global("res", true).unwrap();
        assert_eq!(res.member("json").unwrap().returns, Returns::Json);
        assert!(res.member("json").unwrap().takes_no_arguments());
        assert!(
            !global("vars", false)
                .unwrap()
                .member("set")
                .unwrap()
                .takes_no_arguments()
        );
    }

    #[test]
    fn every_entry_is_documented_in_english_without_em_dashes() {
        fn walk(entry: &ApiEntry) {
            assert!(!entry.doc.is_empty(), "{}", entry.name);
            assert!(!entry.doc.contains('\u{2014}'), "{}", entry.name);
            if entry.kind == MemberKind::Function {
                assert!(entry.signature.is_some(), "{}", entry.name);
            }
            entry.members.iter().for_each(walk);
        }
        GLOBALS.iter().for_each(walk);
    }
}
