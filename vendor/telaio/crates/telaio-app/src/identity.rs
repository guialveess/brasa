//! The facts about an app that the other modules of this crate need.

/// Who the app is. Build it once as a `const` and pass `&IDENTITY` to the helpers.
///
/// ```
/// use telaio_app::AppIdentity;
///
/// const IDENTITY: AppIdentity = AppIdentity {
///     name: "Postino",
///     slug: "postino",
///     app_id: "codes.tanis.postino",
///     version: "0.1.1",
///     env_prefix: "POSTINO",
///     repository: "https://github.com/tanisperez/postino",
/// };
/// assert_eq!(IDENTITY.env_var("LOG"), "POSTINO_LOG");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppIdentity {
    /// The display name, for example `Postino`.
    pub name: &'static str,
    /// The lowercase name used in folders and file names, for example `postino`. It is also the
    /// prefix of the app's crate names, so their log targets (`postino_app`, `postino_cli`) log at
    /// the full level.
    pub slug: &'static str,
    /// The application id: Wayland `app_id`, X11 `WM_CLASS`, Windows AppUserModelID, for example
    /// `codes.tanis.postino`.
    pub app_id: &'static str,
    /// The running version, normally `env!("CARGO_PKG_VERSION")`.
    pub version: &'static str,
    /// The prefix of the app's environment variables, for example `POSTINO`.
    pub env_prefix: &'static str,
    /// The repository URL, without a trailing slash.
    pub repository: &'static str,
}

impl AppIdentity {
    /// The environment variable `<PREFIX>_<suffix>`, for example `POSTINO_LOG`.
    pub fn env_var(&self, suffix: &str) -> String {
        format!("{}_{suffix}", self.env_prefix)
    }

    /// The GitHub release page of the running version.
    pub fn release_notes_url(&self) -> String {
        format!("{}/releases/tag/v{}", self.repository, self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const IDENTITY: AppIdentity = AppIdentity {
        name: "App",
        slug: "app",
        app_id: "test.app",
        version: "1.2.3",
        env_prefix: "APP",
        repository: "https://example.test/app",
    };

    #[test]
    fn env_vars_are_prefixed() {
        assert_eq!(IDENTITY.env_var("LOG"), "APP_LOG");
    }

    #[test]
    fn the_release_notes_url_points_at_the_version_tag() {
        assert_eq!(
            IDENTITY.release_notes_url(),
            "https://example.test/app/releases/tag/v1.2.3"
        );
    }
}
