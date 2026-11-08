//! The built-in template functions (see `docs/format.md`): `uuid()`, `now()`,
//! `isoDate()`, `randomInt(min, max)`, `randomString(len)`, `base64Encode(str)`,
//! `base64Decode(str)`, `urlEncode(str)`.
//!
//! Each function is a plain Rust function, reusable as is from `brasa-script` (as `util.*`),
//! plus a name-based [`call`] dispatcher used by [`crate::interpolate`] to evaluate a
//! `{{ name(arg, ...) }}` marker.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

/// A single, already-resolved argument to a template function call.
///
/// Produced by the `{{ }}` marker parser in [`crate::interpolate`]: a double-quoted string
/// literal or a variable becomes [`Arg::Str`], an integer literal becomes [`Arg::Int`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    /// A string value.
    Str(String),
    /// An integer value.
    Int(i64),
}

/// An error evaluating a template function call.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FunctionError {
    /// No built-in function has this name.
    #[error("unknown function `{0}`")]
    UnknownFunction(String),
    /// The call has the wrong number of arguments.
    #[error("`{function}` expects {expected} argument(s), got {got}")]
    Arity {
        /// The function name.
        function: String,
        /// The number of arguments the function requires.
        expected: usize,
        /// The number of arguments actually given.
        got: usize,
    },
    /// An argument has the wrong type or an invalid value.
    #[error("invalid argument for `{function}`: {message}")]
    InvalidArgument {
        /// The function name.
        function: String,
        /// A human readable description of what is wrong.
        message: String,
    },
}

/// Evaluates a built-in function call by name, given its already-resolved arguments.
///
/// This is the dispatcher used by `{{ name(arg, ...) }}` markers. It never panics: an unknown
/// function, a wrong number of arguments or an argument of the wrong type all return a
/// [`FunctionError`], which `brasa-core::interpolate` turns into a warning rather than an error.
pub fn call(name: &str, args: &[Arg]) -> Result<String, FunctionError> {
    match name {
        "uuid" => {
            expect_arity(name, args, 0)?;
            Ok(uuid())
        }
        "now" => {
            expect_arity(name, args, 0)?;
            Ok(now_millis().to_string())
        }
        "isoDate" => {
            expect_arity(name, args, 0)?;
            Ok(iso_date())
        }
        "randomInt" => {
            expect_arity(name, args, 2)?;
            let min = expect_int(name, args, 0)?;
            let max = expect_int(name, args, 1)?;
            Ok(random_int(min, max).to_string())
        }
        "randomString" => {
            expect_arity(name, args, 1)?;
            let len = expect_int(name, args, 0)?;
            let len = usize::try_from(len)
                .ok()
                .filter(|&len| len <= MAX_RANDOM_STRING_LEN)
                .ok_or_else(|| FunctionError::InvalidArgument {
                    function: name.to_string(),
                    message: format!("length must be between 0 and {MAX_RANDOM_STRING_LEN}"),
                })?;
            Ok(random_string(len))
        }
        "base64Encode" => {
            expect_arity(name, args, 1)?;
            let value = expect_str(name, args, 0)?;
            Ok(base64_encode(value))
        }
        "base64Decode" => {
            expect_arity(name, args, 1)?;
            let value = expect_str(name, args, 0)?;
            base64_decode(value)
        }
        "urlEncode" => {
            expect_arity(name, args, 1)?;
            let value = expect_str(name, args, 0)?;
            Ok(url_encode(value))
        }
        other => Err(FunctionError::UnknownFunction(other.to_string())),
    }
}

/// Checks that `args` has exactly `expected` elements.
fn expect_arity(name: &str, args: &[Arg], expected: usize) -> Result<(), FunctionError> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(FunctionError::Arity {
            function: name.to_string(),
            expected,
            got: args.len(),
        })
    }
}

/// Reads the argument at `index` as an integer, or reports why it could not be used as one.
fn expect_int(name: &str, args: &[Arg], index: usize) -> Result<i64, FunctionError> {
    match &args[index] {
        Arg::Int(value) => Ok(*value),
        Arg::Str(value) => Err(FunctionError::InvalidArgument {
            function: name.to_string(),
            message: format!("expected an integer, got the string \"{value}\""),
        }),
    }
}

