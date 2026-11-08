//! telaio-app: the application shell every app of the family shares, so a new app starts with it
//! working.
//!
//! - [`AppIdentity`]: the names, ids and version every piece below takes.
//! - [`logging`]: the log file behind the `log` facade, with rotation and a panic hook.
//! - [`cli`]: the version line, help header and footer, exit codes and Windows console attach.
//! - [`language`]: the four UI languages, OS language detection and the `auto` choice.
//! - [`number`]: numbers written with the active language's separators.
//! - [`paths`]: the app's config folder.
//! - [`window`]: small helpers to open the main window the same way in every app.
//! - [`about`]: the header of the About pane.
//!
//! The pieces are independent and composable: nothing here owns `main`.
//!
//! The strings of this crate (`locales/telaio_app.yml`) follow the locale the app sets with
//! `rust_i18n::set_locale`.

rust_i18n::i18n!("locales", fallback = "en");

pub mod about;
pub mod cli;
mod identity;
pub mod language;
mod locale;
pub mod logging;
pub mod number;
pub mod paths;
pub mod window;

pub use identity::AppIdentity;
