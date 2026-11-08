//! Pure editing operations on a request's [`Auth`], used by the Auth tab of the request editor
//! (`views/request_editor.rs`). Kept free of `gpui` types, like `state::request_edit`.

use brasa_core::{ApiKeyLocation, Auth, Request};
use rust_i18n::t;

/// The kind of auth a request has, mirroring [`Auth`] without its fields. Used by the Auth tab's
/// type selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthKind {
    /// No auth.
    #[default]
    None,
    /// HTTP Basic.
    Basic,
    /// A bearer token.
    Bearer,
    /// An API key.
    ApiKey,
}

impl AuthKind {
    /// The kind of an existing [`Auth`].
    pub fn of(auth: &Auth) -> Self {
        match auth {
            Auth::None => AuthKind::None,
            Auth::Basic { .. } => AuthKind::Basic,
            Auth::Bearer { .. } => AuthKind::Bearer,
            Auth::ApiKey { .. } => AuthKind::ApiKey,
        }
    }

    /// Every kind, in the order the type selector lists them.
    pub const ALL: [AuthKind; 4] = [
        AuthKind::None,
        AuthKind::Basic,
        AuthKind::Bearer,
        AuthKind::ApiKey,
    ];

    /// The label shown in the type selector.
    pub fn label(self) -> String {
        match self {
            AuthKind::None => t!("request.auth.none"),
            AuthKind::Basic => t!("request.auth.basic"),
            AuthKind::Bearer => t!("request.auth.bearer"),
            AuthKind::ApiKey => t!("request.auth.api_key"),
        }
        .into_owned()
    }

    /// The fields this kind shows, in order.
    pub fn fields(self) -> &'static [AuthField] {
        match self {
            AuthKind::None => &[],
            AuthKind::Basic => &[AuthField::Username, AuthField::Password],
            AuthKind::Bearer => &[AuthField::Token],
            AuthKind::ApiKey => &[AuthField::KeyName, AuthField::KeyValue],
        }
    }
}

/// One text field of the Auth tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthField {
    /// The user name of a basic auth.
    Username,
    /// The password of a basic auth.
    Password,
    /// The token of a bearer auth.
    Token,
    /// The header or query parameter name of an API key.
    KeyName,
    /// The API key itself.
    KeyValue,
}

impl AuthField {
    /// The label shown next to the field.
    pub fn label(self) -> String {
        match self {
            AuthField::Username => t!("request.auth.username"),
            AuthField::Password => t!("request.auth.password"),
            AuthField::Token => t!("request.auth.token"),
            AuthField::KeyName => t!("request.auth.name"),
            AuthField::KeyValue => t!("common.value"),
        }
        .into_owned()
    }

    /// Whether the field holds a secret, so its input is masked and its menu and tooltips hide
    /// variable values.
    pub fn is_secret(self) -> bool {
        matches!(
            self,
            AuthField::Password | AuthField::Token | AuthField::KeyValue
        )
    }
}

/// What the auth kinds a request is not showing held, while its tab is open, so switching the
/// type and back restores them. Never saved, like `state::request_edit::BodyStash`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuthStash {
    basic: Option<Auth>,
    bearer: Option<Auth>,
    api_key: Option<Auth>,
}

impl AuthStash {
    fn slot(&mut self, kind: AuthKind) -> Option<&mut Option<Auth>> {
        match kind {
            AuthKind::None => None,
            AuthKind::Basic => Some(&mut self.basic),
            AuthKind::Bearer => Some(&mut self.bearer),
            AuthKind::ApiKey => Some(&mut self.api_key),
        }
    }
}

/// Switches `request.auth` to `kind`, putting the current auth in `stash` and restoring what
/// `stash` holds for `kind`, or empty fields.
pub fn set_auth_kind(request: &mut Request, kind: AuthKind, stash: &mut AuthStash) {
    let current = std::mem::take(&mut request.auth);
    if let Some(slot) = stash.slot(AuthKind::of(&current)) {
        *slot = Some(current);
    }
    request.auth = stash
        .slot(kind)
        .and_then(Option::take)
        .unwrap_or_else(|| match kind {
            AuthKind::None => Auth::None,
            AuthKind::Basic => Auth::Basic {
                username: String::new(),
                password: String::new(),
            },
            AuthKind::Bearer => Auth::Bearer {
                token: String::new(),
            },
            AuthKind::ApiKey => Auth::ApiKey {
                name: String::new(),
                value: String::new(),
                location: ApiKeyLocation::Header,
            },
        });
}

/// The text of `field` in `auth`, empty when `auth` has no such field.
pub fn field_value(auth: &Auth, field: AuthField) -> &str {
    match (auth, field) {
        (Auth::Basic { username, .. }, AuthField::Username) => username,
        (Auth::Basic { password, .. }, AuthField::Password) => password,
        (Auth::Bearer { token }, AuthField::Token) => token,
        (Auth::ApiKey { name, .. }, AuthField::KeyName) => name,
        (Auth::ApiKey { value, .. }, AuthField::KeyValue) => value,
        _ => "",
    }
}

/// Sets `field` of `auth` to `text`. Does nothing when `auth` has no such field.
pub fn set_field(auth: &mut Auth, field: AuthField, text: String) {
    let target = match (auth, field) {
        (Auth::Basic { username, .. }, AuthField::Username) => username,
        (Auth::Basic { password, .. }, AuthField::Password) => password,
        (Auth::Bearer { token }, AuthField::Token) => token,
        (Auth::ApiKey { name, .. }, AuthField::KeyName) => name,
        (Auth::ApiKey { value, .. }, AuthField::KeyValue) => value,
        _ => return,
    };
    *target = text;
}

