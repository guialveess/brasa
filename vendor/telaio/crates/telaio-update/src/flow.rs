//! The decisions around an update, free of any UI: the build gate, portable copies, package
//! managed Linux installs, the ready update, the state of the checks and how an update is
//! installed. Apps render the result.
//!
//! The blocking work is [`check_and_download`] and [`start_install`]; run them on a background
//! executor.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{Platform, UpdateError, UpdaterConfig, install};

/// Delay between startup and the single automatic check, so it never competes with the first
/// frames. There is no timer or polling after that.
pub const STARTUP_DELAY: Duration = Duration::from_secs(10);

/// Whether this build has an updater: built with the updater flag set to `github` and running on
/// a platform with an installable asset. `build_flag` is the app's
/// `option_env!("<APP>_UPDATER")`, which only the app crate can read at its own build time.
#[must_use]
pub fn enabled_for(build_flag: Option<&str>, platform: Option<Platform>) -> bool {
    build_flag == Some("github") && platform.is_some()
}

/// [`enabled_for`] the platform this binary runs on.
#[must_use]
pub fn updater_enabled(build_flag: Option<&str>) -> bool {
    enabled_for(build_flag, Platform::current())
}

/// Whether a Linux copy is updated by a package manager or a sandbox store, so the app must not
/// announce new versions at all: a binary under `/usr` but not `/usr/local` (deb, rpm, pacman),
/// under `/bin`, `/sbin`, `/app` (Flatpak) or `/snap`, or running `sandboxed` (Flatpak or Snap,
/// see [`is_sandboxed`]). Everything else (the tarball unpacked by hand, a per user install, an
/// AppImage) is a manual install that only learns about new versions from the app. With an
/// unknown executable path the copy counts as package managed, the safe side: nothing is shown.
/// Always `false` on other platforms.
#[must_use]
pub fn is_package_managed(platform: Option<Platform>, exe: Option<&Path>, sandboxed: bool) -> bool {
    if platform != Some(Platform::LinuxX86_64) {
        return false;
    }
    let Some(exe) = exe else {
        return true;
    };
    if sandboxed {
        return true;
    }
    if exe.starts_with("/usr/local") {
        return false;
    }
    ["/usr", "/bin", "/sbin", "/app", "/snap"]
        .iter()
        .any(|prefix| exe.starts_with(prefix))
}

/// Whether this process runs inside a Flatpak or Snap sandbox, told by the variables and the file
/// they set. Reads the environment, so call it once.
#[must_use]
pub fn is_sandboxed() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
        || std::env::var_os("SNAP").is_some()
        || Path::new("/.flatpak-info").exists()
}

/// Whether the automatic check at startup runs: the build has an updater and the user left
/// "Check for updates automatically" on.
#[must_use]
pub fn should_check_at_startup(enabled: bool, setting: bool) -> bool {
    enabled && setting
}

/// Whether this copy only announces new versions instead of downloading them. On Windows, a
/// portable one: the installer leaves its uninstaller next to the exe, so a copy without it comes
/// from the zip and must not run the installer (that would add a second, separate copy). With an
/// unknown executable path the copy counts as portable, the safe side: the user is sent to the
/// release page instead of getting a second install. On Linux, always: the app never replaces
/// itself there (a package managed copy announces nothing, see [`is_package_managed`]).
#[must_use]
pub fn is_portable(platform: Option<Platform>, exe: Option<&Path>) -> bool {
    match platform {
        Some(Platform::WindowsX86_64) => !exe.is_some_and(install::windows_is_installed),
        Some(Platform::LinuxX86_64) => true,
        _ => false,
    }
}

/// A newer version the user can act on: downloaded and verified, waiting for a restart into it,
/// or, for a portable copy, only announced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyUpdate {
    /// The new version, as text.
    pub version: String,
    /// The release page with the notes (and, for a portable copy, the zip to download).
    pub notes_url: String,
    /// The verified installer (Windows) or archive (macOS). `None` for a portable copy, which
    /// downloads nothing: the user takes the new zip from the release page.
    pub path: Option<PathBuf>,
}

