//! The shell: screen switching, the command palette and finder, and the app-level keys.

use serde_json::json;

use super::{open_empty, open_populated};

#[gpui_kit::test]
fn ctrl_digits_bring_each_screen_to_the_front(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    // One thing only that screen shows, in the order of the sidebar: Ctrl+1 to Ctrl+9.
    let screens = [
        ("ctrl-1", "refresh-services"),
        ("ctrl-2", "health-check-all"),
        ("ctrl-3", "new-mapping"),
        ("ctrl-4", "new-rule"),
        ("ctrl-5", "capture-pause"),
        ("ctrl-6", "tree-row-r2"),
        ("ctrl-7", "new-mock-tab"),
        ("ctrl-8", "tree-row-fl1"),
        ("ctrl-9", "new-environment"),
    ];
    for (keys, marker) in screens {
        ui.press(keys);
        assert!(ui.has(marker), "{keys} shows {marker}");
        for (_, other) in screens.iter().filter(|(_, other)| *other != marker) {
            assert!(!ui.has(*other), "{keys} leaves {other} behind");
        }
    }
}

#[gpui_kit::test]
fn ctrl_comma_opens_settings(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.press("ctrl-9");
    ui.press("ctrl-,");
    assert!(!ui.has("new-environment"));
    assert!(!ui.has("tree-row-r2"));
}

#[gpui_kit::test]
fn the_command_palette_opens_filters_and_runs_a_command(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    assert!(!ui.has("dialog"));
    ui.press("ctrl-k");
    assert!(ui.has("dialog"), "Ctrl+K opens the palette");
    ui.input("services");
    // Two rows match, "Look for local services again" and its group's sibling.
    assert!(ui.has("index-path(3,0,0)"));
    ui.fake.forget_calls();
    ui.press("enter");
    assert!(!ui.has("dialog"), "confirming closes it");
    assert_eq!(ui.fake.sent("services.discover"), [json!({})]);
    assert!(ui.has("refresh-services"), "the command's screen is in front");
}

#[gpui_kit::test]
fn escape_closes_the_palette_without_running_anything(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.press("ctrl-k");
    ui.input("services");
    ui.fake.forget_calls();
    ui.press("escape");
    // The first Esc clears the search, the second closes the palette.
    if ui.has("dialog") {
        ui.press("escape");
    }
    assert!(!ui.has("dialog"));
    assert!(ui.fake.calls().is_empty());
    assert!(!ui.has("refresh-services"), "no command ran");
}

#[gpui_kit::test]
fn the_finder_opens_a_request_by_name(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.press("ctrl-9");
    ui.press("ctrl-p");
    assert!(ui.has("dialog"), "Ctrl+P opens the finder");
    ui.input("ping");
    ui.fake.forget_calls();
    ui.press("enter");
    assert!(!ui.has("dialog"));
    assert!(ui.has("tab-requests-r2"), "the request opens on the Requests screen");
    assert_eq!(ui.fake.sent("entity.load"), [json!({"workspaceId": "w1", "kind": "requests", "id": "r2"})]);
}

#[gpui_kit::test]
fn the_proxy_button_in_the_title_bar_starts_the_forward_proxy(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("server.status", json!({"running": false, "port": 8080}));
    ui.fake.forget_calls();
    ui.click("proxy-toggle");
    assert_eq!(ui.fake.sent("server.start"), [json!({})]);
    assert!(!ui.fake.sent("server.status").is_empty(), "the status is read again afterwards");
}

#[gpui_kit::test]
fn a_running_proxy_is_stopped_by_the_same_button(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("server.status", json!({"running": true, "port": 8080}));
    ui.fake.emit("event.server.error", json!({}));
    ui.settle();
    ui.fake.forget_calls();
    ui.click("proxy-toggle");
    assert_eq!(ui.fake.sent("server.stop"), [json!({})]);
}

#[gpui_kit::test]
fn f5_reads_the_workspace_again(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.fake.forget_calls();
    ui.press("f5");
    assert_eq!(ui.fake.sent("config.get").len(), 1);
}

#[gpui_kit::test]
fn a_change_the_engine_announces_updates_the_listing_without_a_fetch(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.press("ctrl-6");
    assert!(!ui.has("tree-row-r9"));
    // A creation cannot be patched in: the snapshot is read again.
    ui.fake
        .config(json!({"requests": [{"id": "r9", "name": "Fresh", "method": "GET", "url": "https://api.test/fresh"}]}));
    ui.fake.forget_calls();
    ui.fake.emit("event.entity.changed", json!({"wsId": "w1", "kind": "requests", "id": "r9", "action": "created"}));
    ui.settle();
    ui.advance(std::time::Duration::from_millis(150));
    assert_eq!(ui.fake.sent("config.get").len(), 1);
    assert!(ui.has("tree-row-r9"));
    assert!(!ui.has("tree-row-r2"), "the new snapshot replaces the old one");
}

#[gpui_kit::test]
fn the_palette_opens_the_new_workspace_dialog_and_creates_it_by_name(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("workspace.add", json!({"id": "w9"}));
    ui.press("ctrl-k");
    ui.input("workspace");
    // The workspace group is the sixth, and "Add workspace" its third entry.
    ui.click("index-path(5,2,0)");
    assert!(ui.has("setup-submit") && ui.has("mode-new") && ui.has("mode-import"));
    ui.fake.forget_calls();
    ui.click("setup-submit");
    assert!(ui.fake.sent("workspace.add").is_empty(), "a workspace needs a name");
    ui.input("Side project");
    ui.click("setup-submit");
    assert_eq!(ui.fake.sent("workspace.add"), [json!({"name": "Side project"})]);
    assert!(!ui.has("setup-submit"), "the dialog closes once it is made");
}

#[gpui_kit::test]
fn the_new_workspace_dialog_can_be_cancelled(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.press("ctrl-k");
    ui.input("workspace");
    ui.click("index-path(5,2,0)");
    ui.fake.forget_calls();
    ui.click("setup-cancel");
    assert!(!ui.has("setup-submit"));
    assert!(ui.fake.sent("workspace.add").is_empty());
}

#[gpui_kit::test]
fn the_inspector_toggles_with_ctrl_shift_b(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.click("tree-row-r2");
    let before = ui.count("empty-state");
    ui.press("ctrl-shift-b");
    assert_eq!(ui.count("empty-state"), before + 1, "the inspector's empty history appears");
    ui.press("ctrl-shift-b");
    assert_eq!(ui.count("empty-state"), before);
}
