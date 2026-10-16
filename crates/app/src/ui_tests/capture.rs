//! Capture: the traffic the proxy has seen, live.

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn nothing_captured_shows_the_empty_state(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Capture);
    assert!(ui.has("empty-state"));
    assert!(ui.has("capture-pause") && ui.has("capture-clear"));
    assert!(!ui.has("capture-c1"));
}

#[gpui_kit::test]
fn captured_exchanges_are_listed_and_were_asked_for_by_workspace(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Capture);
    assert!(ui.has("capture-c1") && ui.has("capture-c2"));
    assert!(!ui.has("empty-state"));
    assert!(ui.fake.sent("capture.list").contains(&json!({"workspaceId": "w1", "summary": true})));
    assert!(!ui.has("capture-close-detail"), "nothing is selected yet");
}

#[gpui_kit::test]
fn selecting_an_exchange_loads_its_detail_and_closing_hides_it(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply(
        "capture.get",
        json!({"entry": {"id": "c1", "method": "GET", "url": "https://api.test/users", "status": 200}}),
    );
    ui.show(Screen::Capture);
    ui.fake.forget_calls();
    ui.click("capture-c1");
    assert_eq!(ui.fake.sent("capture.get"), [json!({"workspaceId": "w1", "id": "c1"})]);
    assert!(ui.has("capture-close-detail") && ui.has("capture-save-request") && ui.has("capture-save-mock"));
    ui.click("capture-close-detail");
    assert!(!ui.has("capture-close-detail"));
}

#[gpui_kit::test]
fn removing_the_selected_exchange_drops_it_and_tells_the_engine(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Capture);
    ui.click("capture-c2");
    ui.fake.forget_calls();
    ui.click("capture-remove");
    assert_eq!(ui.fake.sent("capture.remove"), [json!({"workspaceId": "w1", "ids": ["c2"]})]);
    assert!(!ui.has("capture-c2") && ui.has("capture-c1"));
}

#[gpui_kit::test]
fn pause_and_clear_are_engine_commands(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Capture);
    ui.fake.reply("capture.setPaused", json!({"paused": true}));
    ui.fake.forget_calls();
    ui.click("capture-pause");
    assert_eq!(ui.fake.sent("capture.setPaused"), [json!({"workspaceId": "w1", "paused": true})]);
    ui.fake.forget_calls();
    ui.click("capture-clear");
    assert_eq!(ui.fake.sent("capture.clear"), [json!({"workspaceId": "w1"})]);
    assert!(!ui.has("capture-c1") && ui.has("empty-state"), "the list is empty once cleared");
}

#[gpui_kit::test]
fn live_traffic_is_added_while_capturing_and_ignored_while_paused(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Capture);
    let entry = |id: &str| json!({"entries": [{"id": id, "timestamp": 9, "method": "GET", "url": "https://api.test/x", "workspaceId": "w1"}]});
    ui.fake.emit("event.log.entry", entry("c9"));
    ui.settle();
    assert!(ui.has("capture-c9"));
    ui.fake.reply("capture.setPaused", json!({"paused": true}));
    ui.click("capture-pause");
    ui.fake.emit("event.log.entry", entry("c10"));
    ui.settle();
    assert!(!ui.has("capture-c10"));
}

#[gpui_kit::test]
fn the_cache_switch_tells_the_engine(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Capture);
    ui.fake.forget_calls();
    ui.click("capture-disable-cache");
    assert_eq!(ui.fake.sent("capture.setDisableCache"), [json!({"disableCache": false})]);
}
