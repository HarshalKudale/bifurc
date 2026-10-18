//! The Faults tab of Proxy rules and the policy page of the workspace configuration (0-4).

use serde_json::json;

use super::{Ui, open_populated};
use crate::shell::Screen;

fn open_faults(cx: &mut gpui_kit::TestAppContext) -> Ui {
    let mut ui = open_populated(cx);
    ui.fake.emit(
        "event.faults.changed",
        json!({"faults": [{"id": "f1", "target": "api.example.com/pay", "status": 503, "rate": 30}]}),
    );
    ui.show(Screen::ProxyRules);
    ui.click_within("proxy-tabs", 1usize);
    ui
}

#[gpui_kit::test]
fn the_faults_tab_lists_active_faults_and_clears_one_or_all(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_faults(cx);
    assert!(ui.has("fault-clear-f1"));
    assert!(!ui.has("rule-pattern"), "the rules editor is not shown");
    ui.fake.forget_calls();
    ui.click("fault-clear-f1");
    assert_eq!(ui.fake.sent("fault.clear"), [json!({"id": "f1"})]);
    ui.click("faults-clear-all");
    assert_eq!(ui.fake.sent("fault.clear")[1], json!({}));
}

#[gpui_kit::test]
fn adding_a_fault_sends_what_the_engine_accepts_and_nothing_for_a_bad_input(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_faults(cx);
    ui.fake.forget_calls();
    ui.click("add-fault");
    ui.answer_prompt(&["api.example.com/pay", "", "503", "30", ""]);
    assert_eq!(ui.fake.sent("fault.add"), [json!({"target": "api.example.com/pay", "status": 503, "rate": 30})]);
    ui.click("add-fault");
    ui.answer_prompt(&["a.test", "", "", "", "reset"]);
    assert_eq!(ui.fake.sent("fault.add")[1], json!({"target": "a.test", "reset": true}));
    ui.click("add-fault");
    ui.answer_prompt(&["a.test", "100", "", "", "drop"]);
    assert_eq!(ui.fake.sent("fault.add").len(), 2, "drop with latency is refused before it is sent");
}

#[gpui_kit::test]
fn the_policy_page_shows_the_policy_and_checks_a_command(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply(
        "policy.show",
        json!({"file": "policy.yaml", "applyTo": ["cli", "mcp"], "policies": [
            {"name": "no-prod-writes", "kind": "require_human", "env": ["prod"], "methods": ["POST"], "hosts": [], "paths": []},
        ]}),
    );
    ui.fake.reply("policy.check", json!({"decision": "require_human", "policy": "no-prod-writes", "reason": "held"}));
    ui.show(Screen::WorkspaceConfig);
    assert_eq!(ui.fake.sent("policy.show"), [json!({"workspaceId": "w1"})]);
    ui.click("0-4");
    assert!(ui.has("policy-reveal"), "reveal");
    ui.fake.forget_calls();
    ui.click("policy-check");
    assert!(ui.fake.sent("policy.check").is_empty(), "no URL, nothing to check");
}
