//! Exports a sidebar folder (or the whole workspace) as another tool's collection, from the
//! folder's context menu ("Export as..."). Mirrors `views/load_test/export.rs`'s native save
//! dialog, without its progress tracking: writing a collection is near instant, there is nothing
//! worth a progress bar for.
//!
//! Postman and Insomnia are one file, saved with a native save-file dialog. Bruno is a directory
//! tree (a `bruno.json`, one `.bru` per request, a `folder.bru` per subfolder), so it asks for a
//! parent folder instead and creates its own named subfolder inside it, the same shape
//! `brasa_workspace::create_workspace` already uses for a new workspace, refusing one that
//! already exists and is not empty rather than silently merging into it.

use std::io;
use std::path::{Path, PathBuf};

use gpui::PathPromptOptions;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_format::export::bruno::{BrunoFile, export_bruno_collection};
use brasa_format::export::insomnia::export_insomnia_collection;
use brasa_format::export::postman::export_postman_collection;
use brasa_workspace::sanitize_file_name;

use crate::state::format::shorten_path;

use super::root::AppView;

impl AppView {
    /// Asks where to save `folder_id` (the whole workspace when `None`) as a Postman collection,
    /// then writes it there. `folder_name` becomes both the suggested file name and the
    /// collection's own `info.name`.
    pub(crate) fn export_collection_as_postman(
        &mut self,
        folder_id: Option<String>,
        folder_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        let tree = match workspace.export_tree(folder_id.as_deref()) {
            Ok(tree) => tree,
            Err(error) => {
                log::warn!("collection export: could not read {folder_id:?}: {error}");
                return;
            }
        };
        let directory = workspace.root().to_path_buf();
        let file_name = format!("{}.postman_collection.json", sanitize_file_name(&folder_name));
        let receiver = cx.prompt_for_new_path(&directory, Some(&file_name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = receiver.await else {
                return;
            };
            let text = export_postman_collection(&folder_name, &tree);
            let outcome = cx
                .background_spawn(async move { std::fs::write(&path, &text).map(|()| path) })
                .await;
            let _ = this.update(cx, |view, cx| match outcome {
                Ok(path) => {
                    let shown = shorten_path(&path, dirs::home_dir().as_deref());
                    view.show_toast(t!("shell.sidebar.export_saved", path = shown), cx);
                }
                Err(error) => log::warn!("collection export: failed to write: {error}"),
            });
        })
        .detach();
    }

    /// Asks where to save `folder_id` (the whole workspace when `None`) as an Insomnia export,
    /// then writes it there. See [`Self::export_collection_as_postman`], the same shape.
    pub(crate) fn export_collection_as_insomnia(
        &mut self,
        folder_id: Option<String>,
        folder_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        let tree = match workspace.export_tree(folder_id.as_deref()) {
            Ok(tree) => tree,
            Err(error) => {
                log::warn!("collection export: could not read {folder_id:?}: {error}");
                return;
            }
        };
        let directory = workspace.root().to_path_buf();
        let file_name = format!("{}.insomnia.json", sanitize_file_name(&folder_name));
        let receiver = cx.prompt_for_new_path(&directory, Some(&file_name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = receiver.await else {
                return;
            };
            let exported_at = chrono::Utc::now().to_rfc3339();
            let text = export_insomnia_collection(&folder_name, &tree, &exported_at);
            let outcome = cx
                .background_spawn(async move { std::fs::write(&path, &text).map(|()| path) })
                .await;
            let _ = this.update(cx, |view, cx| match outcome {
                Ok(path) => {
                    let shown = shorten_path(&path, dirs::home_dir().as_deref());
                    view.show_toast(t!("shell.sidebar.export_saved", path = shown), cx);
                }
                Err(error) => log::warn!("collection export: failed to write: {error}"),
            });
        })
        .detach();
    }

    /// Asks for a parent folder, then writes `folder_id` (the whole workspace when `None`) as a
    /// Bruno collection in its own named subfolder of it. Refuses a subfolder that already
    /// exists and is not empty, the same rule `create_workspace` uses for a new workspace.
    pub(crate) fn export_collection_as_bruno(
        &mut self,
        folder_id: Option<String>,
        folder_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        let tree = match workspace.export_tree(folder_id.as_deref()) {
            Ok(tree) => tree,
            Err(error) => {
                log::warn!("collection export: could not read {folder_id:?}: {error}");
                return;
            }
        };
        let options = PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        };
        let receiver = cx.prompt_for_paths(options);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut parents))) = receiver.await else {
                return;
            };
            let Some(parent) = parents.pop() else {
                return;
            };
            let target = parent.join(sanitize_file_name(&folder_name));
            let files = export_bruno_collection(&folder_name, &tree);
            let outcome = cx
                .background_spawn(async move { write_bruno_collection(&target, &files) })
                .await;
            let _ = this.update(cx, |view, cx| match outcome {
                Ok(path) => {
                    let shown = shorten_path(&path, dirs::home_dir().as_deref());
                    view.show_toast(t!("shell.sidebar.export_saved", path = shown), cx);
                }
                Err(error) => {
                    log::warn!("collection export: failed to write Bruno collection: {error}");
                }
            });
        })
        .detach();
    }
}

/// Writes `files` under the new directory `target`, failing if it already exists: an export
/// never merges into or overwrites an existing folder.
fn write_bruno_collection(target: &Path, files: &[BrunoFile]) -> io::Result<PathBuf> {
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", target.display()),
        ));
    }
    std::fs::create_dir_all(target)?;
    for file in files {
        let path = target.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.content)?;
    }
    Ok(target.to_path_buf())
}
