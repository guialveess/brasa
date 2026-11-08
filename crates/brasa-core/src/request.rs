//! The editable request model, as stored in a `.brasa` file.

use crate::auth::Auth;
use crate::body::Body;
use crate::key_value::KeyValue;
use crate::method::Method;

/// A request as edited by the user and stored in a `.brasa` file.
///
/// This is the "source" form, before variable interpolation. See [`crate::ResolvedRequest`] for
/// the form actually sent over the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The HTTP method.
    pub method: Method,
    /// The request URL, may contain `{{ }}` placeholders and a query string.
    pub url: String,
    /// The header list, in file order. Disabled headers are kept but not sent.
    pub headers: Vec<KeyValue>,
    /// The `::: query` section, in file order. Disabled entries are kept but not sent.
    pub query: Vec<KeyValue>,
    /// The `::: auth` section. [`Auth::None`] when absent.
    pub auth: Auth,
    /// The request body.
    pub body: Body,
    /// The `::: pre` section, raw JavaScript. Empty when absent.
    pub pre_script: String,
    /// Whether [`Self::pre_script`] runs before the request is sent. Kept separate from an empty
    /// script, so a script the user wrote but turned off round-trips instead of looking deleted.
    pub pre_enabled: bool,
    /// The `::: post` section, raw JavaScript. Empty when absent.
    pub post_script: String,
    /// Whether [`Self::post_script`] runs after the response comes back, see [`Self::pre_enabled`].
    pub post_enabled: bool,
    /// The `::: docs` section, free Markdown notes. Empty when absent.
    pub docs: String,
}

impl Default for Request {
    /// A bare `GET` request with both scripts enabled, the sensible starting point for a newly
    /// created request, and for one read from a `.brasa` file predating the toggle, which has no
    /// way to say a script was turned off and so always ran.
    fn default() -> Self {
        Request {
            method: Method::default(),
            url: String::new(),
            headers: Vec::new(),
            query: Vec::new(),
            auth: Auth::default(),
            body: Body::default(),
            pre_script: String::new(),
            pre_enabled: true,
            post_script: String::new(),
            post_enabled: true,
            docs: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn default_is_a_bare_get_request() {
        let request = Request::default();
        assert_eq!(request.method, Method::Get);
        assert_eq!(request.url, "");
        assert!(request.headers.is_empty());
        assert!(request.query.is_empty());
        assert_eq!(request.auth, Auth::None);
        assert_eq!(request.body, Body::None);
        assert_eq!(request.pre_script, "");
        assert!(request.pre_enabled);
        assert_eq!(request.post_script, "");
        assert!(request.post_enabled);
        assert_eq!(request.docs, "");
    }
}
