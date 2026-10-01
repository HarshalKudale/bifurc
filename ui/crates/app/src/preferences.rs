//! This client's own preferences: how the window looks, not anything about the workspace.
//!
//! They are the UI's to keep (the engine neither knows nor cares which theme a client uses), so they live
//! in a small file beside the engine's data and are applied before the first frame.

use std::path::PathBuf;

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

use crate::theme;

const FILE_NAME: &str = "desktop-ui.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// The theme's name as the registry lists it. `None` follows the kit's default for the system appearance.
    pub theme: Option<String>,
    pub font_size: f32,
    pub sidebar_collapsed: bool,
    pub folder_tree_open: bool,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: None,
            font_size: theme::DEFAULT_FONT_SIZE,
            sidebar_collapsed: false,
            folder_tree_open: true,
            path: None,
        }
    }
}

impl Global for Preferences {}

impl Preferences {
    /// Reads the preferences kept in `data_dir`, falling back to the defaults for a missing or unreadable file.
    pub fn load(data_dir: PathBuf) -> Self {
        let path = data_dir.join(FILE_NAME);
        let mut preferences: Self =
            std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        if !theme::FONT_SIZES.contains(&preferences.font_size) {
            preferences.font_size = theme::DEFAULT_FONT_SIZE;
        }
        preferences.path = Some(path);
        preferences
    }

    /// Makes the window match the stored preferences.
    pub fn apply(&self, cx: &mut App) {
        if let Some(name) = &self.theme {
            theme::apply(name, cx);
        }
        theme::set_font_size(self.font_size, cx);
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Changes the preferences and writes them. A failed write is not reported: the choice still holds for
    /// this run, and nothing the user is working on depends on it.
    pub fn update(cx: &mut App, change: impl FnOnce(&mut Self)) {
        let preferences = cx.global_mut::<Self>();
        change(preferences);
        if let (Some(path), Ok(text)) = (&preferences.path, serde_json::to_string_pretty(preferences)) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, text);
        }
    }
}
