//! Webhooks: the engine's receivers, what they got, and sample events.

use serde_json::{Value, json};

use super::{open_empty, open_populated};
use crate::{shell::Screen, test_support::Ui};

fn listeners(rows: Value) -> Value {
    json!({"listeners": rows})
}

fn stripe() -> Value {
    json!({"id": "l1", "port": 4020, "name": "stripe", "forward": "http://api.localhost/hooks", "workspaceId": "w1", "detached": true, "hits": 2})
}

fn received(id: &str, origin: &str) -> Value {
    json!({"id": id, "timestamp": 5, "method": "POST", "url": "http://127.0.0.1:4020/hook", "status": 200, "origin": origin})
}

/// The populated window with a listener of this workspace (and one of another), two received requests among
/// other traffic, and the two sample events; the Webhooks screen is in front.
fn open_webhooks(cx: &mut gpui_kit::TestAppContext) -> Ui {
    let mut ui = open_populated(cx);
    let other =
        json!({"id": "l2", "port": 4030, "name": "elsewhere", "workspaceId": "w2", "detached": true, "hits": 0});
    ui.fake.reply("webhook.list", listeners(json!([stripe(), other])));
    ui.fake.reply(
        "webhook.fixtures",
        json!({"fixtures": [
            {"name": "stripe.checkout.session.completed", "description": "A paid Checkout session"},
            {"name": "github.push", "description": "A push with one commit"},
        ]}),
    );
    // The engine filters by origin; the fake answers the way it would.
    ui.fake.reply_with("capture.list", |payload| {
        Ok(match payload["origin"].as_str() {
            Some("webhook") => {
                json!({"paused": false, "entries": [received("h2", "webhook"), received("h1", "webhook")]})
            }
            _ => json!({"paused": false, "entries": [received("c1", "forward")]}),
        })
    });
    ui.fake.reply("capture.get", json!({"entry": received("h1", "webhook")}));
    ui.show(Screen::Webhooks);
    ui
}

#[gpui_kit::test]
fn without_listeners_or_requests_each_section_says_so(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Webhooks);
    assert!(ui.has("webhook-listen"));
    assert!(ui.has("empty-state"), "nothing received");
    assert!(!ui.has("webhook-stop-l1"));
}

#[gpui_kit::test]
fn this_workspaces_listeners_are_listed_and_follow_webhook_changed(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_webhooks(cx);
    assert!(ui.has("webhook-stop-l1") && ui.has("webhook-copy-l1"));
    assert!(!ui.has("webhook-stop-l2"), "another workspace's listener");
    let added = json!({"id": "l3", "port": 4021, "name": "github", "workspaceId": "w1", "detached": true, "hits": 0});
    ui.fake.emit("event.webhook.changed", listeners(json!([stripe(), added])));
    ui.settle();
    assert!(ui.has("webhook-stop-l1") && ui.has("webhook-stop-l3"));
    ui.fake.emit("event.webhook.changed", listeners(json!([added])));
    ui.settle();
    assert!(!ui.has("webhook-stop-l1") && ui.has("webhook-stop-l3"));
}

#[gpui_kit::test]
fn stopping_a_listener_tells_the_engine(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_webhooks(cx);
    ui.fake.forget_calls();
    ui.click("webhook-stop-l1");
    assert_eq!(ui.fake.sent("webhook.stop"), [json!({"id": "l1"})]);
}

#[gpui_kit::test]
fn a_started_listener_is_detached_and_blank_fields_are_left_out(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_webhooks(cx);
    ui.fake.forget_calls();
    ui.click("webhook-listen");
    ui.answer_prompt(&["hooks", "4025", "http://api.localhost/x"]);
    assert_eq!(
        ui.fake.sent("webhook.listen"),
        [
            json!({"workspaceId": "w1", "detach": true, "name": "hooks", "port": 4025, "forward": "http://api.localhost/x"})
        ]
    );
    ui.click("webhook-listen");
    ui.answer_prompt(&["", "", ""]);
    assert_eq!(ui.fake.sent("webhook.listen")[1], json!({"workspaceId": "w1", "detach": true}));
}

#[gpui_kit::test]
fn a_port_that_is_not_a_number_sends_nothing(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_webhooks(cx);
    ui.fake.forget_calls();
    ui.click("webhook-listen");
    ui.answer_prompt(&["", "http", ""]);
    assert!(ui.fake.sent("webhook.listen").is_empty());
}

