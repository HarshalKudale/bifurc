//! Vaults: encrypted secrets, sealed until their passphrase is given.

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn no_vaults_show_the_empty_state_with_a_way_to_create_one(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Vaults);
    assert!(ui.has("empty-state"));
    assert!(ui.has("empty-new-vault"));
    assert!(!ui.has("seal"));
}

#[gpui_kit::test]
fn vaults_are_listed_and_an_unsealed_one_shows_its_secret_names(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    assert!(ui.has("vault-vt1") && ui.has("vault-vt2"));
    // The first vault is open: its secrets are listed by name, each with its actions.
    for name in ["API_KEY", "DB_PASS"] {
        for action in ["copy", "replace", "remove"] {
            assert!(ui.has(format!("{action}-{name}")), "{action}-{name}");
        }
    }
    assert!(ui.has("seal") && ui.has("add-secret"));
    assert!(!ui.has("unseal"));
}

#[gpui_kit::test]
fn a_sealed_vault_offers_to_be_unsealed_and_lists_no_secrets(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.click("vault-vt2");
    assert!(ui.has("unseal"));
    assert!(!ui.has("seal") && !ui.has("copy-API_KEY"));
    ui.fake.forget_calls();
    ui.click("unseal");
    ui.answer_prompt(&["correct horse"]);
    assert_eq!(
        ui.fake.sent("vault.unseal"),
        [json!({"workspaceId": "w1", "id": "vt2", "passphrase": "correct horse"})]
    );
}

#[gpui_kit::test]
fn sealing_sends_the_vault_id(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.fake.forget_calls();
    ui.click("seal");
    assert_eq!(ui.fake.sent("vault.seal"), [json!({"workspaceId": "w1", "id": "vt1"})]);
}

#[gpui_kit::test]
fn adding_and_replacing_a_secret_sends_its_name_and_value(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.click("add-secret");
    ui.fake.forget_calls();
    ui.answer_prompt(&["STRIPE_KEY", "sk_test"]);
    assert_eq!(
        ui.fake.sent("vault.setSecret"),
        [json!({"workspaceId": "w1", "id": "vt1", "name": "STRIPE_KEY", "value": "sk_test"})]
    );
    ui.click("replace-API_KEY");
    ui.fake.forget_calls();
    ui.answer_prompt(&["rotated"]);
    assert_eq!(
        ui.fake.sent("vault.setSecret"),
        [json!({"workspaceId": "w1", "id": "vt1", "name": "API_KEY", "value": "rotated"})]
    );
}

#[gpui_kit::test]
fn removing_a_secret_asks_first(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.fake.forget_calls();
    ui.click("remove-DB_PASS");
    assert!(ui.fake.sent("vault.deleteSecret").is_empty());
    ui.press("enter");
    assert_eq!(ui.fake.sent("vault.deleteSecret"), [json!({"workspaceId": "w1", "id": "vt1", "name": "DB_PASS"})]);
}

#[gpui_kit::test]
fn a_new_vault_needs_the_passphrase_twice(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.click("new-vault");
    ui.fake.forget_calls();
    ui.answer_prompt(&["team", "one", "two"]);
    assert!(ui.fake.sent("vault.create").is_empty(), "differing passphrases create nothing");
    ui.click("new-vault");
    ui.answer_prompt(&["team", "same", "same"]);
    assert_eq!(ui.fake.sent("vault.create"), [json!({"workspaceId": "w1", "name": "team", "passphrase": "same"})]);
}

#[gpui_kit::test]
fn renaming_a_vault_sends_the_new_name(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.click("rename-vault");
    ui.fake.forget_calls();
    ui.answer_prompt(&["Production"]);
    assert_eq!(ui.fake.sent("vault.rename"), [json!({"workspaceId": "w1", "id": "vt1", "name": "Production"})]);
}