impl ReadyUpdate {
    /// Whether the user has to download the new version by hand, from [`Self::notes_url`], instead
    /// of restarting into a downloaded one.
    #[must_use]
    pub fn is_manual_download(&self) -> bool {
        self.path.is_none()
    }
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Nothing newer than the running version.
    UpToDate,
    /// A newer version, downloaded and verified.
    Ready(ReadyUpdate),
}

/// How the last check went, for the app's About or Settings screen.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    /// No check has finished yet.
    #[default]
    Idle,
    /// A check is running.
    Checking,
    /// The last check found nothing newer.
    UpToDate,
    /// The last check failed.
    Failed,
}

/// What the app remembers about updates: the ready one (if any) and whether a check is running.
#[derive(Debug, Default)]
pub struct UpdateState {
    ready: Option<ReadyUpdate>,
    checking: bool,
    installing: bool,
    status: CheckStatus,
}

impl UpdateState {
    /// The update waiting to be installed.
    #[must_use]
    pub fn ready(&self) -> Option<&ReadyUpdate> {
        self.ready.as_ref()
    }

    /// How the last check went, or [`CheckStatus::Checking`] while one runs.
    #[must_use]
    pub fn status(&self) -> CheckStatus {
        self.status
    }

    /// Marks a check as started. Returns `false`, doing nothing, when one is already running or
    /// an update is already ready (there is nothing more to look for).
    pub fn begin_check(&mut self) -> bool {
        if self.checking || self.ready.is_some() {
            return false;
        }
        self.checking = true;
        self.status = CheckStatus::Checking;
        true
    }

    /// Records the end of a check. A ready update is remembered, never replaced by a later one.
    pub fn finish_check(&mut self, outcome: &CheckOutcome) {
        self.checking = false;
        self.status = match outcome {
            CheckOutcome::UpToDate => CheckStatus::UpToDate,
            CheckOutcome::Ready(_) => CheckStatus::Idle,
        };
        if let CheckOutcome::Ready(update) = outcome
            && self.ready.is_none()
        {
            self.ready = Some(update.clone());
        }
    }

    /// Records a failed check.
    pub fn fail_check(&mut self) {
        self.checking = false;
        self.status = CheckStatus::Failed;
    }

    /// Marks the installation as started and returns the update to install. `None`, doing
    /// nothing, when no update is ready or an installation is already running (a double click).
    pub fn begin_install(&mut self) -> Option<ReadyUpdate> {
        if self.installing {
            return None;
        }
        let ready = self.ready.clone()?;
        self.installing = true;
        Some(ready)
    }

    /// Records that the installation did not take over, so the user can try again.
    pub fn fail_install(&mut self) {
        self.installing = false;
    }
}

/// Checks the manifest and, for a newer version, downloads and verifies the asset, unless this is
/// a `portable` copy (see [`is_portable`]), which only learns that the version exists. Blocking.
///
/// # Errors
///
/// Whatever [`UpdaterConfig::check`] and [`crate::download`] report, or [`UpdateError::Io`] when
/// there is no cache directory.
pub fn check_and_download(
    config: &UpdaterConfig,
    platform: Platform,
    portable: bool,
) -> Result<CheckOutcome, UpdateError> {
    let Some(update) = config.check(platform)? else {
        return Ok(CheckOutcome::UpToDate);
    };
    log::info!("update available {}", update.version);
    let ready = ready_update(
        update.version.to_string(),
        update.notes_url.clone(),
        portable,
        || {
            let dir = config.download_dir().ok_or_else(|| {
                UpdateError::Io(std::io::Error::other("the cache directory is unknown"))
            })?;
            crate::download(&update, &dir)
        },
    )?;
    Ok(CheckOutcome::Ready(ready))
}

