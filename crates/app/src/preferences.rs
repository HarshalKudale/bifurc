//! This client's own preferences: how the window looks, not anything about the workspace.
//!
//! They are the UI's to keep (the engine neither knows nor cares which theme a client uses), so they live
//! in a small file beside the engine's data and are applied before the first frame.

use std::{collections::BTreeMap, path::PathBuf};

use gpui_kit::component::dock::{DockAreaState, PanelInfo};
use gpui_kit::{App, Bounds, Global, Pixels, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

use crate::theme;

const FILE_NAME: &str = "desktop-ui.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// The theme's name as the registry lists it. `None` follows the kit's default for the system appearance.
    pub theme: Option<String>,
    /// The language's code in `i18n::LANGUAGES`. `None` until the user has picked one in the first run.
    pub language: Option<String>,
    pub font_size: f32,
    pub sidebar_collapsed: bool,
    pub folder_tree_open: bool,
    /// Where the window was when it last closed. `None` until then: the window opens centered.
    pub window: Option<WindowState>,
    /// The size in pixels of each dock area's one fixed-size pane, by the area's id. The arrangement of the
    /// panes is the screen's code, not a preference, so a layout this build no longer has cannot be restored.
    pub dock_sizes: BTreeMap<String, f32>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: None,
            language: None,
            font_size: theme::DEFAULT_FONT_SIZE,
            sidebar_collapsed: false,
            folder_tree_open: true,
            window: None,
            dock_sizes: BTreeMap::new(),
            path: None,
        }
    }
}

/// A window's rectangle and whether it was maximized, in the physical pixels GPUI reports.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl WindowState {
    /// Fullscreen is kept as maximized: restoring into fullscreen at start would surprise more than help.
    pub fn of(bounds: WindowBounds) -> Self {
        let rect = bounds.get_bounds();
        Self {
            x: rect.origin.x.into(),
            y: rect.origin.y.into(),
            width: rect.size.width.into(),
            height: rect.size.height.into(),
            maximized: !matches!(bounds, WindowBounds::Windowed(_)),
        }
    }

    /// The bounds to open with, or `None` when the saved rectangle is no longer usable.
    pub fn restore(&self, displays: &[Bounds<Pixels>], min: (f32, f32)) -> Option<WindowBounds> {
        let rect = self.fit(&displays.iter().map(|d| Rect::of(*d)).collect::<Vec<_>>(), min)?;
        let bounds = Bounds::new(point(px(rect.x), px(rect.y)), size(px(rect.width), px(rect.height)));
        Some(if self.maximized { WindowBounds::Maximized(bounds) } else { WindowBounds::Windowed(bounds) })
    }

    /// The saved rectangle grown to the minimum size, if enough of its title bar is still on some display.
    fn fit(&self, displays: &[Rect], min: (f32, f32)) -> Option<Rect> {
        let values = [self.x, self.y, self.width, self.height];
        if !values.iter().all(|v| v.is_finite()) {
            return None;
        }
        let rect = Rect { x: self.x, y: self.y, width: self.width.max(min.0), height: self.height.max(min.1) };
        // A window needs somewhere to grab it: its top strip must show on a display, not just a corner of it.
        let strip = Rect { height: rect.height.min(TITLE_STRIP.1), ..rect };
        displays
            .iter()
            .any(|d| strip.overlap(d).is_some_and(|(w, h)| w >= TITLE_STRIP.0.min(rect.width) && h >= strip.height))
            .then_some(rect)
    }
}

/// How much of the window's top edge must be on a display for the user to be able to move it.
const TITLE_STRIP: (f32, f32) = (120., 32.);

#[derive(Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Rect {
    fn of(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: bounds.origin.x.into(),
            y: bounds.origin.y.into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
        }
    }

    /// The width and height of the part both rectangles cover, if any.
    fn overlap(&self, other: &Self) -> Option<(f32, f32)> {
        let w = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let h = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        (w > 0. && h > 0.).then_some((w, h))
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
        if let Some(code) = &self.language {
            crate::i18n::set_language(code);
        }
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

    /// Remembers a dock area's fixed-size pane, writing only when the size actually moved: a dock reports every
    /// layout edit, and a drag is many of them.
    pub fn remember_dock_size(cx: &mut App, dock: &str, pane: f32) {
        // A non-finite size would be written as `null`, which no build could read back.
        if pane.is_finite() && Self::global(cx).dock_sizes.get(dock) != Some(&pane) {
            Self::update(cx, |p| {
                p.dock_sizes.insert(dock.to_owned(), pane);
            });
        }
    }

    /// The saved size of a dock area's fixed-size pane, unless it is not a size a pane could have.
    pub fn dock_size(&self, dock: &str) -> Option<Pixels> {
        self.dock_sizes.get(dock).copied().filter(|s| s.is_finite() && *s >= MIN_PANE).map(px)
    }
}

/// The measured size of one pane of a dock area's center split, or `None` when the center is not a split
/// with `panes` panes (the screen's arrangement changed, so nothing about it is worth keeping).
pub fn pane_size(state: &DockAreaState, panes: usize, index: usize) -> Option<f32> {
    match &state.center.info {
        PanelInfo::Stack { sizes, .. } if sizes.len() == panes && state.center.children.len() == panes => {
            sizes.get(index).map(|s| f32::from(*s))
        }
        _ => None,
    }
}

