//! Mail: the engine's local mail sink, live.

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

fn arrived(id: &str, subject: &str) -> serde_json::Value {
    json!({"received": {"id": id, "timestamp": 3000, "from": "new@app.test", "to": ["cy@dev.test"], "subject": subject}})
}

#[gpui_kit::test]
fn an_empty_inbox_says_where_to_point_the_smtp_settings(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Mail);
    assert!(ui.has("empty-state"));
    assert!(ui.has("mail-clear") && !ui.has("mail-m1"));
}

#[gpui_kit::test]
fn the_mails_are_listed_when_the_screen_is_shown(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.forget_calls();
    ui.show(Screen::Mail);
    assert!(ui.has("mail-m1") && ui.has("mail-m2"));
    assert!(!ui.has("mail-otp"), "nothing is selected yet");
    assert_eq!(ui.fake.sent("mail.list"), [json!({"last": 200})]);
    assert_eq!(ui.fake.sent("mail.status").len(), 1);
    assert!(!ui.has("mail-error"));
}

#[gpui_kit::test]
fn selecting_a_mail_fetches_it_and_shows_its_code_and_link(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mail);
    ui.fake.forget_calls();
    ui.click("mail-m1");
    assert_eq!(ui.fake.sent("mail.get"), [json!({"id": "m1"})]);
    assert!(ui.has("mail-otp") && ui.has("mail-copy-otp"));
    assert!(ui.has("mail-link") && ui.has("mail-open-link") && ui.has("mail-copy-link"));
    assert!(ui.has("mail-part-tabs"));
    // A mail with neither has neither row.
    ui.click("mail-m2");
    assert!(!ui.has("mail-otp") && !ui.has("mail-link"));
    assert!(ui.has("mail-part-tabs"), "an HTML-only mail still opens");
}

#[gpui_kit::test]
fn a_mail_the_sink_dropped_shows_an_empty_state(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mail);
    ui.fake.reply_with("mail.get", |_| Err("NOT_FOUND: dropped".to_owned()));
    ui.click("mail-m1");
    assert!(ui.has("empty-state") && !ui.has("mail-otp") && !ui.has("mail-part-tabs"));
}

#[gpui_kit::test]
fn a_mail_that_arrives_is_added_on_top_and_the_selection_stays(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mail);
    ui.click("mail-m1");
    ui.fake.forget_calls();
    ui.fake.emit("event.mail.changed", arrived("m3", "Welcome"));
    ui.settle();
    assert!(ui.has("mail-m3"));
    assert!(ui.has("mail-otp"), "the open mail is still open");
    assert!(ui.fake.sent("mail.list").is_empty() && ui.fake.sent("mail.get").is_empty(), "no refetch");
    // The same mail again (the list read raced the event) is not listed twice.
    ui.fake.emit("event.mail.changed", arrived("m3", "Welcome"));
    ui.settle();
    assert_eq!(ui.count("mail-m3"), 1);
}

#[gpui_kit::test]
fn the_inbox_being_cleared_elsewhere_empties_the_screen(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mail);
    ui.click("mail-m1");
    ui.fake.emit("event.mail.changed", json!({"cleared": 2}));
    ui.settle();
    assert!(ui.has("empty-state"));
    assert!(!ui.has("mail-m1") && !ui.has("mail-otp"));
}

#[gpui_kit::test]
fn clear_inbox_asks_first_then_tells_the_engine(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mail);
    ui.fake.forget_calls();
    ui.click("mail-clear");
    assert!(ui.fake.sent("mail.clear").is_empty());
    ui.press("enter");
    assert_eq!(ui.fake.sent("mail.clear"), [json!({})]);
    assert!(ui.has("empty-state") && !ui.has("mail-m1"));
}

#[gpui_kit::test]
fn the_filter_narrows_the_list_by_recipient_sender_and_subject(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mail);
    ui.click("mail-filter");
    ui.input("bob@");
    assert!(ui.has("mail-m2") && !ui.has("mail-m1"));
    ui.press("ctrl-a");
    ui.input("VERIFY");
    assert!(ui.has("mail-m1") && !ui.has("mail-m2"));
    ui.press("ctrl-a");
    ui.input("nothing like this");
    assert!(ui.has("empty-state") && !ui.has("mail-m1") && !ui.has("mail-m2"));
}

#[gpui_kit::test]
fn a_sink_that_could_not_start_says_why(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply(
        "mail.status",
        json!({"port": 1025, "running": false, "error": "Port 1025 is already in use", "count": 0}),
    );
    ui.show(Screen::Mail);
    assert!(ui.has("mail-error"));
}
