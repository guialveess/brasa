//! Embeds the release notes of this version (`release-notes/X.Y.Z.md`, #189) for the What's new
//! tab, and the Windows resources (the executable icon, `assets/brasa.rc`) into `brasa.exe`.

use std::path::Path;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    embed_release_notes();
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/brasa.rc");
        println!("cargo:rerun-if-changed=assets/brasa.ico");
        if let Err(error) =
            embed_resource::compile("assets/brasa.rc", embed_resource::NONE).manifest_optional()
        {
            panic!("failed to embed the Windows resources: {error}");
        }
    }
}

/// Writes `$OUT_DIR/release_notes.rs` with `NOTES`, the notes of the version without its
/// pre-release (`0.6.0-dev` reads `0.6.0.md`), or `None` when that file does not exist yet.
fn embed_release_notes() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let base = version.split('-').next().unwrap_or_default();
    let dir = Path::new(&manifest_dir).join("../../release-notes");
    println!("cargo:rerun-if-changed={}", dir.display());
    let notes = dir.join(format!("{base}.md"));
    let value = match fs::canonicalize(&notes) {
        Ok(path) => format!("Some(include_str!({:?}))", path.display().to_string()),
        Err(_) => "None".to_string(),
    };
    let out = Path::new(&env::var("OUT_DIR").unwrap_or_default()).join("release_notes.rs");
    let code = format!("pub const NOTES: Option<&str> = {value};\n");
    if let Err(error) = fs::write(&out, code) {
        panic!("failed to write {}: {error}", out.display());
    }
}