/// Below this a saved pane would be a sliver the user cannot find to drag back.
const MIN_PANE: f32 = 120.;

#[cfg(test)]
mod tests {
    use gpui_kit::component::dock::PanelState;

    use super::*;

    const MIN: (f32, f32) = (1080., 640.);

    fn display(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, width: w, height: h }
    }

    fn saved(x: f32, y: f32, width: f32, height: f32) -> WindowState {
        WindowState { x, y, width, height, maximized: false }
    }

    #[test]
    fn an_older_file_loads_with_the_new_fields_defaulted() {
        let old =
            r#"{"theme":"Dark","language":"en","font_size":14.0,"sidebar_collapsed":true,"folder_tree_open":false}"#;
        let p: Preferences = serde_json::from_str(old).unwrap();
        assert_eq!(p.theme.as_deref(), Some("Dark"));
        assert!(p.sidebar_collapsed && !p.folder_tree_open);
        assert_eq!(p.window, None);
        assert!(p.dock_sizes.is_empty());
    }

    #[test]
    fn an_older_build_ignores_the_new_fields() {
        // Mirrors what an old build's struct does: no `deny_unknown_fields` anywhere in this type.
        let p = Preferences { window: Some(saved(1., 2., 3., 4.)), ..Default::default() };
        let text = serde_json::to_string(&p).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["from_a_future_build"] = true.into();
        assert!(serde_json::from_value::<Preferences>(value).is_ok());
    }

    #[test]
    fn the_new_fields_round_trip() {
        let mut p = Preferences {
            window: Some(WindowState { maximized: true, ..saved(10., 20., 1200., 800.) }),
            ..Default::default()
        };
        p.dock_sizes.insert("requests-dock".into(), 333.);
        let back: Preferences = serde_json::from_str(&serde_json::to_string_pretty(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn a_window_on_a_display_is_kept() {
        let d = [display(0., 0., 1920., 1080.)];
        let r = saved(100., 50., 1440., 900.).fit(&d, MIN).unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (100., 50., 1440., 900.));
    }

    #[test]
    fn a_window_on_a_disconnected_display_is_dropped() {
        let d = [display(0., 0., 1920., 1080.)];
        assert!(saved(2500., 100., 1440., 900.).fit(&d, MIN).is_none());
        assert!(saved(-3000., 100., 1440., 900.).fit(&d, MIN).is_none());
    }

    #[test]
    fn a_partly_visible_window_needs_its_title_strip_shown() {
        let d = [display(0., 0., 1920., 1080.)];
        // Hanging off the right edge but with plenty of its top showing.
        assert!(saved(1500., 100., 1440., 900.).fit(&d, MIN).is_some());
        // Only a sliver of its left edge is showing.
        assert!(saved(1900., 100., 1440., 900.).fit(&d, MIN).is_none());
        // Its title bar is above the screen, so it could not be dragged back.
        assert!(saved(100., -200., 1440., 900.).fit(&d, MIN).is_none());
    }

    #[test]
    fn a_window_on_the_second_display_is_kept() {
        let d = [display(0., 0., 1920., 1080.), display(1920., 0., 1280., 1024.)];
        assert!(saved(2000., 100., 1100., 700.).fit(&d, MIN).is_some());
    }

    #[test]
    fn a_window_smaller_than_the_minimum_is_grown_to_it() {
        let d = [display(0., 0., 1920., 1080.)];
        let r = saved(100., 100., 300., 200.).fit(&d, MIN).unwrap();
        assert_eq!((r.width, r.height), MIN);
        assert!(saved(f32::NAN, 0., 1440., 900.).fit(&d, MIN).is_none());
    }

    fn center(info: PanelInfo, children: usize) -> DockAreaState {
        let leaf = || PanelState { panel_name: "p".into(), children: vec![], info: PanelInfo::tabs(0) };
        DockAreaState {
            center: PanelState {
                panel_name: "StackPanel".into(),
                children: (0..children).map(|_| leaf()).collect(),
                info,
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_pane_size_is_read_only_from_the_expected_split() {
        let two = PanelInfo::Stack { sizes: vec![px(500.), px(320.)], axis: 1 };
        assert_eq!(pane_size(&center(two.clone(), 2), 2, 1), Some(320.));
        // A different number of panes than the code builds is not this layout.
        assert_eq!(pane_size(&center(two, 3), 2, 1), None);
        assert_eq!(pane_size(&center(PanelInfo::tabs(0), 2), 2, 1), None);
    }

    #[test]
    fn a_saved_pane_size_that_cannot_be_a_pane_is_ignored() {
        let mut p = Preferences::default();
        p.dock_sizes.insert("a".into(), 5.);
        p.dock_sizes.insert("b".into(), f32::NAN);
        p.dock_sizes.insert("c".into(), 300.);
        assert_eq!((p.dock_size("a"), p.dock_size("b"), p.dock_size("c")), (None, None, Some(px(300.))));
    }
}
