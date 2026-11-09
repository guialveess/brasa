# Brasa

A lightweight, native HTTP client for Linux, macOS and Windows, with no accounts, telemetry or
cloud storage. Requests live as plain local files, so you decide where they go: git, a shared
folder, or nowhere else.

Forked from [Postino](https://github.com/tanisperez/postino) by Estanislao Pérez Nartallo
(Apache-2.0), rebranded and restyled.

## Install

**macOS** (Apple Silicon):

```
curl -fsSL https://raw.githubusercontent.com/guialveess/brasa/main/install.sh | sh
```

**Windows** (x86_64; run in PowerShell, not WSL):

```
irm https://raw.githubusercontent.com/guialveess/brasa/main/install.ps1 | iex
```

Both download the latest release from [GitHub Releases](https://github.com/guialveess/brasa/releases)
for the current platform. The macOS build is ad-hoc signed, not notarized: `install.sh` clears
the quarantine flag the download carries so the first launch does not need a right-click, Open.

## Building from source

Install Rust with [`rustup`](https://rustup.rs). The toolchain version is pinned in
`rust-toolchain.toml` and installed on the first build. Then install the system dependencies for
your platform:

- **Linux** (Debian and Ubuntu package names):

  ```
  sudo apt install build-essential pkg-config cmake libfontconfig-dev libfreetype-dev \
      libwayland-dev wayland-protocols libxkbcommon-dev libxkbcommon-x11-dev \
      libx11-xcb-dev libxcb1-dev libvulkan-dev
  ```

- **macOS**: Xcode, with its command line tools.
- **Windows**: the Visual Studio Build Tools with the "Desktop development with C++" workload.

Build and run:

```
cargo run                    # debug build
cargo build --release        # optimized binary in target/release/brasa
cargo test --all-targets     # run the tests
```

`make run`, `make release`, `make test`, `make lint`, `make format` wrap the usual commands. Run
`make help` for the full list.

## Stack

Rust + [`gpui`](https://www.gpui.rs) (the UI engine behind the Zed editor), with
[`gpui-kit`](https://crates.io/crates/gpui-kit) for the widgets and
[telaio](https://github.com/tanisperez/telaio) for the design system plumbing (palette tokens,
theme builder, fonts).

## License

Apache License 2.0, inherited from the upstream Postino project. See [LICENSE](LICENSE) and
[NOTICE](NOTICE).
