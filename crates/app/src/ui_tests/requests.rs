//! The Requests screen: the tree, tabs, and the REST editor's open / edit / save / send flows.

use std::time::Duration;

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn an_empty_workspace_offers_the_kinds_of_request_to_start(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Requests);
    for kind in 0..8usize {
        assert!(ui.has(("welcome-new", kind)), "start-a-request button {kind}");
    }
    assert!(ui.has("welcome-import-curl"));
    assert!(ui.has("empty-state"), "the empty tree says so");
    assert!(!ui.has("send-request"), "no editor without a request");
}

#[gpui_kit::test]
fn the_tree_lists_requests_with_deleted_and_locked_ones_too(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Requests);
    for row in ["tree-row-f1", "tree-row-r2", "tree-row-r3", "tree-row-r4"] {
        assert!(ui.has(row), "{row}");
    }
    assert!(!ui.has("tree-row-r1"), "a request in a closed folder is not shown until it opens");
    assert!(!ui.has("empty-state"));
}

#[gpui_kit::test]
fn clicking_a_request_loads_it_and_opens_a_tab(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Requests);
    ui.fake.forget_calls();
    ui.click("tree-row-r2");
    assert!(ui.has("tab-requests-r2"));
    assert!(ui.has("send-request"));
    assert_eq!(ui.fake.sent("entity.load"), [json!({"workspaceId": "w1", "kind": "requests", "id": "r2"})]);
    // Opening the same request again selects its tab instead of opening another.
    ui.click("tree-row-r2");
    assert_eq!(ui.fake.sent("entity.load").len(), 1);
}

#[gpui_kit::test]
fn deleted_and_locked_requests_do_not_open(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Requests);
    ui.fake.forget_calls();
    ui.click("tree-row-r3");
    ui.click("tree-row-r4");
    assert!(ui.fake.sent("entity.load").is_empty());
    assert!(!ui.has("tab-requests-r3") && !ui.has("tab-requests-r4"));
    assert!(!ui.has("send-request"));
}

#[gpui_kit::test]
fn an_edit_of_a_saved_request_is_written_after_the_pause_and_not_before(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Requests);
    ui.click("tree-row-r2");
    ui.fake.forget_calls();
    ui.press("ctrl-l");
    ui.input("/more");
    ui.advance(Duration::from_millis(500));
    assert!(ui.fake.sent("entity.update").is_empty(), "the pause has not passed");
    ui.input("/again");
    ui.advance(Duration::from_millis(500));
    assert!(ui.fake.sent("entity.update").is_empty(), "typing starts the wait over");
    ui.advance(Duration::from_millis(200));
    let updates = ui.fake.sent("entity.update");
    assert_eq!(updates.len(), 1, "a burst of typing is one write");
    assert_eq!(updates[0]["kind"], "requests");
    assert_eq!(updates[0]["entity"]["id"], "r2");
    assert!(updates[0]["entity"]["url"].as_str().unwrap().ends_with("/more/again"), "{}", updates[0]);
}

#[gpui_kit::test]
fn ctrl_s_creates_a_draft(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Requests);
    ui.fake.reply_with("entity.create", |payload| {
        let mut entity = payload["entity"].clone();
        entity["id"] = json!("new1");
        Ok(json!({ "entity": entity }))
    });
    ui.press("ctrl-n");
    assert!(ui.has("send-request"), "a draft opens an editor");
    ui.fake.forget_calls();
    ui.press("ctrl-s");
    assert!(ui.fake.sent("entity.create").is_empty(), "a draft without an address is not created");
    ui.press("ctrl-l");
    ui.input("https://api.test/new");
    assert!(ui.fake.sent("entity.create").is_empty(), "a draft waits for Save");
    ui.press("ctrl-s");
    let created = ui.fake.sent("entity.create");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["kind"], "requests");
    assert_eq!(created[0]["workspaceId"], "w1");
    assert_eq!(created[0]["entity"]["url"], "https://api.test/new");
    assert_eq!(created[0]["entity"]["method"], "GET");
}

#[gpui_kit::test]
fn ctrl_enter_sends_a_saved_request_by_id(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Requests);
    ui.click("tree-row-r2");
    ui.fake.forget_calls();
    ui.press("ctrl-enter");
    let sent = ui.fake.sent("request.send");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["id"], "r2");
    assert_eq!(sent[0]["workspaceId"], "w1");
    assert!(sent[0].get("request").is_none(), "an unedited saved request is sent by id");
}

#[gpui_kit::test]
fn the_send_button_sends_and_escape_cancels_it(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Requests);
    ui.click("tree-row-r2");
    ui.fake.hold("request.send");
    ui.fake.forget_calls();
    ui.click("send-request");
    let sent = ui.fake.sent("request.send");
    assert_eq!(sent.len(), 1);
    let stream = sent[0]["streamId"].clone();
    assert!(ui.fake.sent("request.cancel").is_empty());
    // Focus inside the editor, then Esc.
    ui.press("ctrl-l");
    ui.press("escape");
    assert_eq!(ui.fake.sent("request.cancel"), [json!({ "streamId": stream })]);
    ui.fake.release("request.send", json!({"cancelled": true}));
    ui.settle();
    ui.fake.forget_calls();
    ui.press("escape");
    assert!(ui.fake.sent("request.cancel").is_empty(), "nothing to cancel once the send is over");
}

#[gpui_kit::test]
fn an_edited_request_is_sent_as_the_draft_the_editor_holds(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Requests);
    ui.click("tree-row-r2");
    ui.press("ctrl-l");
    ui.input("?x=1");
    ui.fake.forget_calls();
    ui.press("ctrl-enter");
    let sent = ui.fake.sent("request.send");
    assert_eq!(sent.len(), 1);
    assert!(sent[0]["request"]["url"].as_str().unwrap().ends_with("?x=1"), "{}", sent[0]);
    assert_eq!(sent[0]["entityId"], "r2");
}

#[gpui_kit::test]
fn a_request_that_cannot_be_loaded_says_so(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply_with("entity.load", |_| Err("gone".into()));
    ui.show(Screen::Requests);
    ui.click("tree-row-r2");
    assert!(ui.has("tab-requests-r2"), "the tab opens and reports the failure");
    assert!(!ui.has("send-request"), "there is no editor for a request that did not load");
}
