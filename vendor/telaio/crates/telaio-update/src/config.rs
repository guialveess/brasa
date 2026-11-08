//! What differs from one app to another.

use std::path::PathBuf;

use crate::UpdateError;

/// Everything app specific about the updater. Build it once, usually as a `const` in the app.
///
/// ```
/// use telaio_update::UpdaterConfig;
///
/// const UPDATER: UpdaterConfig = UpdaterConfig {
///     owner: "tanisperez",
///     repo: "postino",
///     app_name: "Postino",
///     url_env: "POSTINO_UPDATE_URL",
///     version: env!("CARGO_PKG_VERSION"),
/// };
/// assert_eq!(
///     UPDATER.latest_url(),
///     "https://github.com/tanisperez/postino/releases/latest/download/latest.json"
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdaterConfig {
    /// GitHub owner of the repository that publishes the releases.
    pub owner: &'static str,
    /// GitHub repository that publishes the releases.
    pub repo: &'static str,
    /// The app name as it is spelled in the macOS bundle (`Postino` for `Postino.app`). Its
    /// lowercase form names the staging folder (`.postino-update`) and the download cache
    /// (`postino/updates`).
    pub app_name: &'static str,
    /// Name of the environment variable that overrides the manifest URL at runtime, to test
    /// updates between pre-releases (`releases/latest` skips them).
    pub url_env: &'static str,
    /// The running version, what a manifest is compared against. Normally
    /// `env!("CARGO_PKG_VERSION")` of the app.
    pub version: &'static str,
}

impl UpdaterConfig {
    /// Where the manifest of the latest stable release is published:
    /// `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`.
    #[must_use]
    pub fn latest_url(&self) -> String {
        format!(
            "https://github.com/{}/{}/releases/latest/download/latest.json",
            self.owner, self.repo
        )
    }

    /// The manifest URL: the [`url_env`](Self::url_env) variable when set and not empty,
    /// otherwise [`latest_url`](Self::latest_url).
    #[must_use]
    pub fn manifest_url(&self) -> String {
        self.manifest_url_with(std::env::var(self.url_env).ok().as_deref())
    }

    /// [`manifest_url`](Self::manifest_url) with the override given instead of read from the
    /// environment.
    pub(crate) fn manifest_url_with(&self, override_url: Option<&str>) -> String {
        match override_url.map(str::trim) {
            Some(value) if !value.is_empty() => value.to_string(),
            _ => self.latest_url(),
        }
    }

    /// Fetches the manifest of [`manifest_url`](Self::manifest_url) and compares it with
    /// [`version`](Self::version). See [`crate::check`]. Blocking.
    ///
    /// # Errors
    ///
    /// The same as [`crate::check`].
    pub fn check(&self, platform: crate::Platform) -> Result<Option<crate::Update>, UpdateError> {
        crate::check(&self.manifest_url(), self.version, platform)
    }

    /// The macOS bundle folder name, `<app_name>.app`.
    #[must_use]
    pub fn bundle_name(&self) -> String {
        format!("{}.app", self.app_name)
    }

    /// The folder, next to the bundle, where the new version is unpacked.
    pub(crate) fn staging_dir(&self) -> String {
        format!(".{}-update", self.app_name.to_lowercase())
    }

    /// Where downloaded updates are kept (`<cache dir>/<app>/updates`), `None` when the OS cache
    /// directory is unknown.
    #[must_use]
    pub fn download_dir(&self) -> Option<PathBuf> {
        dirs::cache_dir().map(|base| base.join(self.app_name.to_lowercase()).join("updates"))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    const CONFIG: UpdaterConfig = UpdaterConfig {
        owner: "tanisperez",
        repo: "postino",
        app_name: "Postino",
        url_env: "TELAIO_UPDATE_TEST_UNSET_URL",
        version: "0.1.0",
    };

    #[test]
    fn the_latest_url_is_built_from_owner_and_repo() {
        assert_eq!(
            CONFIG.latest_url(),
            "https://github.com/tanisperez/postino/releases/latest/download/latest.json"
        );
    }

    #[test]
    fn the_manifest_url_defaults_to_latest() {
        // `set_var` is unsafe and the workspace forbids unsafe code, so the environment is only
        // read unset here, and the override logic is checked through `manifest_url_with`.
        assert_eq!(CONFIG.manifest_url(), CONFIG.latest_url());
    }

    #[test]
    fn the_override_wins_when_it_is_not_blank() {
        assert_eq!(
            CONFIG.manifest_url_with(Some(" http://h/rc/latest.json ")),
            "http://h/rc/latest.json"
        );
        assert_eq!(CONFIG.manifest_url_with(Some("  ")), CONFIG.latest_url());
        assert_eq!(CONFIG.manifest_url_with(Some("")), CONFIG.latest_url());
        assert_eq!(CONFIG.manifest_url_with(None), CONFIG.latest_url());
    }

    #[test]
    fn names_derive_from_the_app_name() {
        assert_eq!(CONFIG.bundle_name(), "Postino.app");
        assert_eq!(CONFIG.staging_dir(), ".postino-update");
    }

    #[test]
    fn downloads_go_under_the_app_cache_dir() {
        if let Some(dir) = CONFIG.download_dir() {
            assert!(dir.ends_with("postino/updates"));
        }
    }
}
