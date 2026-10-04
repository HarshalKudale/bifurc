//! The predefined themes, and applying one by name.
//!
//! Bifurc defines no colors of its own. The themes are the sets GPUI Kit publishes (see `themes/README.md`),
//! embedded at build time and registered with the kit's [`ThemeRegistry`]; choosing one swaps the kit's
//! global theme, which every component already reads.

use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, SharedString, px};

const BUNDLED: [&str; 21] = [
    include_str!("../themes/adventure.json"),
    include_str!("../themes/alduin.json"),
    include_str!("../themes/asciinema.json"),
    include_str!("../themes/aurora.json"),
    include_str!("../themes/ayu.json"),
    include_str!("../themes/catppuccin.json"),
    include_str!("../themes/everforest.json"),
    include_str!("../themes/fahrenheit.json"),
    include_str!("../themes/flexoki.json"),
    include_str!("../themes/gruvbox.json"),
    include_str!("../themes/harper.json"),
    include_str!("../themes/hybrid.json"),
    include_str!("../themes/jellybeans.json"),
    include_str!("../themes/kibble.json"),
    include_str!("../themes/macos-classic.json"),
    include_str!("../themes/mellifluous.json"),
    include_str!("../themes/molokai.json"),
    include_str!("../themes/solarized.json"),
    include_str!("../themes/spaceduck.json"),
    include_str!("../themes/tokyonight.json"),
    include_str!("../themes/twilight.json"),
];

/// The interface font sizes offered in settings, in pixels. The base font is the kit's zoom control: every
/// rem-based size follows it.
///
/// The kit's controls keep fixed inner paddings, so sizes below its default clip their text; the choices
/// start at the default and go up.
pub const FONT_SIZES: [f32; 4] = [16., 17., 18., 20.];
pub const DEFAULT_FONT_SIZE: f32 = 16.;

/// Registers the bundled themes. Call once, after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    for source in BUNDLED {
        // A bundled file that fails to parse is a build mistake, not a user-facing condition: it is skipped
        // and the remaining themes still load.
        let _ = ThemeRegistry::global_mut(cx).load_themes_from_str(source);
    }
}

/// Every available theme as `(name, is_dark)`: the kit's defaults first, then light before dark, by name.
pub fn available(cx: &App) -> Vec<(SharedString, bool)> {
    ThemeRegistry::global(cx).sorted_themes().into_iter().map(|t| (t.name.clone(), t.mode.is_dark())).collect()
}

/// The name of the theme in use.
pub fn current(cx: &App) -> SharedString {
    Theme::global(cx).theme_name().clone()
}

/// Switches to the named theme. Returns `false` if there is no theme of that name.
pub fn apply(name: &str, cx: &mut App) -> bool {
    let Some(config) = ThemeRegistry::global(cx).themes().get(name).cloned() else { return false };
    let mode = config.mode;
    {
        let theme = Theme::global_mut(cx);
        match mode {
            ThemeMode::Light => theme.light_theme = config,
            ThemeMode::Dark => theme.dark_theme = config,
        }
    }
    Theme::change(mode, None, cx);
    cx.refresh_windows();
    true
}

/// Sets the interface font size, which scales the whole rem-based layout.
pub fn set_font_size(size: f32, cx: &mut App) {
    // The base font anchors the rem scale, so it is the one value that is a pixel count by definition.
    Theme::update(cx, |theme| theme.font_size = px(size));
}
