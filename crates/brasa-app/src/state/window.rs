//! The size of the main window, remembered between sessions in `<config dir>/brasa/window.toml`,
//! next to the settings (#214). Only the size and whether it was maximized: the position is left
//! to the window manager (Wayland does not allow choosing it). Reading or writing it never
//! interrupts the user: errors are only logged.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::config::config_dir;

/// The file, relative to the config directory.
const WINDOW_FILE: &str = "window.toml";

/// The window as it was left, in logical pixels. A maximized window keeps the size it goes back to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

impl WindowState {
    /// The size to open with: as saved, but inside the visible area of the display (the display
    /// may be another one now, `None` when the OS does not report it yet) and never under the
    /// window's minimum size.
    pub fn fitted(&self, visible: Option<(f32, f32)>, min: (f32, f32)) -> (f32, f32) {
        let (max_width, max_height) = visible.unwrap_or((f32::MAX, f32::MAX));
        (
            self.width.min(max_width).max(min.0),
            self.height.min(max_height).max(min.1),
        )
    }
}

/// Loads the saved window, `None` the first time or when the file is unreadable.
pub fn load() -> Option<WindowState> {
    read(&config_dir()?)
}

/// Saves the window. Runs once, when it closes.
pub fn save(state: &WindowState) {
    if let Some(base) = config_dir() {
        write(&base, state);
    }
}

fn read(base: &Path) -> Option<WindowState> {
    let content = fs::read_to_string(base.join(WINDOW_FILE)).ok()?;
    match toml::from_str::<WindowState>(&content) {
        Ok(state) if state.width.is_finite() && state.height.is_finite() => Some(state),
        Ok(_) => {
            log::warn!("ignoring {WINDOW_FILE}: the size is not a number");
            None
        }
        Err(error) => {
            log::warn!("ignoring {WINDOW_FILE}: {error}");
            None
        }
    }
}

fn write(base: &Path, state: &WindowState) {
    let result = toml::to_string(state)
        .map_err(|error| error.to_string())
        .and_then(|text| {
            fs::create_dir_all(base).map_err(|error| error.to_string())?;
            fs::write(base.join(WINDOW_FILE), text).map_err(|error| error.to_string())
        });
    if let Err(error) = result {
        log::warn!("cannot save {WINDOW_FILE}: {error}");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const MIN: (f32, f32) = (760.0, 480.0);

    fn state(width: f32, height: f32) -> WindowState {
        WindowState {
            width,
            height,
            maximized: false,
        }
    }

    #[test]
    fn round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()), None);
        let saved = WindowState {
            width: 1100.5,
            height: 700.0,
            maximized: true,
        };
        write(dir.path(), &saved);
        assert_eq!(read(dir.path()), Some(saved));
    }

    #[test]
    fn a_broken_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(WINDOW_FILE), "width = \"big\"").unwrap();
        assert_eq!(read(dir.path()), None);
        fs::write(dir.path().join(WINDOW_FILE), "width = nan\nheight = 700.0").unwrap();
        assert_eq!(read(dir.path()), None);
    }

    #[test]
    fn the_size_is_kept_when_it_fits() {
        let fitted = state(1100.0, 700.0).fitted(Some((1920.0, 1080.0)), MIN);
        assert_eq!(fitted, (1100.0, 700.0));
        assert_eq!(state(1100.0, 700.0).fitted(None, MIN), (1100.0, 700.0));
    }

    #[test]
    fn the_size_fits_a_smaller_display_and_respects_the_minimum() {
        let fitted = state(2500.0, 1400.0).fitted(Some((1920.0, 1080.0)), MIN);
        assert_eq!(fitted, (1920.0, 1080.0));
        assert_eq!(state(100.0, 100.0).fitted(None, MIN), MIN);
        // A display under the minimum: the minimum wins, as the window could not be smaller.
        assert_eq!(state(1000.0, 800.0).fitted(Some((640.0, 400.0)), MIN), MIN);
    }
}
