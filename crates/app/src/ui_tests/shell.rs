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

#[gpui_kit::test]
fn faults_agents_added_show_in_the_footer_and_clear_all_removes_them(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    assert!(!ui.has("faults-menu"), "nothing is shown while no rule is set");
    ui.fake.emit(
        "event.faults.changed",
        json!({"faults": [{"id": "f1", "target": "api.example.com/pay", "status": 503, "rate": 30}]}),
    );
    ui.settle();
    assert!(ui.has("faults-menu"));
    ui.fake.forget_calls();
    ui.click("faults-menu");
    // The rule's own "Clear" comes first, then the separator and "Clear all".
    ui.press("down");
    ui.press("down");
    ui.press("enter");
    assert_eq!(ui.fake.sent("fault.clear"), [json!({})]);
}

#[gpui_kit::test]
fn a_single_fault_is_cleared_by_its_id(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.emit(
        "event.faults.changed",
        json!({"faults": [{"id": "f1", "target": "a.test", "latencyMs": 500, "rate": 100}]}),
    );
    ui.settle();
    ui.fake.forget_calls();
    ui.click("faults-menu");
    ui.press("down");
    ui.press("enter");
    assert_eq!(ui.fake.sent("fault.clear"), [json!({"id": "f1"})]);
}

#[gpui_kit::test]
fn recording_and_replay_badges_stop_their_cassette(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.emit(
        "event.cassette.changed",
        json!({
            "record": {"recording": {"name": "checkout", "workspaceId": "w1", "hosts": ["a.test"], "interactions": 3}},
            "replay": {"replaying": {"name": "smoke", "workspaceId": "w1", "hosts": [], "served": 1, "unmatchedCount": 2}},
        }),
    );
    ui.settle();
    assert!(ui.has("recording-menu") && ui.has("replay-menu"));
    ui.fake.forget_calls();
    ui.click("recording-menu");
    ui.press("down");
    ui.press("enter");
    assert_eq!(ui.fake.sent("cassette.recordStop"), [json!({})]);
    ui.click("replay-menu");
    ui.press("down");
    ui.press("enter");
    assert_eq!(ui.fake.sent("cassette.replayStop"), [json!({})]);

    ui.fake.emit("event.cassette.changed", json!({"record": {"recording": null}, "replay": {"replaying": null}}));
    ui.settle();
    assert!(!ui.has("recording-menu") && !ui.has("replay-menu"), "both go when the engine says they ended");
}

#[gpui_kit::test]
fn an_approval_request_shows_a_badge_and_the_dialog_decides_it(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    let item = json!({
        "id": "a1", "by": "session:agent-a", "method": "POST", "url": "https://api.test/pay", "policy": "no-pay",
        "workspaceId": "w1", "createdAt": 1, "expiresAt": 300_000,
    });
    ui.fake.reply("approvals.list", json!({"pending": [item]}));
    ui.fake.emit("event.approval.requested", item);
    ui.settle();
    assert!(ui.has("approvals-open"), "the footer says something waits");
    assert!(!ui.has("approve-a1"), "nothing opens by itself");

    ui.click("approvals-open");
    assert!(ui.has("approve-a1") && ui.has("deny-a1"));
    ui.fake.forget_calls();
    ui.click("approve-a1");
    assert_eq!(ui.fake.sent("approvals.decide"), [json!({"id": "a1", "decision": "approve", "interactive": true})]);

    // Decided elsewhere (or expired): the list is read again and the row goes.
    ui.fake.reply("approvals.list", json!({"pending": []}));
    ui.fake.emit("event.approval.decided", json!({"id": "a1", "decision": "approved"}));
    ui.settle();
    assert!(!ui.has("approve-a1") && !ui.has("approvals-open"));
}

#[gpui_kit::test]
fn the_types_panel_infers_the_open_request_and_again_after_each_response(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply(
        "types.generate",
        json!({"name": "PingResponse", "lang": "ts", "samples": 1, "code": "export interface PingResponse {}"}),
    );
    ui.click("tree-row-r2");
    ui.press("ctrl-shift-b");
    ui.fake.forget_calls();
    ui.click(("inspector-panel", 2usize));
    assert_eq!(ui.fake.sent("types.generate"), [json!({"workspaceId": "w1", "request": "r2", "lang": "ts"})]);
    assert!(ui.has("types-code"));
    ui.fake.forget_calls();
    ui.click_within("types-langs", 2usize);
    assert_eq!(ui.fake.sent("types.generate")[0]["lang"], "python");
    ui.fake.forget_calls();
    ui.press("ctrl-enter");
    assert_eq!(ui.fake.sent("types.generate").len(), 1, "a new response is a new sample");
}

#[gpui_kit::test]
fn a_request_without_responses_says_to_send_it(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply_with("types.generate", |_| Err("NOT_FOUND no JSON response".into()));
    ui.click("tree-row-r2");
    ui.press("ctrl-shift-b");
    ui.click(("inspector-panel", 2usize));
    assert!(ui.has("empty-state") && !ui.has("types-code"));
}
