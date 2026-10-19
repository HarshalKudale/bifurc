//! Proxy rules: patterns whose traffic is sent somewhere else.

use std::time::Duration;

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn no_rules_show_the_empty_state_with_a_way_to_add_one(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::ProxyRules);
    assert!(ui.has("empty-state"));
    assert!(ui.has("empty-new-rule"));
    assert!(!ui.has("rule-pattern"));
}

#[gpui_kit::test]
fn rules_are_listed_and_the_first_is_open(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::ProxyRules);
    assert!(ui.has("rule-p1") && ui.has("rule-p2"));
    assert!(ui.has("rule-deleted-p3"), "a deleted rule stays listed");
    assert!(ui.has("rule-locked-p4"), "so does one a newer Bifurc saved");
    // Rules are ordered by name, so "Mapped" comes first and is the one open.
    assert!(ui.has("rule-pattern"), "the first rule's editor is open");
    assert!(ui.has("rule-mapping"), "Mapped sends to a mapping");
    assert!(!ui.has("rule-external"));
}

#[gpui_kit::test]
fn selecting_a_rule_shows_its_own_target(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::ProxyRules);
    ui.click("rule-p1");
    assert!(ui.has("rule-external"), "Rewrite sends to an external address");
    assert!(!ui.has("rule-mapping"));
    ui.press("ctrl-tab");
    assert!(ui.has("rule-mapping"), "Ctrl+Tab steps on, wrapping round to Mapped");
    ui.press("ctrl-shift-tab");
    assert!(ui.has("rule-external"), "Ctrl+Shift+Tab steps back");
}

#[gpui_kit::test]
fn the_list_switch_turns_a_rule_on_through_the_engine(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::ProxyRules);
    ui.fake.forget_calls();
    ui.click("rule-enabled-p2");
    assert_eq!(
        ui.fake.sent("entity.setEnabled"),
        [json!({"workspaceId": "w1", "kind": "rules", "id": "p2", "enabled": true})]
    );
}

#[gpui_kit::test]
fn an_edit_of_a_saved_rule_is_written_after_the_pause(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::ProxyRules);
    ui.fake.forget_calls();
    ui.click("rule-regex");
    assert!(ui.fake.sent("entity.update").is_empty(), "not before the pause");
    ui.advance(Duration::from_millis(700));
    let updates = ui.fake.sent("entity.update");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["kind"], "proxyRules");
    assert_eq!(updates[0]["entity"]["id"], "p2");
    assert_eq!(updates[0]["entity"]["useRegex"], true);
    assert_eq!(updates[0]["entity"]["pattern"], "web.test");
}

#[gpui_kit::test]
fn a_new_rule_is_a_draft_that_needs_a_pattern_before_it_is_created(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::ProxyRules);
    ui.fake.reply_with("entity.create", |payload| {
        let mut entity = payload["entity"].clone();
        entity["id"] = json!("p9");
        Ok(json!({ "entity": entity }))
    });
    ui.click("new-rule");
    assert!(ui.has("rule-draft"));
    ui.fake.forget_calls();
    ui.click("entity-save");
    assert!(ui.fake.sent("entity.create").is_empty(), "no pattern, no rule");
    ui.click("rule-pattern");
    ui.input("shop.test");
    ui.click("rule-external");
    ui.input("http://localhost:9100");
    ui.click("entity-save");
    let created = ui.fake.sent("entity.create");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["kind"], "proxyRules");
    assert_eq!(created[0]["entity"]["pattern"], "shop.test");
    assert_eq!(created[0]["entity"]["targetExternal"], "http://localhost:9100");
    assert_eq!(created[0]["entity"]["targetType"], "external");
}

#[gpui_kit::test]
fn deleting_the_open_rule_asks_first(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::ProxyRules);
    ui.fake.forget_calls();
    ui.click("entity-delete");
    assert!(ui.fake.sent("entity.delete").is_empty());
    ui.press("enter");
    assert_eq!(ui.fake.sent("entity.delete"), [json!({"kind": "proxyRules", "id": "p2"})]);
}

#[gpui_kit::test]
fn a_locked_rule_explains_that_bifurc_must_be_updated(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::ProxyRules);
    ui.click("rule-locked-p4");
    assert!(ui.has("needs-update-settings"));
}

#[gpui_kit::test]
fn a_deleted_rule_is_brought_back_or_deleted_for_good(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::ProxyRules);
    ui.click("rule-deleted-p3");
    ui.fake.forget_calls();
    ui.click("deleted-confirm");
    assert_eq!(ui.fake.sent("entity.confirmDelete"), [json!({"workspaceId": "w1", "kind": "proxyRules", "id": "p3"})]);
}
