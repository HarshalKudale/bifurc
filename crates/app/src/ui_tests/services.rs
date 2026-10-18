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

#[gpui_kit::test]
fn dev_servers_are_listed_with_the_action_their_state_allows(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    assert!(ui.has("dev-row-api") && ui.has("dev-row-web") && ui.has("dev-row-worker"));
    assert!(ui.has("dev-stop-api") && ui.has("dev-restart-api") && !ui.has("dev-start-api"));
    assert!(ui.has("dev-start-web") && ui.has("dev-start-worker") && !ui.has("dev-stop-web"));
    assert!(!ui.has("dev-empty-add"));
}

#[gpui_kit::test]
fn without_dev_servers_the_section_explains_and_offers_to_add_one(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::Services);
    assert!(ui.has("dev-empty-add") && ui.has("add-dev-server"));
}

#[gpui_kit::test]
fn start_asks_the_engine_to_bring_the_service_up(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    ui.fake.forget_calls();
    ui.click("dev-start-worker");
    assert_eq!(ui.fake.sent("devservice.up"), [json!({"workspaceId": "w1", "name": "worker"})]);
    ui.click("dev-stop-api");
    assert_eq!(ui.fake.sent("devservice.down"), [json!({"workspaceId": "w1", "name": "api"})]);
}

#[gpui_kit::test]
fn a_changed_event_updates_that_row_and_other_workspaces_are_ignored(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    let worker = |workspace: &str, state: &str| json!({"workspaceId": workspace, "name": "worker", "service": {"name": "worker", "cmd": "node worker.js", "state": state, "pid": 5}});
    ui.fake.emit("event.devservice.changed", worker("w2", "running"));
    ui.settle();
    assert!(ui.has("dev-start-worker"));
    ui.fake.emit("event.devservice.changed", worker("w1", "running"));
    ui.settle();
    assert!(ui.has("dev-stop-worker") && !ui.has("dev-start-worker"));
    ui.fake.emit("event.devservice.changed", json!({"workspaceId": "w1", "name": "worker", "service": null}));
    ui.settle();
    assert!(!ui.has("dev-row-worker"));
}

#[gpui_kit::test]
fn editing_a_dev_server_sends_the_whole_definition_with_its_env(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    ui.click("dev-edit-api");
    ui.fake.forget_calls();
    ui.answer_prompt(&["npm run api2", "3001"]);
    let defined = ui.fake.sent("devservice.define");
    assert_eq!(
        defined,
        [json!({"workspaceId": "w1", "name": "api", "cmd": "npm run api2", "port": 3001, "env": {"A": "1"}})]
    );
}

#[gpui_kit::test]
fn adding_a_dev_server_checks_the_name_and_port_before_calling_the_engine(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    ui.click("add-dev-server");
    ui.fake.forget_calls();
    ui.answer_prompt(&["bad name", "npm start"]);
    assert!(ui.fake.sent("devservice.define").is_empty());
    // The dialog opens again with what was typed, so only the name needs fixing.
    ui.answer_prompt(&["good-name"]);
    assert_eq!(
        ui.fake.sent("devservice.define"),
        [json!({"workspaceId": "w1", "name": "good-name", "cmd": "npm start"})]
    );
}

#[gpui_kit::test]
fn removing_asks_first(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    ui.fake.forget_calls();
    ui.click("dev-remove-worker");
    assert!(ui.fake.sent("devservice.remove").is_empty());
    ui.press("enter");
    assert_eq!(ui.fake.sent("devservice.remove"), [json!({"workspaceId": "w1", "name": "worker"})]);
}

#[gpui_kit::test]
fn freeing_a_port_confirms_then_stops_the_holder_and_lists_again(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    ui.fake.forget_calls();
    ui.click(("free-port", 5432usize));
    assert!(ui.fake.sent("devservice.freePort").is_empty());
    ui.press("enter");
    assert_eq!(ui.fake.sent("devservice.freePort"), [json!({"port": 5432, "kill": true})]);
    assert_eq!(ui.fake.sent("services.discover").len(), 1);
}

#[gpui_kit::test]
fn a_service_with_an_unsealed_vault_starts_without_the_app_reading_its_secrets(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    ui.fake.forget_calls();
    ui.click("dev-start-web");
    assert!(ui.fake.sent("vault.secrets").is_empty());
    assert_eq!(ui.fake.sent("devservice.up"), [json!({"workspaceId": "w1", "name": "web"})]);
}

