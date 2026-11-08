//! Running the git of a WSL distribution from Windows (telaio issue #40).
//!
//! Windows cannot start the Linux `git` binary itself, so every command goes through `wsl.exe`:
//! `wsl.exe -d <distro> --cd <linux path> --exec env ... git <args>`. The repository root the app
//! sees stays a Windows path (`\\wsl.localhost\<distro>\home\me\repo`), and only the working
//! directory handed to `wsl.exe` is translated. The path functions here are pure and work on
//! every platform; [`crate::detect_for`] only uses WSL on Windows.

use std::path::{Path, PathBuf};

/// The host names of the network share Windows serves the WSL file systems on.
const HOSTS: [&str; 2] = ["wsl.localhost", "wsl$"];

/// A folder inside the file system of a WSL distribution, as Windows names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslLocation {
    /// The share host as written, `wsl.localhost` or `wsl$`, kept to build paths of the same
    /// form.
    pub host: String,
    /// The distribution, for example `Ubuntu-24.04`.
    pub distro: String,
    /// The same folder inside the distribution, for example `/home/me/repo`.
    pub linux_path: String,
}

/// Where `path` is inside WSL, when it is a path of the WSL share:
/// `\\wsl.localhost\<distro>\...`, `\\wsl$\<distro>\...`, their `\\?\UNC\` forms, or the same
/// with forward slashes. The host is matched without regard to case.
pub fn wsl_location(path: &Path) -> Option<WslLocation> {
    let text = path.to_str()?.replace('/', "\\");
    let rest = text
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| text.strip_prefix(r"\\"))?;
    let (host, rest) = rest.split_once('\\')?;
    let host = HOSTS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(host))?;
    let (distro, inner) = rest.split_once('\\').unwrap_or((rest, ""));
    if distro.is_empty() || distro == "?" || distro == "." {
        return None;
    }
    let inner = inner.trim_end_matches('\\').replace('\\', "/");
    Some(WslLocation {
        host: (*host).to_string(),
        distro: distro.to_string(),
        linux_path: format!("/{inner}"),
    })
}

/// The Windows path of `linux_path`, a path printed by the git of `distro`: `/mnt/c/...` (the
/// default automount root) is the drive `C:\...`, anything else is inside the distribution,
/// on the share `host`.
pub(crate) fn to_windows(linux_path: &str, distro: &str, host: &str) -> PathBuf {
    if let Some(rest) = linux_path.strip_prefix("/mnt/") {
        let (drive, inner) = rest.split_once('/').unwrap_or((rest, ""));
        let mut letters = drive.chars();
        if let (Some(letter), None) = (letters.next(), letters.next())
            && letter.is_ascii_alphabetic()
        {
            let inner = inner.trim_end_matches('/').replace('/', "\\");
            return PathBuf::from(format!("{}:\\{inner}", letter.to_ascii_uppercase()));
        }
    }
    let inner = linux_path
        .trim_start_matches('/')
        .trim_end_matches('/')
        .replace('/', "\\");
    let path = format!(r"\\{host}\{distro}\{inner}");
    PathBuf::from(path.trim_end_matches('\\').to_string())
}

/// The folder to pass to `wsl.exe --cd`: the Linux path of a folder on the WSL share, any other
/// path as it is (`wsl.exe` translates absolute Windows paths such as `C:\work` itself).
pub(crate) fn cd_argument(dir: &Path) -> String {
    match wsl_location(dir) {
        Some(location) => location.linux_path,
        None => dir.to_string_lossy().into_owned(),
    }
}

