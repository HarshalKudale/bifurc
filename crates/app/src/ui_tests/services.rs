//! Services (local listeners found by the engine) and the health bar (URLs checked on a schedule).

use serde_json::json;

use super::{open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn services_without_any_listener_show_the_empty_state(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Services);
    assert!(ui.has("empty-state"));
    assert!(!ui.has(("service", 3000usize)));
}

#[gpui_kit::test]
fn services_list_each_listening_port_and_offer_a_mapping_for_unmapped_ones(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    assert!(ui.has(("service", 3000usize)) && ui.has(("service", 5432usize)));
    assert!(!ui.has("empty-state"));
    // 3000 is already the target of the `api.localhost` mapping.
    assert!(!ui.has(("map-service", 3000usize)));
    assert!(ui.has(("map-service", 5432usize)));
}

#[gpui_kit::test]
fn refresh_asks_the_engine_for_the_listeners_again(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    ui.fake.forget_calls();
    ui.click("refresh-services");
    assert_eq!(ui.fake.sent("services.discover"), [json!({})]);
}

#[gpui_kit::test]
fn mapping_a_service_creates_a_mapping_to_its_port(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    ui.click(("map-service", 5432usize));
    ui.fake.forget_calls();
    ui.answer_prompt(&["db", "localhost:5432"]);
    let created = ui.fake.sent("entity.create");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["kind"], "mappings");
    assert_eq!(created[0]["workspaceId"], "w1");
    assert_eq!(created[0]["entity"]["domain"], "db.localhost");
    assert_eq!(created[0]["entity"]["target"], "localhost:5432");
}

#[gpui_kit::test]
fn an_empty_health_bar_offers_to_add_a_check(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::HealthBar);
    assert!(ui.has("empty-state"));
    assert!(ui.has("health-empty-new"));
    assert!(!ui.has("health-row-h1"));
}

#[gpui_kit::test]
fn the_health_bar_lists_and_checks_every_url_when_first_shown(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::HealthBar);
    assert!(ui.has("health-row-h1") && ui.has("health-row-h2"));
    assert!(!ui.has("empty-state"));
    let mut urls: Vec<_> =
        ui.fake.sent("healthbar.checkUrl").iter().map(|c| c["url"].as_str().unwrap().to_owned()).collect();
    urls.sort();
    assert_eq!(urls, ["http://localhost:3000/health", "http://localhost:5432"]);
}

#[gpui_kit::test]
fn checking_one_url_by_hand_sends_that_check_as_manual(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::HealthBar);
    ui.fake.forget_calls();
    ui.click("health-check-h2");
    assert_eq!(ui.fake.sent("healthbar.checkUrl"), [json!({"url": "http://localhost:5432", "manual": true})]);
    ui.fake.forget_calls();
    ui.click("health-check-all");
    assert_eq!(ui.fake.sent("healthbar.checkUrl").len(), 2);
}

#[gpui_kit::test]
fn switching_auto_refresh_stores_the_whole_list(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::HealthBar);
    ui.fake.forget_calls();
    ui.click("health-auto-h1");
    let stored = ui.fake.sent("healthbar.saveServices");
    assert_eq!(stored.len(), 1);
    let services = stored[0]["services"].as_array().unwrap();
    assert_eq!(services.len(), 2);
    let api = services.iter().find(|s| s["id"] == "h1").unwrap();
    assert_eq!(api["autoRefreshEnabled"], true);
}

#[gpui_kit::test]
fn a_new_health_check_is_added_to_the_list_and_checked(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::HealthBar);
    ui.click("health-new");
    ui.fake.forget_calls();
    ui.answer_prompt(&["Cache", "http://localhost:6379"]);
    let stored = ui.fake.sent("healthbar.saveServices");
    assert_eq!(stored.len(), 1);
    let services = stored[0]["services"].as_array().unwrap();
    assert_eq!(services.len(), 3);
    let added = services.last().unwrap();
    assert_eq!((added["name"].as_str(), added["url"].as_str()), (Some("Cache"), Some("http://localhost:6379")));
    assert!(ui.fake.sent("healthbar.checkUrl").iter().any(|c| c["url"] == "http://localhost:6379"));
}

#[gpui_kit::test]
fn selecting_a_row_shows_its_response_and_the_close_button_hides_it(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::HealthBar);
    assert!(!ui.has("health-close-detail"));
    ui.click("health-row-h1");
    assert!(ui.has("health-close-detail"));
    ui.click("health-close-detail");
    assert!(!ui.has("health-close-detail"));
}
