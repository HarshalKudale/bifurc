//! Environments: named sets of variables, one of them in use.

use std::time::Duration;

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn a_workspace_without_environments_still_has_the_global_one(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Environments);
    assert!(ui.has("environment-__global__"));
    assert!(!ui.has("environment-e1"));
    assert!(!ui.has("use-environment"), "the global set is always applied; there is nothing to switch to");
    assert!(ui.has("new-environment"));
}

#[gpui_kit::test]
fn every_environment_is_listed_and_the_one_in_use_is_open(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Environments);
    for id in ["__global__", "e1", "e2", "e3", "e4"] {
        assert!(ui.has(format!("environment-{id}")), "environment {id}");
    }
    assert!(ui.has("use-environment"), "an ordinary environment can be put in use");
    assert!(ui.has(("remove-pair", 1u64)), "Staging's variable is in the table");
    // Staging is in use, so its button is spent.
    ui.fake.forget_calls();
    ui.click("use-environment");
    assert!(ui.fake.sent("env.setActive").is_empty());
}

#[gpui_kit::test]
fn selecting_another_environment_and_using_it_tells_the_engine(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Environments);
    ui.click("environment-e2");
    assert!(!ui.has(("remove-pair", 1u64)), "Production has no variables");
    ui.fake.forget_calls();
    ui.click("use-environment");
    assert_eq!(ui.fake.sent("env.setActive"), [json!({"id": "e2"})]);
    assert!(!ui.fake.sent("config.get").is_empty());
}

#[gpui_kit::test]
fn the_global_environment_cannot_be_put_in_use_or_deleted(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Environments);
    ui.click("environment-__global__");
    assert!(!ui.has("use-environment"));
    ui.fake.forget_calls();
    ui.click("entity-delete");
    ui.press("enter");
    assert!(ui.fake.sent("entity.delete").is_empty());
}

#[gpui_kit::test]
fn removing_a_variable_is_written_after_the_pause(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Environments);
    ui.fake.forget_calls();
    ui.click(("remove-pair", 1u64));
    assert!(ui.fake.sent("entity.update").is_empty(), "not before the pause");
    ui.advance(Duration::from_millis(700));
    let updates = ui.fake.sent("entity.update");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["kind"], "environments");
    assert_eq!(updates[0]["entity"]["id"], "e1");
    assert_eq!(updates[0]["entity"]["variables"], json!([]));
}

#[gpui_kit::test]
fn a_new_environment_is_created_by_name(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Environments);
    ui.click("new-environment");
    ui.fake.forget_calls();
    ui.answer_prompt(&["Canary"]);
    assert_eq!(
        ui.fake.sent("entity.create"),
        [json!({"kind": "environments", "workspaceId": "w1", "entity": {"name": "Canary", "variables": []}})]
    );
}

#[gpui_kit::test]
fn deleting_asks_first_and_then_deletes_by_id(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Environments);
    ui.click("environment-e2");
    ui.fake.forget_calls();
    ui.click("entity-delete");
    assert!(ui.fake.sent("entity.delete").is_empty(), "nothing is deleted before the answer");
    ui.press("enter");
    assert_eq!(ui.fake.sent("entity.delete"), [json!({"kind": "environments", "id": "e2"})]);
}

#[gpui_kit::test]
fn a_locked_environment_explains_that_bifurc_must_be_updated(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Environments);
    ui.click("environment-e3");
    assert!(ui.has("needs-update-settings"));
    ui.click("needs-update-cancel");
    assert!(ui.has("use-environment"), "the selection stayed where it was");
}

#[gpui_kit::test]
fn a_deleted_environment_is_brought_back_or_deleted_for_good(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Environments);
    ui.click("environment-e4");
    assert!(ui.has("deleted-revert") && ui.has("deleted-confirm"));
    ui.fake.forget_calls();
    ui.click("deleted-revert");
    assert_eq!(ui.fake.sent("entity.revertDelete"), [json!({"workspaceId": "w1", "kind": "environments", "id": "e4"})]);
}
