//! Claims as locks: another session's claim shows, makes the entity read-only, and can be taken over; the app
//! claims what it saves and lets go when the tab closes.

use std::time::Duration;

use serde_json::{Value, json};

use super::open_populated;
use crate::{shell::Screen, test_support::Ui};

fn claim(by: &str) -> Value {
    json!({"id": "k1", "kind": "mocks", "name": "Users mock", "by": by, "ts": 1, "expiresAt": 1_800_000})
}

fn changed(claims: Vec<Value>) -> Value {
    json!({"workspaceId": "w1", "claims": claims})
}

fn open_mock(cx: &mut gpui_kit::TestAppContext) -> Ui {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mocks);
    ui.click("tree-row-k1");
    ui
}

fn locked_error() -> String {
    json!({"code": "CONFLICT", "message": "claimed", "details": {"key": "claims.locked", "params": {
        "id": "k1", "kind": "mocks", "name": "Users mock", "claimedBy": "session:bot", "expiresAt": 1_800_000
    }}})
    .to_string()
}

#[gpui_kit::test]
fn a_claim_by_someone_else_shows_a_lock_and_a_banner(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_mock(cx);
    assert!(!ui.has("lock-banner") && !ui.has("tree-lock-k1"));
    ui.fake.emit("event.claims.changed", changed(vec![claim("session:bot")]));
    ui.settle();
    assert!(ui.has("lock-banner") && ui.has("tree-lock-k1"));
    ui.fake.emit("event.claims.changed", changed(vec![claim("app")]));
    ui.settle();
    assert!(!ui.has("lock-banner"), "the app's own claim is not a lock on the app");
    assert!(ui.has("tree-lock-k1"), "but it is still visible");
}

#[gpui_kit::test]
fn taking_over_asks_and_then_claims_by_force(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_mock(cx);
    ui.fake.emit("event.claims.changed", changed(vec![claim("session:bot")]));
    ui.settle();
    ui.fake.forget_calls();
    ui.click("lock-take-over");
    assert!(ui.fake.sent("claims.set").is_empty(), "not before the answer");
    ui.press("enter");
    assert_eq!(ui.fake.sent("claims.set"), [json!({"workspaceId": "w1", "id": "k1", "kind": "mocks", "force": true})]);
}

#[gpui_kit::test]
fn the_first_save_claims_and_later_ones_do_not(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_mock(cx);
    ui.fake.forget_calls();
    ui.click("mock-enabled");
    ui.advance(Duration::from_millis(700));
    let order: Vec<String> = ui.fake.calls().into_iter().map(|c| c.action).collect();
    assert_eq!(order, ["claims.set", "entity.update"]);
    assert_eq!(ui.fake.sent("claims.set"), [json!({"workspaceId": "w1", "id": "k1", "kind": "mocks"})]);
    // The engine tells the app it holds the claim; the next save only writes.
    ui.fake.emit("event.claims.changed", changed(vec![claim("app")]));
    ui.settle();
    ui.fake.forget_calls();
    ui.click("mock-enabled");
    ui.advance(Duration::from_millis(700));
    assert!(ui.fake.sent("claims.set").is_empty() && ui.fake.sent("entity.update").len() == 1);
}

#[gpui_kit::test]
fn closing_the_tab_releases_the_apps_claim(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_mock(cx);
    ui.fake.emit("event.claims.changed", changed(vec![claim("app")]));
    ui.settle();
    ui.fake.forget_calls();
    ui.click("entity-close");
    assert_eq!(ui.fake.sent("claims.release"), [json!({"workspaceId": "w1", "ids": ["k1"]})]);
}

#[gpui_kit::test]
fn a_save_refused_as_locked_shows_the_banner(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_mock(cx);
    ui.fake.reply_with("claims.set", |_| Err(locked_error()));
    ui.click("mock-enabled");
    ui.advance(Duration::from_millis(700));
    assert!(ui.has("lock-banner"), "who holds it comes from the refusal");
    assert!(ui.fake.sent("entity.update").is_empty(), "the write was not attempted");
}
