//! The pieces every app of the family shares on the command line: the `<App> X.Y.Z` version line,
//! the common help header and footer, the standard exit codes and the Windows console attach.
//! Parsing stays in each app, which adds its own commands.

use crate::AppIdentity;

/// Exit code of a command that did what it was asked.
pub const EXIT_SUCCESS: u8 = 0;

/// Exit code of a command line that could not be understood (unknown option, missing value).
pub const EXIT_USAGE: u8 = 2;

/// `<App> X.Y.Z`, the first line of `--version` and of `--help`.
pub fn version_line(identity: &AppIdentity) -> String {
    format!("{} {}", identity.name, identity.version)
}

/// The top of a `--help` text: the version line followed by the one line description.
pub fn help_header(identity: &AppIdentity, tagline: &str) -> String {
    format!("{}\n{tagline}\n", version_line(identity))
}

/// The bottom of a `--help` text: the link to the command line documentation.
pub fn help_footer(docs_url: &str) -> String {
    format!("Documentation: {docs_url}\n")
}

/// Lets a Windows GUI subsystem binary print to the console it was started from. Without it,
/// stdout goes nowhere. Does nothing when there is no parent console (a double click). The
/// shell does not wait for a GUI program, so the output lands after its prompt.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: takes a plain process id and touches no memory of ours; failure is harmless.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Other platforms keep the terminal's stdout.
#[cfg(not(windows))]
pub fn attach_parent_console() {}

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
    fn the_version_line_is_the_name_and_the_version() {
        assert_eq!(version_line(&IDENTITY), "App 1.2.3");
    }

    #[test]
    fn the_help_header_and_footer_end_in_a_newline() {
        assert_eq!(
            help_header(&IDENTITY, "Does things."),
            "App 1.2.3\nDoes things.\n"
        );
        assert_eq!(
            help_footer("https://x.test"),
            "Documentation: https://x.test\n"
        );
    }

    #[test]
    fn the_exit_codes_are_the_conventional_ones() {
        assert_eq!(EXIT_SUCCESS, 0);
        assert_eq!(EXIT_USAGE, 2);
    }
}
