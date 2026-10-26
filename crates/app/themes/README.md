# Bundled themes

These theme sets are the ones published with GPUI Kit (`longbridge/gpui-kit`, tag `v0.7.0`, directory `themes/`),
copied unchanged. They are licensed under the Apache License 2.0 by their authors; each file names its author and
source. The app embeds them at build time and registers them with the kit's `ThemeRegistry`, so the Appearance
settings offer the same predefined themes as the GPUI Kit site.

`default-dark.json` is the one exception: the kit's "Default Dark" with dark text on the status badges and a
different surface color per region (`sidebar`, `group_box`, `accordion`, `status_bar`). The registry keeps the
first theme of a name, so `theme.rs` installs this file in place of the kit's instead of registering it.
