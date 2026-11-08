//! The HTTP method of a request.

use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;

/// The HTTP method of a [`crate::Request`].
///
/// The common methods are individual variants. Any other token, uppercase by convention, is kept
/// verbatim in [`Method::Custom`] so a request file can use a nonstandard method without failing to
/// parse.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
    /// `HEAD`
    Head,
    /// `OPTIONS`
    Options,
    /// Any other method, kept exactly as written.
    Custom(String),
}

impl Default for Method {
    /// The default method for a new request, `GET`.
    fn default() -> Self {
        Method::Get
    }
}

impl FromStr for Method {
    type Err = Infallible;

    /// Parses a method token. This never fails: an unrecognized token becomes
    /// [`Method::Custom`], keeping it exactly as given.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "PATCH" => Method::Patch,
            "DELETE" => Method::Delete,
            "HEAD" => Method::Head,
            "OPTIONS" => Method::Options,
            other => Method::Custom(other.to_string()),
        })
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Method::Get => write!(f, "GET"),
            Method::Post => write!(f, "POST"),
            Method::Put => write!(f, "PUT"),
            Method::Patch => write!(f, "PATCH"),
            Method::Delete => write!(f, "DELETE"),
            Method::Head => write!(f, "HEAD"),
            Method::Options => write!(f, "OPTIONS"),
            Method::Custom(name) => write!(f, "{name}"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_standard_methods() {
        assert_eq!("GET".parse(), Ok(Method::Get));
        assert_eq!("POST".parse(), Ok(Method::Post));
        assert_eq!("PUT".parse(), Ok(Method::Put));
        assert_eq!("PATCH".parse(), Ok(Method::Patch));
        assert_eq!("DELETE".parse(), Ok(Method::Delete));
        assert_eq!("HEAD".parse(), Ok(Method::Head));
        assert_eq!("OPTIONS".parse(), Ok(Method::Options));
    }

    #[test]
    fn parses_custom_method_verbatim() {
        assert_eq!("PURGE".parse(), Ok(Method::Custom("PURGE".to_string())));
        // Lowercase or mixed-case tokens are not normalized, they are kept as given: the file
        // format grammar is responsible for rejecting non-uppercase tokens, not this type.
        assert_eq!("get".parse(), Ok(Method::Custom("get".to_string())));
    }

    #[test]
    fn displays_round_trip_for_standard_methods() {
        for token in ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"] {
            let method: Method = token.parse().expect("parsing a method never fails");
            assert_eq!(method.to_string(), token);
        }
    }

    #[test]
    fn displays_custom_method_verbatim() {
        let method: Method = "PURGE".parse().expect("parsing a method never fails");
        assert_eq!(method.to_string(), "PURGE");
    }

    #[test]
    fn default_is_get() {
        assert_eq!(Method::default(), Method::Get);
    }
}