/// Reads the argument at `index` as a string.
fn expect_str<'a>(name: &str, args: &'a [Arg], index: usize) -> Result<&'a str, FunctionError> {
    match &args[index] {
        Arg::Str(value) => Ok(value),
        Arg::Int(value) => Err(FunctionError::InvalidArgument {
            function: name.to_string(),
            message: format!("expected a string, got the integer {value}"),
        }),
    }
}

/// Generates a random UUID v4, for example `"5f8c1e2a-9b3d-4e11-8f2a-0b6a1c2d3e4f"`.
pub fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The current time as milliseconds since the Unix epoch.
///
/// Returns `0` in the practically impossible case that the system clock is set before 1970,
/// rather than panicking.
pub fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// The current UTC time as an RFC 3339 timestamp, for example `"2026-09-25T13:11:00Z"`.
///
/// Implemented by hand with a small, well known civil calendar algorithm (see
/// [`civil_from_days`]) instead of adding a date and time dependency.
pub fn iso_date() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_seconds = now.as_secs() as i64;
    let days = total_seconds.div_euclid(86_400);
    let seconds_of_day = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Converts a day count since the Unix epoch (1970-01-01) into a proleptic Gregorian
/// `(year, month, day)` civil date.
///
/// This is Howard Hinnant's `civil_from_days` algorithm
/// (<https://howardhinnant.github.io/date_algorithms.html>), in the public domain. It is valid
/// for every day count representable by `i64` and avoids pulling in a date and time crate for a
/// single conversion.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// A random integer in the inclusive range `[min, max]`.
///
/// If `min` is greater than `max`, the two bounds are swapped rather than treating it as an
/// error, so the function always succeeds.
pub fn random_int(min: i64, max: i64) -> i64 {
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    fastrand::i64(min..=max)
}

/// The longest string `randomString` accepts, so a typo such as `randomString(1e12)` fails
/// instead of exhausting memory.
pub const MAX_RANDOM_STRING_LEN: usize = 1_048_576;

/// The alphanumeric alphabet used by [`random_string`].
const ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// A random alphanumeric string of the given length.
pub fn random_string(len: usize) -> String {
    (0..len)
        .map(|_| {
            let index = fastrand::usize(0..ALPHANUMERIC.len());
            ALPHANUMERIC[index] as char
        })
        .collect()
}

/// Encodes a string as standard base64.
pub fn base64_encode(input: &str) -> String {
    BASE64.encode(input.as_bytes())
}

/// Decodes a standard base64 string back to text.
///
/// Fails if `input` is not valid base64, or if the decoded bytes are not valid UTF-8.
pub fn base64_decode(input: &str) -> Result<String, FunctionError> {
    let bytes = BASE64
        .decode(input)
        .map_err(|error| FunctionError::InvalidArgument {
            function: "base64Decode".to_string(),
            message: format!("not valid base64: {error}"),
        })?;
    String::from_utf8(bytes).map_err(|error| FunctionError::InvalidArgument {
        function: "base64Decode".to_string(),
        message: format!("decoded bytes are not valid UTF-8: {error}"),
    })
}