/// Text printed by `wsl.exe` itself (as opposed to git) is UTF-16, such as "There is no
/// distribution with the supplied name". Decodes it when it looks like UTF-16, and as UTF-8
/// otherwise.
pub(crate) fn decode_message(bytes: &[u8]) -> String {
    let utf16 = bytes.len() >= 2
        && bytes.len().is_multiple_of(2)
        && bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count() * 2 >= bytes.len() / 2;
    if !utf16 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_start_matches('\u{feff}')
        .to_string()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn location(host: &str, distro: &str, linux_path: &str) -> Option<WslLocation> {
        Some(WslLocation {
            host: host.to_string(),
            distro: distro.to_string(),
            linux_path: linux_path.to_string(),
        })
    }

    #[test]
    fn recognizes_the_paths_of_the_wsl_share() {
        let at = |text: &str| wsl_location(Path::new(text));
        assert_eq!(
            at(r"\\wsl.localhost\Ubuntu-24.04\home\me\repo"),
            location("wsl.localhost", "Ubuntu-24.04", "/home/me/repo")
        );
        assert_eq!(
            at(r"\\WSL$\Ubuntu\home\me\"),
            location("wsl$", "Ubuntu", "/home/me")
        );
        assert_eq!(
            at(r"\\?\UNC\wsl.localhost\Debian\srv"),
            location("wsl.localhost", "Debian", "/srv")
        );
        assert_eq!(
            at("//wsl.localhost/Ubuntu/usr/bin/git"),
            location("wsl.localhost", "Ubuntu", "/usr/bin/git")
        );
        assert_eq!(
            at(r"\\wsl.localhost\Ubuntu"),
            location("wsl.localhost", "Ubuntu", "/")
        );
        assert_eq!(at(r"\\server\share\repo"), None);
        assert_eq!(at(r"C:\Users\me\repo"), None);
        assert_eq!(at("/home/me/repo"), None);
        assert_eq!(at(r"\\wsl.localhost\"), None);
    }

    #[test]
    fn translates_linux_paths_back_to_windows() {
        assert_eq!(
            to_windows("/home/me/repo", "Ubuntu-24.04", "wsl.localhost"),
            PathBuf::from(r"\\wsl.localhost\Ubuntu-24.04\home\me\repo")
        );
        assert_eq!(
            to_windows("/home/me/repo/", "Ubuntu", "wsl$"),
            PathBuf::from(r"\\wsl$\Ubuntu\home\me\repo")
        );
        assert_eq!(
            to_windows("/mnt/c/Users/me/repo", "Ubuntu", "wsl.localhost"),
            PathBuf::from(r"C:\Users\me\repo")
        );
        assert_eq!(
            to_windows("/mnt/d", "Ubuntu", "wsl.localhost"),
            PathBuf::from(r"D:\")
        );
        // Not a drive letter: a folder of the distribution that happens to be under /mnt.
        assert_eq!(
            to_windows("/mnt/data/repo", "Ubuntu", "wsl.localhost"),
            PathBuf::from(r"\\wsl.localhost\Ubuntu\mnt\data\repo")
        );
    }

    #[test]
    fn the_working_directory_is_the_linux_path_on_the_share() {
        assert_eq!(
            cd_argument(Path::new(r"\\wsl.localhost\Ubuntu\home\me\repo")),
            "/home/me/repo"
        );
        assert_eq!(cd_argument(Path::new(r"C:\work\repo")), r"C:\work\repo");
    }

    #[test]
    fn decodes_the_utf16_messages_of_wsl() {
        let text = "There is no distribution with the supplied name.\r\n";
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_message(&bytes), text);
        assert_eq!(
            decode_message(b"fatal: not a git repository"),
            "fatal: not a git repository"
        );
        assert_eq!(decode_message("ñandú".as_bytes()), "ñandú");
    }

    /// A stand-in for `wsl.exe`: checks the arguments `Runner::Wsl` passes, goes to `--cd` and
    /// runs the rest with the git of this machine. An unknown distribution fails like `wsl.exe`
    /// does, with a UTF-16 message.
    #[cfg(unix)]
    const FAKE_WSL: &str = r#"#!/bin/sh
[ "$1" = -d ] || exit 90
if [ "$2" != Test ]; then
    printf 'N\000o\000 \000d\000i\000s\000t\000r\000o\000\n\000' >&2
    exit 1
fi
shift 2
if [ "$1" = --cd ]; then cd "$2" || exit 92; shift 2; fi
[ "$1" = --exec ] || exit 93
shift
exec "$@"
"#;

    #[cfg(unix)]
    fn fake_wsl(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("wsl.exe");
        std::fs::write(&path, FAKE_WSL).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    fn runner(launcher: &Path, distro: &str) -> crate::run::Runner {
        crate::run::Runner::Wsl {
            launcher: launcher.to_path_buf(),
            distro: distro.to_string(),
            program: "git".to_string(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn runs_git_through_wsl_and_keeps_the_windows_root() {
        let Ok(native) = crate::detect(&crate::Executable::Auto) else {
            eprintln!("skipping, no usable git");
            return;
        };
        let tools = tempfile::tempdir().unwrap();
        let launcher = fake_wsl(tools.path());
        let git = crate::detect::detect_with(runner(&launcher, "Test"), PathBuf::from("git"))
            .expect("git through the fake wsl");
        assert_eq!(git.version(), native.version());
        assert_eq!(git.wsl_distro(), Some("Test"));

        let repo_dir = tempfile::tempdir().unwrap();
        let linux_root = dunce::canonicalize(repo_dir.path()).unwrap();
        let linux_root = linux_root.to_str().unwrap();
        git.init(&to_windows(linux_root, "Test", "wsl$")).unwrap();
        std::fs::create_dir(repo_dir.path().join("api")).unwrap();
        std::fs::write(repo_dir.path().join("api/a.txt"), "a\n").unwrap();

        // The app opens a folder of the share, and gets the root on the same share back.
        let folder = to_windows(&format!("{linux_root}/api"), "Test", "wsl$");
        let repo = git.discover(&folder).unwrap().expect("a repository");
        assert_eq!(repo.root(), to_windows(linux_root, "Test", "wsl$"));
        assert_eq!(repo.prefix(), "api");

        // Commands run in the Linux root, with input on stdin.
        repo.stage(&["api/a.txt".to_string()]).unwrap();
        let status = repo.status().unwrap();
        assert_eq!(status.entries.len(), 1);
        assert_eq!(status.entries[0].path, "api/a.txt");
    }

    #[cfg(unix)]
    #[test]
    fn an_unknown_distribution_fails_with_the_message_of_wsl() {
        let tools = tempfile::tempdir().unwrap();
        let launcher = fake_wsl(tools.path());
        let result = crate::detect::detect_with(runner(&launcher, "Nope"), PathBuf::from("git"));
        match result {
            Err(crate::DetectError::Failed(message)) => {
                assert!(message.contains("No distro"), "{message}");
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }
}