#[gpui_kit::test]
fn received_requests_are_asked_for_by_origin_and_live_ones_of_other_origins_are_ignored(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut ui = open_webhooks(cx);
    assert!(ui.fake.sent("capture.list").contains(&json!({"workspaceId": "w1", "summary": true, "origin": "webhook"})));
    assert!(ui.has("capture-h1") && ui.has("capture-h2"));
    assert!(!ui.has("capture-c1"), "traffic is not a webhook");
    let live = |id: &str, origin: &str| json!({"entries": [{"id": id, "timestamp": 9, "method": "POST", "url": "http://127.0.0.1:4020/x", "workspaceId": "w1", "origin": origin}]});
    ui.fake.emit("event.log.entry", live("h3", "webhook"));
    ui.fake.emit("event.log.entry", live("c9", "forward"));
    ui.settle();
    assert!(ui.has("capture-h3") && !ui.has("capture-c9"));
}

#[gpui_kit::test]
fn a_received_request_can_be_replayed_to_its_default_target_or_to_a_url(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_webhooks(cx);
    ui.fake.reply("webhook.replay", json!({"status": 204}));
    ui.click("capture-h1");
    assert!(ui.has("webhook-replay") && ui.has("webhook-replay-to") && ui.has("capture-save-request"));
    ui.fake.forget_calls();
    ui.click("webhook-replay");
    assert_eq!(ui.fake.sent("webhook.replay"), [json!({"workspaceId": "w1", "id": "h1"})]);
    ui.click("webhook-replay-to");
    ui.answer_prompt(&["http://127.0.0.1:4021"]);
    assert_eq!(
        ui.fake.sent("webhook.replay")[1],
        json!({"workspaceId": "w1", "id": "h1", "to": "http://127.0.0.1:4021"})
    );
}

#[gpui_kit::test]
fn capture_has_no_replay_buttons(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("capture.get", json!({"entry": {"id": "c1", "method": "GET", "url": "https://api.test/users"}}));
    ui.show(Screen::Capture);
    ui.click("capture-c1");
    assert!(ui.has("capture-save-request") && !ui.has("webhook-replay"));
}

#[gpui_kit::test]
fn a_sample_event_goes_to_a_listener_by_name_or_to_a_url(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_webhooks(cx);
    ui.fake.reply("webhook.trigger", json!({"status": 200}));
    ui.fake.forget_calls();
    ui.click("webhook-send-github.push");
    ui.answer_prompt(&["", ""]);
    ui.click("webhook-send-github.push");
    ui.answer_prompt(&["stripe", ""]);
    ui.click("webhook-send-github.push");
    ui.answer_prompt(&["http://127.0.0.1:4021", ""]);
    assert_eq!(
        ui.fake.sent("webhook.trigger"),
        [
            json!({"workspaceId": "w1", "fixture": "github.push"}),
            json!({"workspaceId": "w1", "fixture": "github.push", "name": "stripe"}),
            json!({"workspaceId": "w1", "fixture": "github.push", "to": "http://127.0.0.1:4021"}),
        ]
    );
}

#[gpui_kit::test]
fn a_sample_event_signed_with_a_sealed_vault_unseals_it_first(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_webhooks(cx);
    ui.fake.reply("webhook.trigger", json!({"status": 200}));
    ui.fake.forget_calls();
    ui.click("webhook-send-github.push");
    ui.answer_prompt(&["stripe", "locked"]);
    assert!(ui.fake.sent("webhook.trigger").is_empty(), "the passphrase is asked first");
    ui.answer_prompt(&["hunter2"]);
    assert_eq!(ui.fake.sent("vault.unseal"), [json!({"workspaceId": "w1", "id": "vt2", "passphrase": "hunter2"})]);
    assert_eq!(
        ui.fake.sent("webhook.trigger"),
        [json!({"workspaceId": "w1", "fixture": "github.push", "name": "stripe", "vault": "Locked"})]
    );
    // An unsealed vault signs straight away.
    ui.click("webhook-send-github.push");
    ui.answer_prompt(&["stripe", "Prod"]);
    assert_eq!(ui.fake.sent("webhook.trigger")[1]["vault"], "Prod");
    assert_eq!(ui.fake.sent("vault.unseal").len(), 1);
}
