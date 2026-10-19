//! First-run setup, shown in place of the shell until the app is set up and has a workspace.

use serde_json::json;

use super::{Ui, open_populated};

fn not_onboarded(cx: &mut gpui_kit::TestAppContext) -> Ui {
    Ui::open(cx, |fake| {
        fake.reply(
            "setup.getState",
            json!({"gitInstalled": true, "onboarded": false, "termsAccepted": false, "privacyAccepted": false}),
        );
    })
}

#[gpui_kit::test]
fn a_set_up_app_with_a_workspace_goes_straight_to_the_shell(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    assert!(!ui.has("onboarding-continue"));
    assert!(ui.has("proxy-toggle"), "the shell's title bar is there");
}

#[gpui_kit::test]
fn an_app_that_was_never_set_up_starts_the_walkthrough(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = not_onboarded(cx);
    assert!(ui.has("onboarding-continue") && ui.has("onboarding-language"));
    assert!(!ui.has("onboarding-back"), "there is nothing before the first step");
    assert!(!ui.has("proxy-toggle"), "the shell is not shown yet");
}

#[gpui_kit::test]
fn a_workspace_less_app_is_walked_through_setup_even_when_it_was_set_up(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = Ui::open(cx, |fake| fake.config(json!({"workspaces": []})));
    assert!(ui.has("onboarding-continue"));
    assert!(!ui.has("proxy-toggle"));
}

#[gpui_kit::test]
fn the_terms_must_be_accepted_before_the_engine_is_told(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = not_onboarded(cx);
    ui.click("onboarding-continue");
    assert!(ui.has("accept-terms") && ui.has("onboarding-back"), "the second step asks for the terms");
    ui.fake.forget_calls();
    ui.click("onboarding-continue");
    assert!(ui.fake.sent("setup.acceptTerms").is_empty(), "Continue is disabled until they are ticked");
    ui.click("accept-terms");
    ui.click("onboarding-continue");
    assert_eq!(ui.fake.sent("setup.acceptTerms"), [json!({})]);
    assert!(ui.has("accept-privacy"), "the privacy policy follows");
    ui.click("accept-privacy");
    ui.click("onboarding-continue");
    assert_eq!(ui.fake.sent("setup.acceptPrivacy"), [json!({})]);
}

#[gpui_kit::test]
fn back_returns_to_the_previous_step(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = not_onboarded(cx);
    ui.click("onboarding-continue");
    assert!(ui.has("accept-terms"));
    ui.click("onboarding-back");
    assert!(!ui.has("accept-terms") && ui.has("onboarding-language"));
}