#[gpui_kit::test]
fn a_sealed_vault_asks_for_its_passphrase_before_the_service_starts(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    ui.fake.forget_calls();
    ui.click("dev-start-db");
    assert!(ui.fake.sent("devservice.up").is_empty());
    ui.answer_prompt(&["hunter2"]);
    assert_eq!(ui.fake.sent("vault.unseal"), [json!({"workspaceId": "w1", "id": "vt2", "passphrase": "hunter2"})]);
    assert_eq!(ui.fake.sent("devservice.up"), [json!({"workspaceId": "w1", "name": "db"})]);
}

fn output(workspace: &str, name: &str, lines: &[(&str, &str)]) -> serde_json::Value {
    let lines: Vec<_> = lines.iter().map(|(stream, text)| json!({"ts": 5, "stream": stream, "text": text})).collect();
    json!({"workspaceId": workspace, "name": name, "lines": lines})
}

/// The screen with `api` selected and these lines already in its log.
fn open_with_log(cx: &mut gpui_kit::TestAppContext, lines: &[(&str, &str)]) -> crate::test_support::Ui {
    let mut ui = open_populated(cx);
    ui.fake.reply("devservice.logs", output("w1", "api", lines));
    ui.show(Screen::Services);
    assert!(!ui.has("dev-logs"));
    ui.fake.forget_calls();
    ui.click("dev-row-api");
    ui
}

#[gpui_kit::test]
fn selecting_a_dev_server_loads_its_last_lines_and_selecting_it_again_closes_them(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "ready"), ("stderr", "Error: boom")]);
    assert_eq!(ui.fake.sent("devservice.logs"), [json!({"workspaceId": "w1", "name": "api", "last": 1000})]);
    assert!(ui.has("dev-logs") && ui.has(("dev-log", 0usize)) && ui.has(("dev-log", 1usize)));
    assert!(!ui.has(("dev-log", 2usize)));
    ui.click("dev-row-api");
    assert!(!ui.has("dev-logs"));
}

#[gpui_kit::test]
fn a_server_without_output_says_so(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[]);
    assert!(ui.has("dev-logs") && ui.has("empty-state") && !ui.has(("dev-log", 0usize)));
}

#[gpui_kit::test]
fn output_events_of_the_shown_server_are_appended_and_others_are_ignored(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    ui.fake.emit("event.devservice.output", output("w1", "db", &[("stdout", "not ours")]));
    ui.fake.emit("event.devservice.output", output("w2", "api", &[("stdout", "other workspace")]));
    ui.settle();
    assert!(!ui.has(("dev-log", 1usize)));
    ui.fake.emit("event.devservice.output", output("w1", "api", &[("stdout", "GET /"), ("stderr", "WARN slow")]));
    ui.settle();
    assert!(ui.has(("dev-log", 2usize)) && !ui.has(("dev-log", 3usize)));
}

#[gpui_kit::test]
fn the_search_and_errors_only_narrow_the_lines_shown(cx: &mut gpui_kit::TestAppContext) {
    let lines = [("stdout", "ready"), ("stdout", "GET /users"), ("stderr", "Error: boom"), ("stdout", "GET /orders")];
    let mut ui = open_with_log(cx, &lines);
    assert!(ui.has(("dev-log", 3usize)));
    ui.click("dev-log-search");
    ui.input("GET");
    assert!(ui.has(("dev-log", 1usize)) && !ui.has(("dev-log", 2usize)));
    ui.click("dev-log-errors");
    assert!(ui.has("empty-state") && !ui.has(("dev-log", 0usize)), "no error line has GET");
    ui.press("ctrl-a");
    ui.input("boom");
    assert!(ui.has(("dev-log", 0usize)) && !ui.has(("dev-log", 1usize)));
}

#[gpui_kit::test]
fn clearing_the_view_empties_it_without_asking_the_engine_and_new_output_still_arrives(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    ui.fake.forget_calls();
    ui.click("dev-log-clear");
    assert!(!ui.has(("dev-log", 0usize)));
    assert!(ui.fake.calls().is_empty());
    ui.fake.emit("event.devservice.output", output("w1", "api", &[("stdout", "again")]));
    ui.settle();
    assert!(ui.has(("dev-log", 0usize)) && !ui.has(("dev-log", 1usize)));
}

#[gpui_kit::test]
fn removing_the_shown_server_closes_its_output(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    ui.fake.emit("event.devservice.changed", json!({"workspaceId": "w1", "name": "api", "service": null}));
    ui.settle();
    assert!(!ui.has("dev-logs"));
}
