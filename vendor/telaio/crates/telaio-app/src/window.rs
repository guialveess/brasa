//! Small helpers to start the app and open its main window the same way in every app. They are
//! meant to be called from the app's own `main`, which keeps its flow (settings, open queue,
//! root view, debug hooks):
//!
//! ```ignore
//! let app = telaio_app::window::application();
//! app.run(move |cx| {
//!     telaio_app::window::init(cx, &IDENTITY);
//!     // key bindings, telaio_ui::install(...)
//!     let options = telaio_app::window::options(cx, &IDENTITY, WindowSizes::default());
//!     cx.spawn(async move |cx| {
//!         cx.open_window(options, |window, cx| {
//!             choice.apply(Some(window), cx);
//!             let view = cx.new(|cx| MyView::new(window, cx));
//!             telaio_app::window::follow_system_appearance(window, |cx| ...);
//!             cx.new(|cx| Root::new(view, window, cx))
//!         })
//!     })
//!     .detach();
//! });
//! ```

use gpui_kit::component::TitleBar;
use gpui_kit::component::theme::Theme;
use gpui_kit::*;

use crate::AppIdentity;

/// The window size used when the OS reports no display at the time the window opens.
const FALLBACK_SIZE: (f32, f32) = (1280.0, 760.0);

/// How much of the display's visible area a window may take at most.
const MAX_DISPLAY_FRACTION: f32 = 0.9;

/// The sizes of the main window, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSizes {
    /// The size the window opens with, capped to 90% of the display's visible bounds.
    pub default: (f32, f32),
    /// The smallest size the user can resize it to.
    pub min: (f32, f32),
}

impl Default for WindowSizes {
    /// 1440x900, at least 760x480.
    fn default() -> Self {
        Self {
            default: (1440.0, 900.0),
            min: (760.0, 480.0),
        }
    }
}

/// The gpui application with the complete Lucide icon catalog registered.
///
/// `with_assets` registers the bundled icon SVGs: without it every `Icon` (tree chevrons, window
/// controls, checkboxes) renders as empty space. `gpui_kit::assets::Assets` only embeds a small
/// curated subset; `AllAssets` embeds the full catalog (`gpui_kit::assets::IconName`), a strict
/// superset of it.
pub fn application() -> Application {
    gpui_kit::application().with_assets(gpui_kit::assets::AllAssets)
}

/// Gives the OS the app's id and name (the AppUserModelID on Windows, the name for notifications
/// elsewhere) and runs `gpui_kit::init`. Call first inside `Application::run`.
pub fn init(cx: &mut App, identity: &AppIdentity) {
    cx.set_app_identity(identity.app_id, identity.name);
    gpui_kit::init(cx);
}

/// The options of the main window: centered at `sizes.default` capped to 90% of the display's
/// visible bounds, client side decorations on Linux so the `TitleBar` is the only one (`Root`
/// then draws the border and resize edges; other platforms ignore the option), and the app id as
/// the Wayland `app_id` and X11 `WM_CLASS`, which must match the desktop file for the right icon
/// and name in the dock and alt-tab.
///
/// `cx.primary_display()` can legitimately return nothing while the app starts: on Wayland the
/// compositor hands over the display only after the window's first configure event, which no
/// synchronous call at window creation can observe. Without a fallback, gpui's own default
/// placement can come out taller than the visible screen and hide the status bar, so a fixed
/// 1280x760 is used then.
pub fn options(cx: &App, identity: &AppIdentity, sizes: WindowSizes) -> WindowOptions {
    let window_bounds = match cx.primary_display() {
        Some(display) => {
            let visible = display.visible_bounds();
            let cap = size(
                visible.size.width * MAX_DISPLAY_FRACTION,
                visible.size.height * MAX_DISPLAY_FRACTION,
            );
            let target = size(px(sizes.default.0), px(sizes.default.1)).min(&cap);
            WindowBounds::Windowed(Bounds::centered_at(visible.center(), target))
        }
        None => WindowBounds::Windowed(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(FALLBACK_SIZE.0), px(FALLBACK_SIZE.1)),
        }),
    };
    WindowOptions {
        window_bounds: Some(window_bounds),
        window_min_size: Some(size(px(sizes.min.0), px(sizes.min.1))),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some(identity.app_id.to_string()),
        ..TitleBar::window_options()
    }
}

/// Keeps the theme in step with the OS light or dark setting while `follows_system` says the
/// app's current choice is `System`.
///
/// The observer is always attached, whatever the choice at startup: a settings view can switch to
/// `System` at any time and must start following the OS at once. Asking the app on every OS change
/// (instead of attaching only when the choice is `System`) is what lets `Light` and `Dark` stop
/// following it without detaching anything: the observer simply does nothing while they are
/// active. Idle windows do no work, since it only runs when the OS appearance changes.
pub fn follow_system_appearance(
    window: &mut Window,
    follows_system: impl Fn(&App) -> bool + 'static,
) {
    window
        .observe_window_appearance(move |window, cx| {
            if follows_system(cx) {
                Theme::sync_system_appearance(Some(window), cx);
            }
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::WindowSizes;
    use pretty_assertions::assert_eq;

    #[test]
    fn the_default_sizes_are_the_apps_usual_ones() {
        let sizes = WindowSizes::default();
        assert_eq!(sizes.default, (1440.0, 900.0));
        assert_eq!(sizes.min, (760.0, 480.0));
    }
}
