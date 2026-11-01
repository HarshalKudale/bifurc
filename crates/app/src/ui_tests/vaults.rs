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

/// Types `values` into the fields of the step on screen, from the one with focus, Tab moving on.
fn fill(ui: &mut crate::test_support::Ui, values: &[&str]) {
    for (ix, value) in values.iter().enumerate() {
        if ix > 0 {
            ui.press("tab");
        }
        ui.press("ctrl-a");
        ui.input(value);
    }
}

#[gpui_kit::test]
fn a_local_vault_is_made_in_two_steps_and_needs_the_passphrase_twice(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.click("new-vault");
    assert!(ui.has("provider-local") && ui.has("provider-aws") && ui.has("provider-bitwarden"));
    ui.click("provider-local");
    ui.click("vault-setup-next");
    ui.fake.forget_calls();
    fill(&mut ui, &["team", "correct horse", "another horse"]);
    ui.click("vault-setup-next");
    assert!(ui.fake.sent("vault.create").is_empty(), "differing passphrases create nothing");
    ui.click("vault-setup-back");
    ui.click("vault-setup-next");
    fill(&mut ui, &["team", "correct horse", "correct horse"]);
    ui.click("vault-setup-next");
    assert_eq!(
        ui.fake.sent("vault.create"),
        [json!({"workspaceId": "w1", "name": "team", "passphrase": "correct horse"})]
    );
}

#[gpui_kit::test]
fn a_linked_vault_is_made_with_its_source_which_can_be_pulled_before(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.click("new-vault");
    ui.click("provider-aws");
    ui.click("vault-setup-next");
    fill(&mut ui, &["team", "prod/app", "eu-west-1"]);
    ui.click("vault-setup-next");
    ui.fake.forget_calls();
    ui.click("check-source");
    let settings = json!({"secretId": "prod/app", "region": "eu-west-1"});
    assert_eq!(ui.fake.sent("vault.test"), [json!({"workspaceId": "w1", "provider": "aws", "settings": settings})]);
    ui.click("vault-setup-next");
    assert_eq!(
        ui.fake.sent("vault.create"),
        [json!({"workspaceId": "w1", "name": "team", "provider": "aws", "settings": settings, "credential": ""})]
    );
}

#[gpui_kit::test]
fn a_linked_vaults_secrets_are_not_edited_here(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("vault.list", json!([
        {"id": "vt3", "name": "Team", "provider": "aws", "providerLabel": "AWS Secrets Manager", "state": "unsealed",
         "secrets": ["STRIPE"], "path": "vaults/vt3.json", "approved": true, "settings": {"secretId": "prod/app"}},
    ]));
    ui.fake.emit("event.vault.changed", json!({"workspaceId": "w1", "id": "vt3"}));
    ui.show(Screen::Vaults);
    assert!(ui.has("copy-STRIPE"));
    assert!(!ui.has("add-secret") && !ui.has("replace-STRIPE") && !ui.has("remove-STRIPE"));
}

#[gpui_kit::test]
fn a_linked_vault_unseals_without_a_passphrase(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.click("vault-vt3");
    assert!(!ui.has("change-passphrase") && ui.has("check-vault"));
    ui.fake.forget_calls();
    ui.click("unseal");
    assert_eq!(ui.fake.sent("vault.unseal"), [json!({"workspaceId": "w1", "id": "vt3"})]);
}

#[gpui_kit::test]
fn a_vault_a_sync_changed_is_reviewed_and_approved_before_use(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.click("vault-vt4");
    assert!(ui.has("setup-vault"));
    assert!(!ui.has("unseal"));
    ui.click("setup-vault");
    ui.click("vault-setup-next");
    ui.fake.forget_calls();
    ui.click("vault-setup-next");
    assert_eq!(
        ui.fake.sent("vault.configure"),
        [json!({"workspaceId": "w1", "id": "vt4", "settings": {"secretId": "prod/db", "region": "us-east-1"}})]
    );
}

#[gpui_kit::test]
fn a_vault_moved_off_the_repository_says_so_until_acknowledged(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply(
        "vault.list",
        json!([
            {"id": "vt2", "name": "Locked", "provider": "local", "state": "sealed", "secrets": [], "path": null,
             "approved": true, "movedAt": 5},
        ]),
    );
    ui.fake.emit("event.vault.changed", json!({"workspaceId": "w1", "id": "vt2"}));
    ui.show(Screen::Vaults);
    assert!(ui.has("notification"), "the app says it once");
    assert!(ui.has("vault-moved-ok"));
    ui.fake.forget_calls();
    ui.click("vault-moved-ok");
    assert_eq!(ui.fake.sent("vault.acknowledge"), [json!({"workspaceId": "w1", "id": "vt2"})]);
}

#[gpui_kit::test]
fn the_beta_notice_heads_an_empty_screen_and_a_vault(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Vaults);
    assert!(ui.has("vaults-beta"));

    let mut ui = open_populated(cx);
    ui.show(Screen::Vaults);
    ui.click("vault-vt2");
    assert!(ui.has("vaults-beta"), "on a vault's detail too");
}

#[gpui_kit::test]
fn dismissing_the_moved_notice_acknowledges_every_moved_vault_and_it_stays_gone(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    let moved = |at: serde_json::Value| {
        json!([
            {"id": "vt1", "name": "Keys", "provider": "local", "state": "unsealed", "secrets": [], "path": null,
             "approved": true, "movedAt": at},
            {"id": "vt2", "name": "Locked", "provider": "local", "state": "sealed", "secrets": [], "path": null,
             "approved": true, "movedAt": at},
        ])
    };
    ui.fake.reply("vault.list", moved(json!(5)));
    ui.fake.emit("event.vault.changed", json!({"workspaceId": "w1", "id": "vt1"}));
    ui.show(Screen::Vaults);
    assert!(ui.has("vault-moved-ok"));
    ui.fake.forget_calls();
    ui.click("vault-moved-ok");
    let mut sent = ui.fake.sent("vault.acknowledge");
    sent.sort_by_key(|call| call["id"].as_str().unwrap().to_owned());
    assert_eq!(sent, [json!({"workspaceId": "w1", "id": "vt1"}), json!({"workspaceId": "w1", "id": "vt2"})]);

    // The engine now says nothing was moved; reopening the screen shows no notice.
    ui.fake.reply("vault.list", moved(serde_json::Value::Null));
    ui.fake.emit("event.vault.changed", json!({"workspaceId": "w1", "id": "vt1"}));
    ui.show(Screen::Vaults);
    assert!(!ui.has("vault-moved-ok"));
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
