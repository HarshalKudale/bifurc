//! Flows: graphs of requests and logic that the engine runs.

use std::time::Duration;

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn no_flows_show_the_empty_tree_and_no_canvas(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Flows);
    assert!(ui.has("empty-state"));
    assert!(ui.has("folder-tree-new"));
    assert!(!ui.has("flow-run"));
}

#[gpui_kit::test]
fn the_first_flow_is_listed_and_opened_with_its_status_read(cx: &mut gpui_kit::TestAppContext) {
    // The screen opens the first flow as the window starts, whichever screen is in front.
    let mut ui = open_populated(cx);
    ui.show(Screen::Flows);
    assert!(ui.has("tree-row-fl1"));
    assert!(ui.has("flow-run") && ui.has("flow-add-node"), "the canvas is open on it");
    assert_eq!(ui.fake.sent("entity.load"), [json!({"workspaceId": "w1", "kind": "flows", "id": "fl1"})]);
    assert_eq!(ui.fake.sent("flow.status"), [json!({"workspaceId": "w1", "id": "fl1"})]);
}

#[gpui_kit::test]
fn run_asks_the_engine_to_run_the_open_flow(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Flows);
    ui.fake.reply("flow.run", json!({"runId": "run-1"}));
    ui.fake.forget_calls();
    ui.click("flow-run");
    assert_eq!(ui.fake.sent("flow.run"), [json!({"workspaceId": "w1", "id": "fl1"})]);
    // Its run is under way: the button turns into Stop.
    assert!(ui.has("flow-stop"));
    ui.fake.forget_calls();
    ui.click("flow-stop");
    assert_eq!(ui.fake.sent("flow.cancel"), [json!({"workspaceId": "w1", "id": "fl1"})]);
}

#[gpui_kit::test]
fn adding_a_node_is_saved_after_the_pause(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Flows);
    ui.fake.forget_calls();
    ui.click("flow-add-node");
    ui.press("down");
    ui.press("enter");
    assert!(ui.fake.sent("entity.update").is_empty(), "not before the pause");
    ui.advance(Duration::from_millis(1500));
    let updates = ui.fake.sent("entity.update");
    assert_eq!(updates.len(), 1, "{:?}", ui.fake.calls().iter().map(|c| c.action.clone()).collect::<Vec<_>>());
    assert_eq!(updates[0]["kind"], "flows");
    assert_eq!(updates[0]["entity"]["id"], "fl1");
    assert_eq!(updates[0]["entity"]["nodes"].as_array().map(Vec::len), Some(1));
}

#[gpui_kit::test]
fn running_a_flow_with_unsaved_changes_saves_it_first(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Flows);
    ui.click("flow-add-node");
    ui.press("down");
    ui.press("enter");
    ui.fake.forget_calls();
    ui.click("flow-run");
    ui.advance(Duration::from_millis(100));
    let calls: Vec<_> = ui.fake.calls().into_iter().map(|c| c.action).collect();
    let saved = calls.iter().position(|a| a == "entity.update").expect("the edit is written");
    let ran = calls.iter().position(|a| a == "flow.run").expect("then the flow runs");
    assert!(saved < ran, "{calls:?}");
}

#[gpui_kit::test]
fn a_new_flow_is_created_by_name(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Flows);
    ui.press("ctrl-shift-g");
    ui.fake.forget_calls();
    ui.answer_prompt(&["Signup"]);
    let created = ui.fake.sent("entity.create");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["kind"], "flows");
    assert_eq!(created[0]["entity"]["name"], "Signup");
}
