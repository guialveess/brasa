//! Finding the `git` executable and checking its version.

use std::env;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::run::{Runner, run};
use crate::{DetectError, Git, GitError, wsl_location};

/// The oldest git the apps work with.
pub const MIN_VERSION: Version = Version {
    major: 2,
    minor: 30,
    patch: 0,
};

/// Where to look for git, as chosen in the settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Executable {
    /// Search the `PATH` (and the usual install folder on Windows). On Windows, with a folder
    /// inside WSL, the git of that distribution instead (see [`detect_for`]).
    Auto,
    /// Use this executable. On Windows, a path inside WSL
    /// (`\\wsl.localhost\Ubuntu\usr\bin\git`) runs that git through `wsl.exe`.
    Path(PathBuf),
}

/// A git version, `major.minor.patch`. Anything after the patch number is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    /// The major number.
    pub major: u32,
    /// The minor number.
    pub minor: u32,
    /// The patch number, 0 when git does not print one.
    pub patch: u32,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Version {
    /// Parses the output of `git --version`, such as `git version 2.47.1`,
    /// `git version 2.47.1.windows.2` or `git version 2.39.3 (Apple Git-146)`.
    pub fn parse(output: &str) -> Option<Version> {
        let token = output
            .trim()
            .strip_prefix("git version")?
            .split_whitespace()
            .next()?;
        let mut numbers = token.split('.').map_while(|part| part.parse::<u32>().ok());
        Some(Version {
            major: numbers.next()?,
            minor: numbers.next()?,
            patch: numbers.next().unwrap_or(0),
        })
    }
}

/// Finds a usable git and returns a handle to it. Same as [`detect_for`] without a folder, so
/// never the git of WSL for [`Executable::Auto`].
pub fn detect(exe: &Executable) -> Result<Git, DetectError> {
    detect_for(exe, None)
}

/// Finds a usable git for the repositories under `folder` (the workspace the app opened) and
/// returns a handle to it.
///
/// On Windows, with [`Executable::Auto`] and a `folder` inside WSL
/// (`\\wsl.localhost\<distro>\...` or `\\wsl$\<distro>\...`), that is the git of the
/// distribution, run through `wsl.exe`: it reads the repository on its own file system, with the
/// user's Linux configuration and keys. Git for Windows is not needed then. A git found this way
/// belongs to that distribution, so detect again when the folder moves to another one
/// ([`Git::wsl_distro`] against [`wsl_location`] of the new folder).
pub fn detect_for(exe: &Executable, folder: Option<&Path>) -> Result<Git, DetectError> {
    let wsl = |path: &Path| wsl_location(path).filter(|_| cfg!(windows));
    let (runner, path) = match exe {
        Executable::Auto => match folder.and_then(wsl) {
            Some(location) => (Runner::wsl(&location.distro, "git"), PathBuf::from("git")),
            None => {
                let path = find_in_path().ok_or(DetectError::NotFound)?;
                (Runner::Native(path.clone()), path)
            }
        },
        Executable::Path(path) => match wsl(path) {
            Some(location) => (
                Runner::wsl(&location.distro, &location.linux_path),
                path.clone(),
            ),
            None if path.is_file() => (Runner::Native(path.clone()), path.clone()),
            None => return Err(DetectError::NotFound),
        },
    };
    detect_with(runner, path)
}

/// Runs `git --version` with `runner` and checks it.
pub(crate) fn detect_with(runner: Runner, path: PathBuf) -> Result<Git, DetectError> {
    guard_xcode_shim(&path)?;

    let output = match run(&runner, None, &["--version"]) {
        Ok(output) => output,
        Err(GitError::Spawn(err)) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(DetectError::NotFound);
        }
        Err(err) => return Err(DetectError::Failed(err.to_string())),
    };
    let text = String::from_utf8_lossy(&output);
    let version = Version::parse(&text).ok_or_else(|| {
        DetectError::Failed(format!("unexpected version output {:?}", text.trim()))
    })?;
    if version < MIN_VERSION {
        return Err(DetectError::TooOld { path, version });
    }
    Ok(Git::new(runner, path, version))
}

/// The executable file name of git on this platform.
fn file_name() -> &'static str {
    if cfg!(windows) { "git.exe" } else { "git" }
}

/// Searches every folder of `PATH`, then the default Git for Windows install folder.
fn find_in_path() -> Option<PathBuf> {
    let from_path = env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|dir| dir.join(file_name()))
            .find(|candidate| candidate.is_file())
    });
    if from_path.is_some() {
        return from_path;
    }
    if cfg!(windows) {
        let program_files = env::var_os("ProgramFiles")?;
        let candidate = PathBuf::from(program_files)
            .join("Git")
            .join("cmd")
            .join("git.exe");
        return candidate.is_file().then_some(candidate);
    }
    None
}

/// On macOS `/usr/bin/git` is a shim that opens the Xcode Command Line Tools installer when they
/// are missing, so check for them first and never run the shim in that case.
fn guard_xcode_shim(path: &Path) -> Result<(), DetectError> {
    if cfg!(target_os = "macos") && path == Path::new("/usr/bin/git") {
        let installed = std::process::Command::new("xcode-select")
            .arg("-p")
            .stdin(std::process::Stdio::null())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if !installed {
            return Err(DetectError::XcodeToolsMissing);
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn v(major: u32, minor: u32, patch: u32) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn parses_the_usual_version_lines() {
        assert_eq!(Version::parse("git version 2.47.1\n"), Some(v(2, 47, 1)));
        assert_eq!(
            Version::parse("git version 2.47.1.windows.2"),
            Some(v(2, 47, 1))
        );
        assert_eq!(
            Version::parse("git version 2.39.3 (Apple Git-146)"),
            Some(v(2, 39, 3))
        );
        assert_eq!(Version::parse("git version 2.30"), Some(v(2, 30, 0)));
        assert_eq!(Version::parse("hello"), None);
        assert_eq!(Version::parse("git version x.y"), None);
    }

    #[test]
    fn versions_are_ordered_and_displayed() {
        assert!(v(2, 9, 0) < MIN_VERSION);
        assert!(v(2, 30, 0) >= MIN_VERSION);
        assert!(v(2, 100, 0) > v(2, 47, 1));
        assert_eq!(v(2, 47, 1).to_string(), "2.47.1");
    }

    #[test]
    fn a_missing_explicit_path_is_not_found() {
        let result = detect(&Executable::Path(PathBuf::from("/definitely/not/git")));
        assert!(matches!(result, Err(DetectError::NotFound)));
    }
}
