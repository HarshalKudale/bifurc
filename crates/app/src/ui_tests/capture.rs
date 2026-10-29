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
    // The list is not empty; the one empty state is the detail panel's, with nothing selected.
    assert_eq!(ui.count("empty-state"), 1);
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

fn grep_answer(matched: &[&str]) -> serde_json::Value {
    let lines: Vec<_> = matched
        .iter()
        .enumerate()
        .map(|(seq, text)| json!({"service": "api", "seq": seq, "ts": 1, "stream": "stdout", "text": text, "match": true}))
        .collect();
    json!({"lines": lines, "traced": true})
}

/// Capture with `c1` (traced as `tr-1`) selected and its detail loaded.
fn open_selected(cx: &mut gpui_kit::TestAppContext) -> crate::test_support::Ui {
    let mut ui = open_populated(cx);
    ui.fake.reply(
        "capture.get",
        json!({"entry": {"id": "c1", "durationMs": 40, "method": "GET",
            "url": "https://api.test/users?x=1", "status": 200, "traceId": "tr-1"}}),
    );
    ui.show(Screen::Capture);
    ui.click("capture-c1");
    ui
}

#[gpui_kit::test]
fn the_logs_tab_searches_the_dev_servers_only_when_it_is_opened(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_selected(cx);
    ui.fake.reply("devservice.grep", grep_answer(&["GET /users tr-1"]));
    assert!(ui.fake.sent("devservice.grep").is_empty(), "the tab is not open");
    ui.click_within("capture-section-tabs", 3usize);
    let asked = ui.fake.sent("devservice.grep");
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["needles"], json!(["c1", "tr-1"]));
    assert_eq!(asked[0]["workspaceId"], "w1");
    assert!(ui.has(("capture-log", 0usize)) && !ui.has("capture-logs-approximate"));
}

#[gpui_kit::test]
fn without_the_id_in_any_line_the_lines_near_the_request_are_shown_as_approximate(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_selected(cx);
    ui.fake.reply_with("devservice.grep", |payload| {
        Ok(if payload.get("sinceTs").is_some() {
            grep_answer(&["GET /users 500"])
        } else {
            json!({"lines": [], "traced": false})
        })
    });
    ui.click_within("capture-section-tabs", 3usize);
    let asked = ui.fake.sent("devservice.grep");
    assert_eq!(asked.len(), 2);
    assert_eq!(
        (&asked[1]["needles"], &asked[1]["sinceTs"], &asked[1]["untilTs"]),
        (&json!(["/users"]), &json!(-999), &json!(2041))
    );
    assert!(ui.has("capture-logs-approximate") && ui.has(("capture-log", 0usize)));
}

#[gpui_kit::test]
fn when_nothing_matches_the_logs_tab_says_why(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_selected(cx);
    ui.fake.reply("devservice.grep", json!({"lines": [], "traced": false}));
    ui.click_within("capture-section-tabs", 3usize);
    assert!(ui.has("empty-state") && !ui.has(("capture-log", 0usize)));
    // The reason follows from the running dev servers (the fixture has one) and whether tracing stamped an id.
    ui.fake.reply("devservice.list", json!({"services": []}));
    ui.click("capture-c2");
    ui.click_within("capture-section-tabs", 3usize);
    assert!(ui.has("empty-state"));
}

fn open_pinnable(cx: &mut gpui_kit::TestAppContext, pinned: &[&str]) -> crate::test_support::Ui {
    let mut ui = open_populated(cx);
    let entry = |id: &str, ts: u64| {
        let mut entry = json!({"id": id, "timestamp": ts, "method": "GET", "url": "https://api.test/x", "status": 200, "via": "proxy"});
        if pinned.contains(&id) {
            entry["pinned"] = json!(true);
        }
        entry
    };
    ui.fake.reply(
        "capture.list",
        json!({"paused": false, "disableCache": true, "limit": 200, "entries": [entry("c2", 2), entry("c1", 1)]}),
    );
    ui.fake.reply("capture.pin", json!({"ok": true, "changed": 1, "missing": []}));
    ui.show(Screen::Capture);
    ui
}

#[gpui_kit::test]
fn a_pinned_exchange_is_marked_in_the_list(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_pinnable(cx, &["c1"]);
    assert!(ui.has("capture-pinned-c1") && !ui.has("capture-pinned-c2"));
}

#[gpui_kit::test]
fn the_detail_pins_and_unpins_the_selected_exchange(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_pinnable(cx, &[]);
    ui.click("capture-c1");
    ui.fake.forget_calls();
    ui.click("capture-pin");
    assert_eq!(ui.fake.sent("capture.pin"), [json!({"workspaceId": "w1", "ids": ["c1"], "pinned": true})]);
    assert!(ui.has("capture-pinned-c1"), "the marker shows at once");
    ui.fake.forget_calls();
    ui.click("capture-pin");
    assert_eq!(ui.fake.sent("capture.pin"), [json!({"workspaceId": "w1", "ids": ["c1"], "pinned": false})]);
    assert!(!ui.has("capture-pinned-c1"));
}

#[gpui_kit::test]
fn the_ticked_exchanges_are_pinned_and_unpinned_together(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_pinnable(cx, &[]);
    assert!(!ui.has("capture-pin-checked"), "nothing is ticked yet");
    ui.click("capture-check-c1");
    ui.click("capture-check-c2");
    ui.fake.forget_calls();
    ui.click("capture-pin-checked");
    let ids = json!(["c1", "c2"]);
    assert_eq!(ui.fake.sent("capture.pin"), [json!({"workspaceId": "w1", "ids": ids, "pinned": true})]);
    assert!(ui.has("capture-pinned-c1") && ui.has("capture-pinned-c2"));
    ui.fake.forget_calls();
    ui.click("capture-unpin-checked");
    assert_eq!(ui.fake.sent("capture.pin"), [json!({"workspaceId": "w1", "ids": ids, "pinned": false})]);
    assert!(!ui.has("capture-pinned-c1") && !ui.has("capture-pinned-c2"));
}

#[gpui_kit::test]
fn pinned_only_narrows_the_list(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_pinnable(cx, &["c1"]);
    ui.click("capture-pinned-only");
    assert!(ui.has("capture-c1") && !ui.has("capture-c2"));
}

#[gpui_kit::test]
fn webhook_traffic_is_listed_unless_its_chip_is_off(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("capture.list", json!({"paused": false, "disableCache": true, "entries": [
        {"id": "w9", "timestamp": 3, "method": "POST", "url": "http://127.0.0.1:4020/hook", "status": 200, "via": "proxy", "origin": "webhook"},
    ]}));
    ui.show(Screen::Capture);
    assert!(ui.has("capture-w9"));
}

#[gpui_kit::test]
fn without_https_interception_capture_says_what_the_extension_covers(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("server.status", json!({"running": true, "port": 8080, "https": false}));
    ui.fake.emit("event.server.error", json!({}));
    ui.settle();
    ui.show(Screen::Capture);
    assert!(ui.has("capture-https-settings"));
    // Decrypting HTTPS itself, the proxy captures everything: nothing to explain.
    ui.fake.reply("server.status", json!({"running": true, "port": 8080, "https": true}));
    ui.fake.emit("event.server.error", json!({}));
    ui.settle();
    assert!(!ui.has("capture-https-settings"));
}
