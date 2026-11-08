//! The command line of Brasa: the commands that run in the terminal and exit instead of
//! opening the window. No `gpui`, so everything is unit tested without a window.
//!
//! Only the first argument is read; anything that is not a command (no argument, a path, an
//! unknown flag) is left to the app, which treats it as the path to open.
//!
//! To add a command:
//! 1. Create a module with its type, a `parse` from the first argument, a `run` returning an
//!    exit code and its help text (see [`info`]).
//! 2. Register it in one place: a variant in [`Command`], a line in [`Command::parse`] and one
//!    in [`Command::run`], and mention it in the help of [`info`].
//! 3. Add its tests next to it.

pub mod info;

use std::ffi::OsStr;
use std::process::ExitCode;
use std::sync::LazyLock;

use telaio_app::AppIdentity;

pub use info::Info;

/// Who Brasa is, for every piece of `telaio-app` the app and the command line use: the name,
/// the lowercase name of folders and files, the application id (Wayland `app_id`, X11 `WM_CLASS`,
/// Windows AppUserModelID, the same as `dev.brasa.app.desktop` and the Windows installer
/// shortcut), the version, the environment variable prefix and the repository.
pub const IDENTITY: AppIdentity = AppIdentity {
    name: "Brasa",
    slug: "brasa",
    app_id: "dev.brasa.app",
    version: env!("CARGO_PKG_VERSION"),
    env_prefix: "BRASA",
    repository: "https://github.com/guialveess/brasa",
};

/// The short git commit this build was made from, captured by `build.rs` (or taken from the
/// `BRASA_COMMIT` environment variable at build time). `None` outside a repository, for example
/// a source tarball.
pub const COMMIT: Option<&str> = option_env!("BRASA_COMMIT");

/// The suffix of the versions between two releases (`0.4.0-dev`), whose builds show their commit.
const DEV_SUFFIX: &str = "-dev";

/// The version as shown to people: `0.4.0-dev (4e74e2d)` for a development build with a known
/// commit, the bare version otherwise. A release is named by its tag, so it never shows a commit.
/// The updater, the release notes link and anything machine readable use the bare version.
pub fn format_version_label(version: &str, commit: Option<&str>) -> String {
    match commit {
        Some(commit) if version.ends_with(DEV_SUFFIX) => format!("{version} ({commit})"),
        _ => version.to_string(),
    }
}

/// [`format_version_label`] of this build.
pub fn version_label() -> &'static str {
    static LABEL: LazyLock<&'static str> = LazyLock::new(|| {
        let label = format_version_label(IDENTITY.version, COMMIT);
        // `AppIdentity` wants a `&'static str`: leaked once, for the life of the process.
        Box::leak(label.into_boxed_str())
    });
    *LABEL
}

/// [`IDENTITY`] with the version label, for what only displays the version (About, the version
/// line). Pass [`IDENTITY`] itself to everything that needs the bare version.
pub fn display_identity() -> AppIdentity {
    AppIdentity {
        version: version_label(),
        ..IDENTITY
    }
}

/// A command line command that runs without opening the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// `-h`/`--help` and `-V`/`--version`.
    Info(Info),
}

impl Command {
    /// The command asked by the first argument, or `None` when Brasa should start normally.
    pub fn parse(first_arg: Option<&OsStr>) -> Option<Self> {
        let arg = first_arg?.to_str()?;
        Info::parse(arg).map(Command::Info)
    }

    /// Runs the command and returns the process exit code.
    pub fn run(self) -> ExitCode {
        let code = match self {
            Command::Info(info) => info.run(),
        };
        ExitCode::from(code)
    }
}

/// Handles the command line before anything else is set up. Returns `None` when the first
/// argument is not a command (the app starts normally), otherwise runs it, attaching to the
/// parent console first on Windows, and returns the exit code.
pub fn run_if_cli(first_arg: Option<&OsStr>) -> Option<ExitCode> {
    let command = Command::parse(first_arg)?;
    telaio_app::cli::attach_parent_console();
    Some(command.run())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_build_shows_its_commit_next_to_the_version() {
        assert_eq!(
            format_version_label("0.4.0-dev", Some("4e74e2d")),
            "0.4.0-dev (4e74e2d)"
        );
        assert_eq!(format_version_label("0.4.0-dev", None), "0.4.0-dev");
    }

    #[test]
    fn a_release_is_named_by_its_tag_only() {
        assert_eq!(format_version_label("0.4.0", Some("4e74e2d")), "0.4.0");
        assert_eq!(
            format_version_label("0.4.0-rc.1", Some("4e74e2d")),
            "0.4.0-rc.1"
        );
        assert_eq!(format_version_label("0.4.0", None), "0.4.0");
    }

    #[test]
    fn the_display_identity_only_changes_the_version() {
        let display = display_identity();
        assert_eq!(display.version, version_label());
        assert_eq!(display.name, IDENTITY.name);
        assert_eq!(display.repository, IDENTITY.repository);
        assert!(version_label().starts_with(IDENTITY.version));
    }

    #[test]
    fn no_argument_or_a_path_is_not_a_command() {
        assert_eq!(Command::parse(None), None);
        assert_eq!(Command::parse(Some(OsStr::new("/home/me/requests"))), None);
        assert_eq!(Command::parse(Some(OsStr::new("-x"))), None);
    }

    #[test]
    fn the_info_flags_are_commands() {
        assert_eq!(
            Command::parse(Some(OsStr::new("--help"))),
            Some(Command::Info(Info::Help))
        );
        assert_eq!(
            Command::parse(Some(OsStr::new("-V"))),
            Some(Command::Info(Info::Version))
        );
    }

    #[test]
    fn run_if_cli_ignores_what_is_not_a_command() {
        assert!(run_if_cli(None).is_none());
        assert!(run_if_cli(Some(OsStr::new("orders"))).is_none());
    }
}
