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

/// The populated window with a second mock of `/users`, a scenario of the first, and the Mocks screen in front.
fn open_with_scenario(cx: &mut gpui_kit::TestAppContext) -> crate::test_support::Ui {
    let mut ui = crate::test_support::Ui::open(cx, |fake| {
        super::fixtures::populated(fake);
        let mut config = super::fixtures::config();
        config["mocks"].as_array_mut().unwrap().push(
            json!({"id": "k5", "name": "Users mock · error500", "method": "GET", "urlPattern": "/users", "enabled": false}),
        );
        fake.config(config);
        fake.reply_with("entity.list", |payload| {
            Ok(match payload["kind"].as_str() {
                Some("mocks") => json!({"entities": [
                    {"id": "k1", "name": "Users mock", "method": "GET", "urlPattern": "/users", "enabled": true, "createdAt": 1},
                    {"id": "k5", "name": "Users mock · error500", "method": "GET", "urlPattern": "/users", "enabled": false, "createdAt": 2},
                    {"id": "k2", "name": "Orders mock", "method": "POST", "urlPattern": "/orders", "enabled": false},
                ]}),
                _ => json!({"entities": []}),
            })
        });
        fake.reply("mocks.serveStatus", json!({"servers": []}));
    });
    ui.show(Screen::Mocks);
    ui
}

#[gpui_kit::test]
fn a_mock_with_scenarios_shows_a_switcher_that_enables_the_chosen_one(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_scenario(cx);
    ui.click("tree-row-k1");
    assert!(ui.has("mock-scenario-k1") && ui.has("mock-scenario-k5"));
    assert!(ui.has("mock-scenarios") && ui.has("mock-hits"), "the bar and its count");
    ui.fake.forget_calls();
    ui.click("mock-scenario-k5");
    assert_eq!(
        ui.fake.sent("entity.setEnabled"),
        [json!({"workspaceId": "w1", "kind": "mocks", "id": "k5", "enabled": true})]
    );
}

#[gpui_kit::test]
fn a_mock_alone_in_its_call_has_no_switcher_and_no_hits_bar(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mocks);
    ui.click("tree-row-k1");
    assert!(ui.has("mock-pattern"));
    assert!(!ui.has("mock-scenarios") && !ui.has("mock-hits"));
}

#[gpui_kit::test]
fn hits_are_read_from_the_mock_answers_in_capture_and_grow_with_live_traffic(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = crate::test_support::Ui::open(cx, |fake| {
        super::fixtures::populated(fake);
        fake.reply("mocks.serveStatus", json!({"servers": []}));
    });
    ui.show(Screen::Mocks);
    assert!(ui.fake.sent("capture.list").contains(&json!({"workspaceId": "w1", "summary": true, "via": "mock"})));
    ui.click("tree-row-k1");
    assert!(!ui.has("mock-hits"), "nothing answered yet");
    let live = json!({"entries": [{"id": "x1", "timestamp": 9, "method": "GET", "url": "http://a/users", "status": 200, "via": "mock", "mockId": "k1", "workspaceId": "w1"}]});
    ui.fake.emit("event.log.entry", live.clone());
    ui.fake.emit("event.log.entry", live);
    ui.settle();
    assert!(ui.has("mock-hits"));
}

#[gpui_kit::test]
fn serving_on_a_port_is_detached_and_blank_fields_are_left_out(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_scenario(cx);
    ui.fake.forget_calls();
    ui.click("mock-serve");
    ui.answer_prompt(&["4010", "http://localhost:3000"]);
    ui.click("mock-serve");
    ui.answer_prompt(&["", ""]);
    assert_eq!(
        ui.fake.sent("mocks.serve"),
        [
            json!({"workspaceId": "w1", "detach": true, "port": 4010, "fallthrough": "http://localhost:3000"}),
            json!({"workspaceId": "w1", "detach": true}),
        ]
    );
    assert!(!ui.fake.sent("mocks.serveStatus").is_empty(), "no event says it started, so the list is read again");
    ui.fake.forget_calls();
    ui.click("mock-serve");
    ui.answer_prompt(&["http", ""]);
    assert!(ui.fake.sent("mocks.serve").is_empty(), "a port is a number");
}