/// The [`ReadyUpdate`] of a newer version: `download` runs for an installed copy, never for a
/// `portable` one, which keeps no file.
fn ready_update(
    version: String,
    notes_url: String,
    portable: bool,
    download: impl FnOnce() -> Result<PathBuf, UpdateError>,
) -> Result<ReadyUpdate, UpdateError> {
    let path = if portable {
        log::info!("portable copy, not downloading the installer");
        None
    } else {
        Some(download()?)
    };
    Ok(ReadyUpdate {
        version,
        notes_url,
        path,
    })
}

/// How an update is installed on a platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// Run the installer, then quit.
    WindowsInstaller,
    /// Unpack next to the bundle, swap it after quitting, reopen.
    MacosSwap,
    /// Nothing to install in place: the user takes the new version from the release page.
    ReleasePage,
}

/// The install method of `platform`.
#[must_use]
pub fn install_method(platform: Platform) -> InstallMethod {
    match platform {
        Platform::WindowsX86_64 => InstallMethod::WindowsInstaller,
        Platform::MacosAarch64 => InstallMethod::MacosSwap,
        Platform::LinuxX86_64 => InstallMethod::ReleasePage,
    }
}

/// What the app does after trying to install.
#[derive(Debug, PartialEq, Eq)]
pub enum InstallOutcome {
    /// The installer or the swap script is running: quit now.
    Quit,
    /// Not possible in place (not in a bundle, folder not writable): send the user to the
    /// release page.
    Fallback,
    /// Failed, with the error text to show.
    Failed(String),
}

