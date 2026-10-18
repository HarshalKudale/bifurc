//! Record and replay in the Capture toolbar.

use serde_json::json;

use super::open_populated;
use crate::shell::Screen;

#[gpui_kit::test]
fn record_asks_for_a_name_and_hosts_and_follows_the_engine_event(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Capture);
    ui.fake.reply("cassette.recordStart", json!({"name": "checkout", "hosts": ["a.test", "b.test"]}));
    ui.fake.forget_calls();
    ui.click("capture-record");
    ui.answer_prompt(&["checkout", "a.test, b.test"]);
    assert_eq!(
        ui.fake.sent("cassette.recordStart"),
        [json!({"workspaceId": "w1", "name": "checkout", "hosts": ["a.test", "b.test"]})]
    );
    ui.fake.emit(
        "event.cassette.changed",
        json!({"record": {"recording": {"name": "checkout", "workspaceId": "w1", "hosts": [], "interactions": 3}},
               "replay": {"replaying": null}}),
    );
    ui.settle();
    ui.fake.forget_calls();
    ui.click("capture-replay");
    assert!(ui.fake.sent("cassette.list").is_empty(), "replay is disabled while recording");
    ui.fake.reply("cassette.recordStop", json!({"name": "checkout", "interactions": 3, "masked": 0}));
    ui.click("capture-record");
    assert_eq!(ui.fake.sent("cassette.recordStop"), [json!({})]);
}

#[gpui_kit::test]
fn replay_lists_the_cassettes_and_starts_the_picked_one(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Capture);
    ui.fake.reply(
        "cassette.list",
        json!({"cassettes": [
            {"name": "checkout", "hosts": ["a.test"], "interactions": 3},
            {"name": "smoke", "hosts": ["b.test"], "interactions": 1},
        ]}),
    );
    ui.fake.reply("cassette.replayStart", json!({"name": "smoke", "hosts": ["b.test"], "interactions": 1}));
    ui.fake.forget_calls();
    ui.click("capture-replay");
    assert_eq!(ui.fake.sent("cassette.list"), [json!({"workspaceId": "w1"})]);
    assert!(ui.has(("cassette", 0usize)) && ui.has(("cassette", 1usize)));
    ui.click(("cassette", 1usize));
    ui.click("cassette-passthrough");
    ui.click("replay-confirm");
    assert_eq!(
        ui.fake.sent("cassette.replayStart"),
        [json!({"workspaceId": "w1", "name": "smoke", "passthrough": true})]
    );
    ui.fake.emit(
        "event.cassette.changed",
        json!({"record": {"recording": null},
               "replay": {"replaying": {"name": "smoke", "workspaceId": "w1", "hosts": [], "served": 0, "unmatchedCount": 2}}}),
    );
    ui.settle();
    ui.fake.reply("cassette.replayStop", json!({"name": "smoke", "served": 4, "unmatchedCount": 2, "unmatched": []}));
    ui.fake.forget_calls();
    ui.click("capture-record");
    assert!(!ui.has("prompt-confirm"), "record is disabled while replaying");
    ui.click("capture-replay");
    assert_eq!(ui.fake.sent("cassette.replayStop"), [json!({})]);
}
