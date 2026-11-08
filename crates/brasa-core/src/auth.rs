//! The authorization of a request.

/// The `::: auth <type>` section of a [`crate::Request`], as edited before interpolation.
///
/// Every field may hold `{{ }}` markers. The runner turns it into a header (or a query entry for
/// an API key sent in the query) when the request is resolved, unless the request already has an
/// enabled header or query entry with that name, which wins.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Auth {
    /// No authorization.
    #[default]
    None,
    /// HTTP Basic: `Authorization: Basic base64(username:password)`.
    Basic {
        /// The user name.
        username: String,
        /// The password.
        password: String,
    },
    /// A bearer token: `Authorization: Bearer <token>`.
    Bearer {
        /// The token.
        token: String,
    },
    /// An API key, sent as a header or a query parameter named `name`.
    ApiKey {
        /// The header or query parameter name, for example `X-API-Key`.
        name: String,
        /// The key itself.
        value: String,
        /// Where the key is sent.
        location: ApiKeyLocation,
    },
}

/// Where an [`Auth::ApiKey`] is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApiKeyLocation {
    /// As a request header.
    #[default]
    Header,
    /// As a query parameter, appended to the URL.
    Query,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn default_is_none() {
        assert_eq!(Auth::default(), Auth::None);
        assert_eq!(ApiKeyLocation::default(), ApiKeyLocation::Header);
    }
}