#[gpui_kit::test]
fn running_servers_of_this_workspace_can_be_stopped(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_scenario(cx);
    ui.fake.reply(
        "mocks.serveStatus",
        json!({"servers": [
            {"port": 4010, "workspaceId": "w1", "fallthrough": null, "detached": true, "hits": 1},
            {"port": 4011, "workspaceId": "w2", "fallthrough": null, "detached": true, "hits": 0},
        ]}),
    );
    ui.show(Screen::Webhooks);
    ui.show(Screen::Mocks);
    assert!(ui.has("mock-server-stop-4010") && ui.has("mock-server-copy-4010"));
    assert!(!ui.has("mock-server-stop-4011"), "another workspace's server");
    ui.fake.forget_calls();
    ui.click("mock-server-stop-4010");
    assert_eq!(ui.fake.sent("mocks.serveStop"), [json!({"port": 4010})]);
    // A request a server answered asks for its hit count again.
    ui.fake.forget_calls();
    ui.fake.emit("event.log.entry", json!({"entries": [{"id": "s1", "timestamp": 9, "method": "GET", "url": "http://127.0.0.1:4010/x", "workspaceId": "w1", "origin": "server"}]}));
    ui.settle();
    assert!(!ui.fake.sent("mocks.serveStatus").is_empty());
}

#[gpui_kit::test]
fn the_open_mock_is_checked_against_its_paired_request_and_drift_is_listed(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply(
        "contract.check",
        json!({"checks": [{"mock": "Users mock", "mockId": "k1", "method": "GET", "path": "/users", "request": "List users",
            "source": "real", "ok": false, "truncated": 3,
            "differences": [{"kind": "+", "path": ".extra", "text": "+ .extra (string)"}]}],
            "unpaired": [], "drifted": 1, "ok": 0, "failed": 0, "unpairedCount": 0}),
    );
    ui.show(Screen::Mocks);
    ui.click("tree-row-k1");
    ui.fake.forget_calls();
    ui.click("contract-check");
    let sent = ui.fake.sent("contract.check");
    assert_eq!(sent.len(), 1);
    assert_eq!((&sent[0]["workspaceId"], &sent[0]["mock"]), (&json!("w1"), &json!("k1")));
    assert!(sent[0].get("all").is_none());
    assert!(ui.has("contract-drift"));
    assert!(!ui.has("contract-report"), "one mock's answer stays in its editor");
}

#[gpui_kit::test]
fn checking_all_asks_for_options_then_lists_each_mock_and_opens_it_on_click(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply(
        "contract.check",
        json!({"checks": [{"mock": "Users mock", "mockId": "k1", "method": "GET", "path": "/users", "request": "List users",
            "source": "capture", "ok": true, "truncated": 0, "differences": []}],
            "unpaired": [{"mock": "Orders mock", "mockId": "k2", "method": "POST", "reason": "no saved request matches"}],
            "drifted": 0, "ok": 1, "failed": 0, "unpairedCount": 1}),
    );
    ui.show(Screen::Mocks);
    ui.fake.forget_calls();
    ui.click("mock-check-all");
    ui.click("contract-from-capture");
    ui.click("contract-run");
    let sent = ui.fake.sent("contract.check");
    assert_eq!(sent.len(), 1);
    assert_eq!((&sent[0]["all"], &sent[0]["fromCapture"]), (&json!(true), &json!(true)));
    assert!(ui.has("contract-summary") && ui.has("contract-row-k1") && ui.has("contract-unpaired-k2"));
    ui.click("contract-row-k1");
    assert!(ui.has("tab-mock-k1"));
    ui.click("contract-close");
    assert!(!ui.has("contract-report"));
}

#[gpui_kit::test]
fn a_failed_check_says_why(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply_with("contract.check", |_| Err("NOT_FOUND no such mock".into()));
    ui.show(Screen::Mocks);
    ui.click("tree-row-k1");
    ui.click("contract-check");
    assert!(ui.has("contract-error"));
}