/// Starts the installation of `ready`. Blocking (it unpacks the archive on macOS), so run it on
/// the background executor. `exe` is the running executable. An update with nothing downloaded
/// (a portable copy) falls back to the release page.
#[must_use]
pub fn start_install(
    config: &UpdaterConfig,
    platform: Platform,
    ready: &ReadyUpdate,
    exe: &Path,
) -> InstallOutcome {
    let Some(path) = ready.path.as_deref() else {
        return InstallOutcome::Fallback;
    };
    match install_method(platform) {
        InstallMethod::WindowsInstaller => match install::spawn_windows_installer(path) {
            Ok(()) => InstallOutcome::Quit,
            Err(error) => InstallOutcome::Failed(error.to_string()),
        },
        InstallMethod::MacosSwap => {
            let Some(bundle) = install::macos_bundle_from_exe(exe) else {
                return InstallOutcome::Fallback;
            };
            let staged = match install::macos_stage(config, path, &bundle) {
                Ok(staged) => staged,
                Err(UpdateError::Io(error))
                    if error.kind() == std::io::ErrorKind::PermissionDenied =>
                {
                    return InstallOutcome::Fallback;
                }
                Err(error) => return InstallOutcome::Failed(error.to_string()),
            };
            let script =
                install::macos_swap_script(config, std::process::id(), &bundle, &staged, "open");
            match install::spawn_detached_script(&script) {
                Ok(()) => InstallOutcome::Quit,
                Err(error) => InstallOutcome::Failed(error.to_string()),
            }
        }
        InstallMethod::ReleasePage => InstallOutcome::Fallback,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const CONFIG: UpdaterConfig = UpdaterConfig {
        owner: "tanisperez",
        repo: "postino",
        app_name: "Postino",
        url_env: "POSTINO_UPDATE_URL",
        version: "0.1.0",
    };

    fn ready(version: &str) -> ReadyUpdate {
        ReadyUpdate {
            version: version.to_string(),
            notes_url: "https://example.com/notes".to_string(),
            path: Some(PathBuf::from("/tmp/postino-update")),
        }
    }

    fn portable_ready(version: &str) -> ReadyUpdate {
        ReadyUpdate {
            path: None,
            ..ready(version)
        }
    }

    fn ready_with(
        portable: bool,
        download: impl FnOnce() -> Result<PathBuf, UpdateError>,
    ) -> Result<ReadyUpdate, UpdateError> {
        let notes_url = "https://example.com/notes".to_string();
        ready_update("0.2.0".to_string(), notes_url, portable, download)
    }

    #[test]
    fn the_gate_needs_the_build_flag_and_a_platform() {
        let mac = Some(Platform::MacosAarch64);
        assert!(enabled_for(Some("github"), mac));
        assert!(!enabled_for(None, mac));
        assert!(!enabled_for(Some("other"), mac));
        assert!(!enabled_for(Some("github"), None));
    }

    #[test]
    fn only_a_windows_copy_without_the_uninstaller_is_portable() {
        let windows = Some(Platform::WindowsX86_64);
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("postino.exe");
        std::fs::write(&exe, "exe").unwrap();
        assert!(is_portable(windows, Some(&exe)));
        std::fs::write(dir.path().join("unins000.exe"), "uninstaller").unwrap();
        assert!(!is_portable(windows, Some(&exe)));
    }

    #[test]
    fn an_unknown_executable_counts_as_portable_on_windows() {
        assert!(is_portable(Some(Platform::WindowsX86_64), None));
    }

    #[test]
    fn other_platforms_are_never_portable() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("postino");
        assert!(!is_portable(Some(Platform::MacosAarch64), Some(&exe)));
        assert!(!is_portable(Some(Platform::MacosAarch64), None));
        assert!(!is_portable(None, Some(&exe)));
    }

    #[test]
    fn a_linux_copy_is_always_portable() {
        let linux = Some(Platform::LinuxX86_64);
        assert!(is_portable(
            linux,
            Some(Path::new("/home/me/.local/bin/postino"))
        ));
        assert!(is_portable(linux, None));
    }

    #[test]
    fn linux_package_managers_own_system_paths() {
        let linux = Some(Platform::LinuxX86_64);
        let managed = |path: &str| is_package_managed(linux, Some(Path::new(path)), false);
        assert!(managed("/usr/bin/postino"));
        assert!(managed("/usr/lib/postino/postino"));
        assert!(managed("/bin/postino"));
        assert!(managed("/app/bin/postino"));
        assert!(managed("/snap/postino/12/bin/postino"));
        assert!(!managed("/usr/local/bin/postino"));
        assert!(!managed("/home/me/.local/bin/postino"));
        assert!(!managed("/opt/postino/bin/postino"));
        assert!(!managed("/tmp/.mount_PostinAbc/usr/bin/postino"));
        assert!(!managed("/usrx/bin/postino"));
    }

    #[test]
    fn a_sandbox_or_an_unknown_exe_is_package_managed_on_linux() {
        let linux = Some(Platform::LinuxX86_64);
        let home = Path::new("/home/me/.local/bin/postino");
        assert!(is_package_managed(linux, Some(home), true));
        assert!(is_package_managed(
            linux,
            Some(Path::new("/usr/local/bin/postino")),
            true
        ));
        assert!(is_package_managed(linux, None, false));
    }

    #[test]
    fn other_platforms_are_never_package_managed() {
        let exe = Path::new("/usr/bin/postino");
        assert!(!is_package_managed(
            Some(Platform::MacosAarch64),
            Some(exe),
            true
        ));
        assert!(!is_package_managed(
            Some(Platform::WindowsX86_64),
            None,
            true
        ));
        assert!(!is_package_managed(None, Some(exe), true));
    }

    #[test]
    fn a_linux_update_falls_back_to_the_release_page() {
        assert_eq!(
            install_method(Platform::LinuxX86_64),
            InstallMethod::ReleasePage
        );
        let outcome = start_install(
            &CONFIG,
            Platform::LinuxX86_64,
            &ready("0.2.0"),
            Path::new("/home/me/.local/bin/postino"),
        );
        assert_eq!(outcome, InstallOutcome::Fallback);
    }

    #[test]
    fn a_portable_copy_downloads_nothing() {
        let ready = ready_with(true, || panic!("a portable copy must not download")).unwrap();
        assert_eq!(ready, portable_ready("0.2.0"));
        assert!(ready.is_manual_download());
    }

    #[test]
    fn an_installed_copy_downloads_the_installer() {
        let ready = ready_with(false, || Ok(PathBuf::from("/tmp/postino-update"))).unwrap();
        assert_eq!(ready, self::ready("0.2.0"));
        assert!(!ready.is_manual_download());
    }

    #[test]
    fn a_failed_download_is_an_error() {
        let result = ready_with(false, || Err(UpdateError::Network("down".to_string())));
        assert!(result.is_err());
    }

    #[test]
    fn a_portable_update_is_kept_like_any_other() {
        let mut state = UpdateState::default();
        state.begin_check();
        state.finish_check(&CheckOutcome::Ready(portable_ready("0.2.0")));
        assert_eq!(state.ready(), Some(&portable_ready("0.2.0")));
        assert!(!state.begin_check());
    }

    #[test]
    fn a_portable_update_falls_back_to_the_release_page() {
        let outcome = start_install(
            &CONFIG,
            Platform::WindowsX86_64,
            &portable_ready("0.2.0"),
            Path::new("C:/Tools/postino.exe"),
        );
        assert_eq!(outcome, InstallOutcome::Fallback);
    }

    #[test]
    fn the_startup_check_needs_the_gate_and_the_setting() {
        assert!(should_check_at_startup(true, true));
        assert!(!should_check_at_startup(true, false));
        assert!(!should_check_at_startup(false, true));
    }

    #[test]
    fn a_check_runs_once_at_a_time() {
        let mut state = UpdateState::default();
        assert!(state.begin_check());
        assert!(!state.begin_check());
        state.fail_check();
        assert!(state.begin_check());
    }

    #[test]
    fn the_status_follows_the_last_check() {
        let mut state = UpdateState::default();
        assert_eq!(state.status(), CheckStatus::Idle);
        state.begin_check();
        assert_eq!(state.status(), CheckStatus::Checking);
        state.fail_check();
        assert_eq!(state.status(), CheckStatus::Failed);
        state.begin_check();
        state.finish_check(&CheckOutcome::UpToDate);
        assert_eq!(state.status(), CheckStatus::UpToDate);
        state.begin_check();
        state.finish_check(&CheckOutcome::Ready(ready("0.2.0")));
        assert_eq!(state.status(), CheckStatus::Idle);
    }

    #[test]
    fn up_to_date_leaves_nothing_ready() {
        let mut state = UpdateState::default();
        state.begin_check();
        state.finish_check(&CheckOutcome::UpToDate);
        assert_eq!(state.ready(), None);
        assert!(state.begin_check());
    }

    #[test]
    fn a_ready_update_is_kept_and_stops_further_checks() {
        let mut state = UpdateState::default();
        state.begin_check();
        state.finish_check(&CheckOutcome::Ready(ready("0.2.0")));
        assert_eq!(state.ready(), Some(&ready("0.2.0")));
        assert!(!state.begin_check());
        state.finish_check(&CheckOutcome::Ready(ready("0.3.0")));
        assert_eq!(state.ready(), Some(&ready("0.2.0")));
    }

    #[test]
    fn install_needs_a_ready_update_and_runs_once() {
        let mut state = UpdateState::default();
        assert_eq!(state.begin_install(), None);
        state.finish_check(&CheckOutcome::Ready(ready("0.2.0")));
        assert_eq!(state.begin_install(), Some(ready("0.2.0")));
        assert_eq!(state.begin_install(), None);
        state.fail_install();
        assert_eq!(state.begin_install(), Some(ready("0.2.0")));
    }

    #[test]
    fn the_install_method_follows_the_platform() {
        assert_eq!(
            install_method(Platform::WindowsX86_64),
            InstallMethod::WindowsInstaller
        );
        assert_eq!(
            install_method(Platform::MacosAarch64),
            InstallMethod::MacosSwap
        );
    }

    #[test]
    fn macos_outside_a_bundle_falls_back_to_the_release_page() {
        let outcome = start_install(
            &CONFIG,
            Platform::MacosAarch64,
            &ready("0.2.0"),
            Path::new("/usr/local/bin/postino"),
        );
        assert_eq!(outcome, InstallOutcome::Fallback);
    }
}
