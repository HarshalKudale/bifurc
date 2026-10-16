//! Settings: this client's look, and the engine's own settings, in pages.

use serde_json::json;

use super::open_populated;
use crate::shell::Screen;

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
