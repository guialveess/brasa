//! Where an app keeps its files. The log folder is in [`crate::logging::log_dir`].

use std::path::PathBuf;

use crate::AppIdentity;

/// The OS config folder of the app, `<config dir>/<slug>`: `~/.config/postino` on Linux,
/// `~/Library/Application Support/postino` on macOS, `%APPDATA%\postino` on Windows. `None` when
/// the OS does not report the base directory. The folder is not created.
pub fn config_dir(identity: &AppIdentity) -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join(identity.slug))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_dir_ends_in_the_slug() {
        let identity = AppIdentity {
            name: "App",
            slug: "app",
            app_id: "test.app",
            version: "1.0.0",
            env_prefix: "APP",
            repository: "https://example.test/app",
        };
        if let Some(dir) = config_dir(&identity) {
            assert!(dir.ends_with("app"), "{dir:?}");
            assert_eq!(dir.parent(), dirs::config_dir().as_deref());
        }
    }
}