/// Percent-encodes a string for safe use in a URL, leaving only the RFC 3986 unreserved
/// characters (`A-Z a-z 0-9 - . _ ~`) unescaped.
pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn uuid_looks_like_a_v4_uuid() {
        let value = uuid();
        let parsed: uuid::Uuid = value.parse().expect("uuid() must produce a valid UUID");
        assert_eq!(parsed.get_version_num(), 4);
    }

    #[test]
    fn uuid_calls_are_independent() {
        assert_ne!(uuid(), uuid());
    }

    #[test]
    fn now_millis_is_plausible() {
        // Any timestamp after 2026-01-01T00:00:00Z, in milliseconds.
        assert!(now_millis() > 1_767_225_600_000);
    }

    #[test]
    fn iso_date_matches_rfc3339_shape() {
        let value = iso_date();
        assert_eq!(value.len(), 20);
        assert_eq!(value.as_bytes()[4], b'-');
        assert_eq!(value.as_bytes()[7], b'-');
        assert_eq!(value.as_bytes()[10], b'T');
        assert_eq!(value.as_bytes()[13], b':');
        assert_eq!(value.as_bytes()[16], b':');
        assert_eq!(value.as_bytes()[19], b'Z');
        assert!(value.starts_with("202"));
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(19_601), (2023, 9, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29)); // leap day
    }

    #[test]
    fn random_int_stays_in_bounds() {
        for _ in 0..200 {
            let value = random_int(5, 10);
            assert!((5..=10).contains(&value), "{value} out of bounds");
        }
    }

    #[test]
    fn random_int_accepts_a_single_value_range() {
        assert_eq!(random_int(7, 7), 7);
    }

    #[test]
    fn random_int_swaps_inverted_bounds() {
        for _ in 0..50 {
            let value = random_int(10, 5);
            assert!((5..=10).contains(&value));
        }
    }

    #[test]
    fn random_string_has_the_requested_length_and_alphabet() {
        let value = random_string(24);
        assert_eq!(value.chars().count(), 24);
        assert!(value.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn random_string_of_zero_length_is_empty() {
        assert_eq!(random_string(0), "");
    }

    #[test]
    fn base64_round_trips() {
        let encoded = base64_encode("user:pass");
        assert_eq!(encoded, "dXNlcjpwYXNz");
        assert_eq!(base64_decode(&encoded), Ok("user:pass".to_string()));
    }

    #[test]
    fn base64_decode_rejects_invalid_input() {
        assert!(base64_decode("not base64!!").is_err());
    }

    #[test]
    fn url_encode_escapes_reserved_characters() {
        assert_eq!(url_encode("a b/c"), "a%20b%2Fc");
        assert_eq!(url_encode("safe-._~123ABC"), "safe-._~123ABC");
    }

    #[test]
    fn call_dispatches_uuid() {
        let result = call("uuid", &[]).expect("uuid() takes no arguments");
        assert!(result.parse::<uuid::Uuid>().is_ok());
    }

    #[test]
    fn call_dispatches_random_int_with_int_args() {
        let result = call("randomInt", &[Arg::Int(1), Arg::Int(1)]).expect("valid call");
        assert_eq!(result, "1");
    }

    #[test]
    fn call_dispatches_base64_encode_with_variable_argument() {
        let result = call("base64Encode", &[Arg::Str("hi".to_string())]).expect("valid call");
        assert_eq!(result, base64_encode("hi"));
    }

    #[test]
    fn call_reports_unknown_function() {
        assert_eq!(
            call("doesNotExist", &[]),
            Err(FunctionError::UnknownFunction("doesNotExist".to_string()))
        );
    }

    #[test]
    fn call_reports_wrong_arity() {
        assert_eq!(
            call("uuid", &[Arg::Int(1)]),
            Err(FunctionError::Arity {
                function: "uuid".to_string(),
                expected: 0,
                got: 1
            })
        );
    }

    #[test]
    fn call_reports_invalid_argument_type() {
        assert!(matches!(
            call("randomInt", &[Arg::Str("a".to_string()), Arg::Int(1)]),
            Err(FunctionError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn call_reports_negative_random_string_length() {
        assert!(matches!(
            call("randomString", &[Arg::Int(-1)]),
            Err(FunctionError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn call_limits_the_random_string_length() {
        let max = MAX_RANDOM_STRING_LEN as i64;
        assert_eq!(
            call("randomString", &[Arg::Int(max)])
                .expect("at the limit")
                .len(),
            MAX_RANDOM_STRING_LEN
        );
        assert!(matches!(
            call("randomString", &[Arg::Int(max + 1)]),
            Err(FunctionError::InvalidArgument { .. })
        ));
        assert!(matches!(
            call("randomString", &[Arg::Int(i64::MAX)]),
            Err(FunctionError::InvalidArgument { .. })
        ));
    }
}
