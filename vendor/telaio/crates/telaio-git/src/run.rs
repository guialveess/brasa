//! The one place that spawns `git`, so every command gets the same environment and logging.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::GitError;
use crate::wsl;

/// How a git command is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Runner {
    /// A git executable of this machine.
    Native(PathBuf),
    /// The git of a WSL distribution, started from Windows through `launcher` (`wsl.exe`).
    Wsl {
        /// `wsl.exe`, or a stand-in in tests.
        launcher: PathBuf,
        /// The distribution, for `-d`.
        distro: String,
        /// The git to run inside it: `git` (from its `PATH`) or a Linux path.
        program: String,
    },
}

impl Runner {
    /// The git of `distro`, through the `wsl.exe` of the `PATH`.
    pub(crate) fn wsl(distro: &str, program: &str) -> Runner {
        Runner::Wsl {
            launcher: PathBuf::from("wsl.exe"),
            distro: distro.to_string(),
            program: program.to_string(),
        }
    }

    /// The command that runs git with `args` in `cwd`, before the shared options and
    /// environment are added.
    fn command(&self, cwd: Option<&Path>) -> Command {
        match self {
            Runner::Native(path) => {
                let mut cmd = Command::new(path);
                if let Some(cwd) = cwd {
                    cmd.current_dir(cwd);
                }
                cmd
            }
            Runner::Wsl {
                launcher,
                distro,
                program,
            } => {
                let mut cmd = Command::new(launcher);
                cmd.args(["-d", distro.as_str()]);
                if let Some(cwd) = cwd {
                    cmd.arg("--cd").arg(wsl::cd_argument(cwd));
                }
                // The environment of Windows does not reach Linux (only through `WSLENV`), so
                // `env` sets it inside the distribution.
                cmd.args([
                    "--exec",
                    "env",
                    "-u",
                    "LC_ALL",
                    "-u",
                    "LANGUAGE",
                    "GIT_TERMINAL_PROMPT=0",
                    "LC_MESSAGES=C",
                    program.as_str(),
                ]);
                cmd
            }
        }
    }

    /// Text printed on a failure: `wsl.exe`'s own messages are UTF-16.
    fn message(&self, bytes: &[u8]) -> String {
        match self {
            Runner::Native(_) => String::from_utf8_lossy(bytes).into_owned(),
            Runner::Wsl { .. } => wsl::decode_message(bytes),
        }
    }
}

/// Runs `git` with the global options and environment every invocation shares, and returns its
/// standard output. A non zero exit becomes [`GitError::Failed`].
///
/// Messages are forced to English (`LC_MESSAGES=C`) so the few places that look at stderr can
/// rely on them, and git never waits for a terminal prompt.
pub(crate) fn run(git: &Runner, cwd: Option<&Path>, args: &[&str]) -> Result<Vec<u8>, GitError> {
    run_with(git, cwd, args, None, &[])
}

/// Like [`run`], with an optional payload for the standard input and a list of non zero exit
/// codes that still count as success (`git diff --no-index` exits with 1 when files differ).
pub(crate) fn run_with(
    git: &Runner,
    cwd: Option<&Path>,
    args: &[&str],
    stdin: Option<&[u8]>,
    ok_codes: &[i32],
) -> Result<Vec<u8>, GitError> {
    let mut cmd = git.command(cwd);
    cmd.args([
        "-c",
        "core.quotepath=false",
        "--no-optional-locks",
        "--literal-pathspecs",
    ])
    .args(args)
    .env("GIT_TERMINAL_PROMPT", "0")
    .env("LC_MESSAGES", "C")
    .env_remove("LC_ALL")
    .env_remove("LANGUAGE")
    .stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW, so no console window flashes for every command.
        cmd.creation_flags(0x0800_0000);
    }

    let command = format!("git {}", args.join(" "));
    let started = Instant::now();
    let output = match stdin {
        None => cmd.output()?,
        Some(payload) => {
            let mut child = cmd.spawn()?;
            let pipe = child.stdin.take();
            // Written from another thread while this one drains stdout and stderr, so a large
            // payload and a large output can never block each other.
            std::thread::scope(|scope| {
                scope.spawn(move || {
                    if let Some(mut pipe) = pipe {
                        use std::io::Write;
                        // A broken pipe means git stopped reading, its exit status says why.
                        let _ = pipe.write_all(payload);
                    }
                });
                child.wait_with_output()
            })?
        }
    };
    log::debug!("{command} ({} ms)", started.elapsed().as_millis());

    let accepted = output.status.code().is_some_and(|c| ok_codes.contains(&c));
    if output.status.success() || accepted {
        Ok(output.stdout)
    } else {
        Err(GitError::Failed {
            command,
            code: output.status.code(),
            stderr: git.message(&output.stderr).trim().to_string(),
            stdout: git.message(&output.stdout).trim().to_string(),
        })
    }
}
