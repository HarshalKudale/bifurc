//! Mocks: rules that answer requests instead of (or before) the real server.

use std::time::Duration;

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn no_mocks_offer_to_create_or_import_one(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Mocks);
    assert!(ui.has("welcome-new-mock"));
    assert!(ui.has("welcome-import-curl"));
    assert!(ui.has("empty-state"), "the empty tree says so");
    assert!(!ui.has("mock-enabled"), "no editor without a mock");
}

#[gpui_kit::test]
fn mocks_are_listed_with_a_switch_each_unless_deleted_or_locked(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mocks);
    for row in ["tree-row-k1", "tree-row-k2", "tree-row-k3", "tree-row-k4"] {
        assert!(ui.has(row), "{row}");
    }
    assert!(ui.has("mock-enabled-k1") && ui.has("mock-enabled-k2"));
    assert!(!ui.has("mock-enabled-k3") && !ui.has("mock-enabled-k4"));
}

#[gpui_kit::test]
fn clicking_a_mock_loads_it_into_a_tab(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mocks);
    ui.fake.forget_calls();
    ui.click("tree-row-k1");
    assert!(ui.has("tab-mock-k1"));
    assert!(ui.has("mock-enabled") && ui.has("mock-pattern"), "the editor is open");
    assert_eq!(ui.fake.sent("entity.load"), [json!({"workspaceId": "w1", "kind": "mocks", "id": "k1"})]);
}

#[gpui_kit::test]
fn the_tree_switch_turns_a_mock_on_through_the_engine(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mocks);
    ui.fake.forget_calls();
    ui.click("mock-enabled-k2");
    assert_eq!(
        ui.fake.sent("entity.setEnabled"),
        [json!({"workspaceId": "w1", "kind": "mocks", "id": "k2", "enabled": true})]
    );
    assert!(ui.fake.sent("entity.load").is_empty(), "switching is not opening");
}

#[gpui_kit::test]
fn the_editor_switch_saves_the_mock_after_the_pause(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mocks);
    ui.click("tree-row-k1");
    ui.fake.forget_calls();
    ui.click("mock-enabled");
    assert!(ui.fake.sent("entity.update").is_empty(), "not before the pause");
    ui.advance(Duration::from_millis(700));
    let updates = ui.fake.sent("entity.update");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["kind"], "mocks");
    assert_eq!(updates[0]["entity"]["id"], "k1");
    assert_eq!(updates[0]["entity"]["enabled"], false);
    assert_eq!(updates[0]["entity"]["urlPattern"], "/users");
}

#[gpui_kit::test]
fn a_new_mock_is_created_by_ctrl_s_once_it_has_a_pattern(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mocks);
    ui.fake.reply_with("entity.create", |payload| {
        let mut entity = payload["entity"].clone();
        entity["id"] = json!("new-mock");
        Ok(json!({ "entity": entity }))
    });
    ui.press("ctrl-shift-m");
    assert!(ui.has("mock-pattern"), "Ctrl+Shift+M opens a draft");
    ui.fake.forget_calls();
    ui.press("ctrl-s");
    assert!(ui.fake.sent("entity.create").is_empty(), "a mock needs a pattern");
    ui.click("mock-pattern");
    ui.input("/health");
    assert!(ui.fake.sent("entity.create").is_empty(), "a draft waits for Save");
    ui.press("ctrl-s");
    let created = ui.fake.sent("entity.create");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["kind"], "mocks");
    assert_eq!(created[0]["workspaceId"], "w1");
    assert_eq!(created[0]["entity"]["urlPattern"], "/health");
}

#[gpui_kit::test]
fn a_locked_mock_explains_that_bifurc_must_be_updated(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mocks);
    ui.fake.forget_calls();
    ui.click("tree-row-k4");
    assert!(ui.has("needs-update-settings"));
    assert!(ui.fake.sent("entity.load").is_empty());
    assert!(!ui.has("tab-mock-k4"));
}

#[gpui_kit::test]
fn a_deleted_mock_is_brought_back_or_deleted_for_good(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mocks);
    ui.fake.forget_calls();
    ui.click("tree-row-k3");
    assert!(ui.has("deleted-revert") && ui.has("deleted-confirm"));
    assert!(ui.fake.sent("entity.load").is_empty());
    assert!(!ui.has("tab-mock-k3"));
    ui.click("deleted-revert");
    assert_eq!(ui.fake.sent("entity.revertDelete"), [json!({"workspaceId": "w1", "kind": "mocks", "id": "k3"})]);
}