/// Sets where an API key is sent. Does nothing for another kind of auth.
pub fn set_api_key_location(auth: &mut Auth, new_location: ApiKeyLocation) {
    if let Auth::ApiKey { location, .. } = auth {
        *location = new_location;
    }
}

/// What a request sends in place of its auth: an enabled header (or, for an API key in the
/// query, an enabled Params entry) of the same name, which the runner lets win. Compares the
/// names as written, before `{{ }}` markers are replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overridden {
    /// An enabled header of the Headers tab, with its name as written there.
    Header(String),
    /// An enabled entry of the Params tab, with its name.
    Param(String),
}

/// The entry that is sent in place of `request`'s auth, if any. See [`Overridden`].
pub fn overridden_by(request: &Request) -> Option<Overridden> {
    let (name, location) = match &request.auth {
        Auth::None => return None,
        Auth::Basic { .. } | Auth::Bearer { .. } => ("Authorization", ApiKeyLocation::Header),
        Auth::ApiKey { name, location, .. } if !name.is_empty() => (name.as_str(), *location),
        Auth::ApiKey { .. } => return None,
    };
    match location {
        ApiKeyLocation::Header => request
            .headers
            .iter()
            .find(|header| header.enabled && header.key.eq_ignore_ascii_case(name))
            .map(|header| Overridden::Header(header.key.clone())),
        ApiKeyLocation::Query => request
            .query
            .iter()
            .find(|entry| entry.enabled && entry.key == name)
            .map(|entry| Overridden::Param(entry.key.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brasa_core::KeyValue;
    use pretty_assertions::assert_eq;

    fn bearer(token: &str) -> Auth {
        Auth::Bearer {
            token: token.to_string(),
        }
    }

    #[test]
    fn kind_of_matches_every_variant() {
        for kind in AuthKind::ALL {
            let mut request = Request::default();
            set_auth_kind(&mut request, kind, &mut AuthStash::default());
            assert_eq!(AuthKind::of(&request.auth), kind);
        }
    }

    #[test]
    fn switching_kind_and_back_restores_the_fields() {
        let mut request = Request {
            auth: bearer("t"),
            ..Request::default()
        };
        let mut stash = AuthStash::default();
        set_auth_kind(&mut request, AuthKind::Basic, &mut stash);
        assert_eq!(field_value(&request.auth, AuthField::Username), "");
        set_field(&mut request.auth, AuthField::Username, "demo".to_string());
        set_auth_kind(&mut request, AuthKind::None, &mut stash);
        assert_eq!(request.auth, Auth::None);

        set_auth_kind(&mut request, AuthKind::Bearer, &mut stash);
        assert_eq!(request.auth, bearer("t"));
        set_auth_kind(&mut request, AuthKind::Basic, &mut stash);
        assert_eq!(field_value(&request.auth, AuthField::Username), "demo");
    }

    #[test]
    fn set_field_ignores_a_field_of_another_kind() {
        let mut auth = bearer("t");
        set_field(&mut auth, AuthField::Username, "x".to_string());
        assert_eq!(auth, bearer("t"));
        set_field(&mut auth, AuthField::Token, "u".to_string());
        assert_eq!(auth, bearer("u"));
        assert_eq!(field_value(&auth, AuthField::Password), "");
    }

    #[test]
    fn api_key_location_changes_only_an_api_key() {
        let mut auth = bearer("t");
        set_api_key_location(&mut auth, ApiKeyLocation::Query);
        assert_eq!(auth, bearer("t"));

        let mut request = Request::default();
        set_auth_kind(&mut request, AuthKind::ApiKey, &mut AuthStash::default());
        set_api_key_location(&mut request.auth, ApiKeyLocation::Query);
        assert!(matches!(
            request.auth,
            Auth::ApiKey {
                location: ApiKeyLocation::Query,
                ..
            }
        ));
    }

    #[test]
    fn an_enabled_authorization_header_overrides_basic_and_bearer() {
        let mut request = Request {
            headers: vec![KeyValue::new("authorization", "Bearer x")],
            auth: bearer("t"),
            ..Request::default()
        };
        assert_eq!(
            overridden_by(&request),
            Some(Overridden::Header("authorization".to_string()))
        );
        request.headers[0].enabled = false;
        assert_eq!(overridden_by(&request), None);
        request.auth = Auth::None;
        request.headers[0].enabled = true;
        assert_eq!(overridden_by(&request), None);
    }

    #[test]
    fn an_api_key_is_overridden_by_an_entry_where_it_goes() {
        let mut request = Request {
            headers: vec![KeyValue::new("X-Key", "a")],
            query: vec![KeyValue::new("X-Key", "b")],
            auth: Auth::ApiKey {
                name: "x-key".to_string(),
                value: "k".to_string(),
                location: ApiKeyLocation::Header,
            },
            ..Request::default()
        };
        assert_eq!(
            overridden_by(&request),
            Some(Overridden::Header("X-Key".to_string()))
        );
        // Query parameter names are case sensitive.
        set_api_key_location(&mut request.auth, ApiKeyLocation::Query);
        assert_eq!(overridden_by(&request), None);
        set_field(&mut request.auth, AuthField::KeyName, "X-Key".to_string());
        assert_eq!(
            overridden_by(&request),
            Some(Overridden::Param("X-Key".to_string()))
        );
    }
}
