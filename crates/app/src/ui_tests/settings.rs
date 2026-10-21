//! Settings: this client's look, and the engine's own settings, in pages.

use serde_json::json;

use super::open_populated;
use crate::shell::Screen;
use crate::test_support::Ui;

/// The sidebar entries of the settings pages, in order: Appearance, Companions, Servers, Requests, HTTPS, About,
/// Licenses.
const PAGES: [&str; 7] = ["0-0", "0-1", "0-2", "0-3", "0-4", "0-5", "0-6"];

#[gpui_kit::test]
fn every_page_is_listed_and_appearance_is_open(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Settings);
    for page in PAGES {
        assert!(ui.has(page), "page {page}");
    }
    assert!(!ui.has("restart-engine"), "the Servers page is not open yet");
}

#[gpui_kit::test]
fn the_servers_page_starts_either_proxy_through_the_engine(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Settings);
    ui.click("0-2");
    assert!(ui.has("restart-engine") && ui.has("proxy-start-stop") && ui.has("reverse-proxy-start-stop"));
    ui.fake.forget_calls();
    ui.click("proxy-start-stop");
    assert_eq!(ui.fake.sent("server.start"), [json!({})]);
    ui.fake.forget_calls();
    ui.click("reverse-proxy-start-stop");
    assert_eq!(ui.fake.sent("server.start"), [json!({"target": "reverse"})]);
}

#[gpui_kit::test]
fn a_running_proxy_is_stopped_from_the_same_button(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("server.status", json!({"running": true, "port": 8080}));
    ui.fake.emit("event.server.error", json!({}));
    ui.settle();
    ui.show(Screen::Settings);
    ui.click("0-2");
    ui.fake.forget_calls();
    ui.click("proxy-start-stop");
    assert_eq!(ui.fake.sent("server.stop"), [json!({})]);
}

#[gpui_kit::test]
fn the_https_page_generates_a_certificate(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Settings);
    ui.click("0-4");
    assert!(ui.has("generate-certificate") && ui.has("import-certificate") && ui.has("install-certificate"));
    ui.fake.forget_calls();
    ui.click("generate-certificate");
    assert_eq!(ui.fake.sent("tls.generate"), [json!({})]);
    assert!(!ui.fake.sent("tls.certStatus").is_empty(), "the certificate's state is read again");
}

#[gpui_kit::test]
fn the_about_page_can_look_for_an_update(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Settings);
    ui.click("0-5");
    assert!(ui.has("check-for-update"));
}

#[gpui_kit::test]
fn the_licenses_page_lists_the_bundled_notices(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Settings);
    ui.click("0-6");
    assert!(ui.has("view-notices-0"), "each license has a button that shows its notices");
    assert!(ui.has("view-notices-1"));
}

#[gpui_kit::test]
fn a_license_group_opens_its_notices_in_a_dialog(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Settings);
    ui.click("0-6");
    // The groups are listed below the fold; scroll them into view.
    ui.scroll_at(900., 500., 30.);
    assert!(!ui.has("notices-close"));
    ui.click("view-notices-0");
    assert!(ui.has("notices-close"), "the notices are shown");
    ui.click("notices-close");
    assert!(!ui.has("notices-close"));
}

/// The Servers page over a config that holds `stored`.
fn servers_page(cx: &mut gpui_kit::TestAppContext, stored: serde_json::Value) -> crate::test_support::Ui {
    let mut ui = Ui::open(cx, |fake| {
        super::fixtures::populated(fake);
        let mut config = super::fixtures::config();
        config.as_object_mut().unwrap().extend(stored.as_object().cloned().unwrap());
        fake.config(config);
    });
    ui.show(Screen::Settings);
    ui.click("0-2");
    ui
}

/// Scrolls the page down until the row is in view.
fn reveal(ui: &mut Ui, id: &str) {
    for _ in 0..30 {
        if ui.visible(id.to_owned()) {
            return;
        }
        ui.scroll_at(900., 500., 4.);
    }
    panic!("{id} never came into view");
}

/// Types `text` over whatever the row holds and commits it with Enter.
fn type_into(ui: &mut crate::test_support::Ui, id: &str, text: &str) {
    reveal(ui, id);
    ui.click(id.to_owned());
    ui.press("ctrl-a");
    if text.is_empty() {
        ui.press("backspace");
    } else {
        ui.input(text);
    }
    ui.fake.forget_calls();
    ui.press("enter");
}

#[gpui_kit::test]
fn the_four_rows_write_their_engine_keys(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = servers_page(cx, json!({}));
    for (id, key, text, value) in [
        ("setting-capture-limit", "captureLimit", "500", json!(500)),
        ("setting-trace-header", "traceHeader", "X-Req-Id", json!("X-Req-Id")),
        ("setting-mail-port", "mailPort", "2525", json!(2525)),
        ("setting-approval-timeout", "approvalTimeoutSeconds", "60", json!(60)),
    ] {
        type_into(&mut ui, id, text);
        assert_eq!(ui.fake.sent("settings.update"), [json!({ key: value })], "{key}");
    }
}

#[gpui_kit::test]
fn a_blank_row_goes_back_to_the_default(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = servers_page(
        cx,
        json!({"captureLimit": 500, "mailPort": 2525, "approvalTimeoutSeconds": 60, "traceHeader": "x-id"}),
    );
    for (id, key) in [
        ("setting-capture-limit", "captureLimit"),
        ("setting-trace-header", "traceHeader"),
        ("setting-mail-port", "mailPort"),
        ("setting-approval-timeout", "approvalTimeoutSeconds"),
    ] {
        type_into(&mut ui, id, "");
        // The engine reads null as unset, which is the default.
        assert_eq!(ui.fake.sent("settings.update"), [json!({ key: null })], "{key}");
    }
}

#[gpui_kit::test]
fn a_value_outside_the_engines_bounds_is_not_sent(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = servers_page(cx, json!({}));
    for (id, text) in [
        ("setting-capture-limit", "5"),
        ("setting-capture-limit", "10001"),
        ("setting-mail-port", "70000"),
        ("setting-approval-timeout", "9"),
        ("setting-trace-header", "not a header"),
    ] {
        type_into(&mut ui, id, text);
        assert!(ui.fake.sent("settings.update").is_empty(), "{id} = {text}");
    }
}

#[test]
fn what_was_typed_becomes_the_value_to_send() {
    use crate::screens::settings::{Accepts, setting_value};
    let ports = Accepts::Number { min: 1, max: 65_535 };
    assert_eq!(setting_value(ports, " 2525 "), Some(json!(2525)));
    assert_eq!(setting_value(ports, ""), Some(serde_json::Value::Null));
    assert_eq!(setting_value(ports, "0"), None);
    assert_eq!(setting_value(ports, "65536"), None);
    assert_eq!(setting_value(ports, "-1"), None);
    assert_eq!(setting_value(Accepts::HeaderName, "X-Req_Id"), Some(json!("X-Req_Id")));
    assert_eq!(setting_value(Accepts::HeaderName, "x id"), None);
}
