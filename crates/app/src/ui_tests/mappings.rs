//! Mappings: name-to-port routes of the reverse proxy.

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn no_mappings_show_the_empty_state_with_a_way_to_add_one(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Mappings);
    assert!(ui.has("empty-state"));
    assert!(ui.has("empty-new-mapping"));
    assert!(!ui.has("mapping-enabled-m1"));
}

#[gpui_kit::test]
fn mappings_are_listed_and_only_a_usable_one_has_a_switch(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mappings);
    assert!(!ui.has("empty-state"));
    assert!(ui.has("mapping-enabled-m1"), "an ordinary mapping has its switch");
    assert!(!ui.has("mapping-enabled-m2"), "a deleted one does not");
    assert!(!ui.has("mapping-enabled-m3"), "nor does one a newer Bifurc saved");
    assert!(ui.has("mapping-revert-delete-m2") && ui.has("mapping-confirm-delete-m2"));
}

#[gpui_kit::test]
fn the_switch_turns_a_mapping_off_through_the_engine(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mappings);
    ui.fake.forget_calls();
    ui.click("mapping-enabled-m1");
    assert_eq!(
        ui.fake.sent("entity.setEnabled"),
        [json!({"workspaceId": "w1", "kind": "mappings", "id": "m1", "enabled": false})]
    );
    assert!(!ui.fake.sent("config.get").is_empty(), "the snapshot is read again afterwards");
}

#[gpui_kit::test]
fn a_deleted_mapping_can_be_taken_back_or_deleted_for_good(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mappings);
    ui.fake.forget_calls();
    ui.click("mapping-revert-delete-m2");
    assert_eq!(ui.fake.sent("entity.revertDelete"), [json!({"workspaceId": "w1", "kind": "mappings", "id": "m2"})]);
    ui.click("mapping-confirm-delete-m2");
    assert_eq!(ui.fake.sent("entity.confirmDelete"), [json!({"workspaceId": "w1", "kind": "mappings", "id": "m2"})]);
}

#[gpui_kit::test]
fn a_new_mapping_is_created_with_its_domain_and_target(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mappings);
    ui.click("new-mapping");
    ui.fake.forget_calls();
    ui.answer_prompt(&["shop", "localhost:8080"]);
    let created = ui.fake.sent("entity.create");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["kind"], "mappings");
    assert_eq!(created[0]["entity"]["domain"], "shop.localhost");
    assert_eq!(created[0]["entity"]["target"], "localhost:8080");
}

#[gpui_kit::test]
fn a_prompt_with_a_required_field_empty_creates_nothing(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Mappings);
    ui.click("new-mapping");
    ui.fake.forget_calls();
    ui.click("prompt-confirm");
    assert!(ui.fake.sent("entity.create").is_empty());
    assert!(ui.has("prompt-confirm"), "the dialog stays open for the missing name");
}
