//! The audit log: the workspace's history, commit by commit.

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn no_history_shows_the_empty_state(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::AuditLog);
    assert!(ui.has("empty-state"));
    assert!(ui.has("audit-refresh") && ui.has("audit-export-csv") && ui.has("audit-export-json"));
}

#[gpui_kit::test]
fn commits_are_listed_newest_page_first(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::AuditLog);
    assert!(ui.has("audit-row-aaa111") && ui.has("audit-row-bbb222"));
    assert!(!ui.has("empty-state"));
    assert!(
        ui.fake.sent("audit.list").contains(&json!({"workspaceId": "w1", "limit": 100, "offset": 0})),
        "the first page is asked for"
    );
}

#[gpui_kit::test]
fn expanding_a_commit_loads_what_it_changed(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("history.patch", json!({"files": []}));
    ui.show(Screen::AuditLog);
    ui.fake.forget_calls();
    ui.click("audit-row-aaa111");
    let patches = ui.fake.sent("history.patch");
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0]["workspaceId"], "w1");
    assert_eq!(patches[0]["commitHash"], "aaa111");
    // Collapsing and expanding again does not ask twice.
    ui.click("audit-row-aaa111");
    ui.click("audit-row-aaa111");
    assert_eq!(ui.fake.sent("history.patch").len(), 1);
}

#[gpui_kit::test]
fn reload_asks_for_the_first_page_again(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::AuditLog);
    ui.fake.forget_calls();
    ui.click("audit-refresh");
    assert_eq!(ui.fake.sent("audit.list"), [json!({"workspaceId": "w1", "limit": 100, "offset": 0})]);
}

#[gpui_kit::test]
fn the_next_page_button_asks_for_the_next_offset(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply(
        "audit.list",
        json!({"total": 250, "entries": [
            {"commitHash": "aaa111", "ts": 1, "action": "create", "entity": "requests", "entityName": "Ping"},
        ]}),
    );
    ui.show(Screen::AuditLog);
    ui.click("audit-refresh");
    ui.fake.forget_calls();
    ui.click("audit-next");
    assert_eq!(ui.fake.sent("audit.list"), [json!({"workspaceId": "w1", "limit": 100, "offset": 100})]);
}

#[gpui_kit::test]
fn the_filter_menu_narrows_the_listing(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::AuditLog);
    ui.fake.forget_calls();
    ui.click("audit-action");
    // Menu rows: "All actions", then created / updated / deleted.
    ui.press("down");
    ui.press("down");
    ui.press("enter");
    let lists = ui.fake.sent("audit.list");
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0]["action"], "create");
    assert_eq!(lists[0]["offset"], 0);
}
