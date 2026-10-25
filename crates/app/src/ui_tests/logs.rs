//! The logs dock: a tab for each dev server the app starts, with its output.

use serde_json::{Value, json};

use super::{open_empty, open_populated};
use crate::{shell::Screen, test_support::Ui};

fn output(workspace: &str, name: &str, lines: &[(&str, &str)]) -> Value {
    let lines: Vec<_> = lines.iter().map(|(stream, text)| json!({"ts": 5, "stream": stream, "text": text})).collect();
    json!({"workspaceId": workspace, "name": name, "lines": lines})
}

fn changed(name: &str, service: Value) -> Value {
    json!({"workspaceId": "w1", "name": name, "service": service})
}

/// The Services screen over `api` running with these lines (so its tab is there from the start), its row
/// clicked so the dock shows it.
fn open_with_log(cx: &mut gpui_kit::TestAppContext, lines: &[(&str, &str)]) -> Ui {
    let mut ui = Ui::open(cx, |fake| {
        super::fixtures::populated(fake);
        fake.reply("devservice.logs", output("w1", "api", lines));
    });
    ui.show(Screen::Services);
    assert!(!ui.has("logs-dock"));
    ui.fake.forget_calls();
    ui.click("dev-row-api");
    ui
}

#[gpui_kit::test]
fn the_buttons_of_a_row_do_not_open_the_dock_behind_them(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::Services);
    ui.click("dev-stop-api");
    ui.click("dev-edit-web");
    assert!(!ui.has("logs-dock"));
}

#[gpui_kit::test]
fn the_footer_button_shows_and_hides_the_dock(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    assert!(!ui.has("logs-dock"));
    ui.click("logs-toggle");
    assert!(ui.has("logs-dock") && ui.has("empty-state") && !ui.has("logs-stop"));
    ui.click("logs-hide");
    assert!(!ui.has("logs-dock"));
    ui.click("logs-toggle");
    ui.click("logs-toggle");
    assert!(!ui.has("logs-dock"));
}

#[gpui_kit::test]
fn starting_a_dev_server_opens_its_tab_in_the_dock_with_the_output_so_far(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("devservice.up", json!({"service": {"name": "worker", "cmd": "node worker.js", "state": "running"}}));
    ui.fake.reply("devservice.logs", output("w1", "worker", &[("stdout", "booting"), ("stderr", "Error: boom")]));
    ui.show(Screen::Services);
    assert!(!ui.has("logs-dock"));
    ui.fake.forget_calls();
    ui.click("dev-start-worker");
    assert_eq!(ui.fake.sent("devservice.up"), [json!({"workspaceId": "w1", "name": "worker"})]);
    assert_eq!(ui.fake.sent("devservice.logs"), [json!({"workspaceId": "w1", "name": "worker", "last": 1000})]);
    assert!(ui.has("logs-dock") && ui.has(("logs-line", 0usize)) && ui.has(("logs-line", 1usize)));
    assert!(!ui.has(("logs-line", 2usize)));
}

#[gpui_kit::test]
fn a_running_server_stops_from_its_tab_and_only_then_can_the_tab_close(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    ui.fake.reply("devservice.down", json!({"service": {"name": "api", "cmd": "npm run api", "state": "stopped"}}));

    // No cross on the tab; running, there is nothing to close yet.
    assert!(ui.has("tab-logs-api") && !ui.has("close-tab"));
    ui.click("logs-close");
    assert!(ui.has(("logs-line", 0usize)));

    ui.fake.forget_calls();
    ui.click("logs-stop");
    assert_eq!(ui.fake.sent("devservice.down"), [json!({"workspaceId": "w1", "name": "api"})]);
    // The logs stay to be read after the stop, and stopping again is not possible.
    assert!(ui.has(("logs-line", 0usize)));
    ui.fake.forget_calls();
    ui.click("logs-stop");
    assert!(ui.fake.sent("devservice.down").is_empty());

    ui.click("logs-close");
    assert!(!ui.has(("logs-line", 0usize)) && ui.has("empty-state"));
    assert!(ui.fake.sent("devservice.down").is_empty());
}

#[gpui_kit::test]
fn each_dev_server_gets_a_tab_of_its_own_and_a_row_click_brings_its_tab_back(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    assert!(ui.has(("logs-line", 0usize)));
    ui.fake.reply("devservice.logs", output("w1", "web", &[("stdout", "vite"), ("stdout", "ready in 1s")]));
    ui.click("dev-row-web");
    assert!(ui.has(("logs-line", 1usize)));
    // Back on the first tab, its own lines are there, and clicking its row again opens no second tab.
    ui.click("tab-logs-api");
    assert!(ui.has(("logs-line", 0usize)) && !ui.has(("logs-line", 1usize)));
    ui.fake.forget_calls();
    ui.click("dev-row-api");
    assert!(ui.fake.sent("devservice.logs").is_empty());
    assert!(ui.has(("logs-line", 0usize)) && !ui.has(("logs-line", 1usize)));
}

#[gpui_kit::test]
fn a_window_opened_over_running_servers_gives_them_their_tabs(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = Ui::open(cx, |fake| {
        super::fixtures::populated(fake);
        fake.reply("devservice.logs", output("w1", "api", &[("stdout", "running since before")]));
    });
    ui.click("logs-toggle");
    // `api` runs in the fixture; the stopped and crashed servers get no tab.
    assert!(ui.has("tab-logs-api") && !ui.has("tab-logs-web") && !ui.has("tab-logs-worker"));
    assert!(ui.has(("logs-line", 0usize)));
}

#[gpui_kit::test]
fn a_server_without_output_says_so(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[]);
    assert!(ui.has("logs-dock") && ui.has("empty-state") && !ui.has(("logs-line", 0usize)));
}

#[gpui_kit::test]
fn output_events_reach_the_tab_of_that_server_only(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    ui.fake.emit("event.devservice.output", output("w1", "db", &[("stdout", "not ours")]));
    ui.fake.emit("event.devservice.output", output("w2", "api", &[("stdout", "other workspace")]));
    ui.settle();
    assert!(!ui.has(("logs-line", 1usize)));
    ui.fake.emit("event.devservice.output", output("w1", "api", &[("stdout", "GET /"), ("stderr", "WARN slow")]));
    ui.settle();
    assert!(ui.has(("logs-line", 2usize)) && !ui.has(("logs-line", 3usize)));
}

#[gpui_kit::test]
fn the_search_and_errors_only_narrow_the_lines_shown(cx: &mut gpui_kit::TestAppContext) {
    let lines = [("stdout", "ready"), ("stdout", "GET /users"), ("stderr", "Error: boom"), ("stdout", "GET /orders")];
    let mut ui = open_with_log(cx, &lines);
    assert!(ui.has(("logs-line", 3usize)));
    ui.click("logs-search");
    ui.input("GET");
    assert!(ui.has(("logs-line", 1usize)) && !ui.has(("logs-line", 2usize)));
    ui.click("logs-errors");
    assert!(ui.has("empty-state") && !ui.has(("logs-line", 0usize)), "no error line has GET");
    ui.press("ctrl-a");
    ui.input("boom");
    assert!(ui.has(("logs-line", 0usize)) && !ui.has(("logs-line", 1usize)));
}

#[gpui_kit::test]
fn clearing_the_view_empties_it_without_asking_the_engine_and_new_output_still_arrives(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    ui.fake.forget_calls();
    ui.click("logs-clear");
    assert!(!ui.has(("logs-line", 0usize)));
    assert!(ui.fake.calls().is_empty());
    ui.fake.emit("event.devservice.output", output("w1", "api", &[("stdout", "again")]));
    ui.settle();
    assert!(ui.has(("logs-line", 0usize)) && !ui.has(("logs-line", 1usize)));
}

#[gpui_kit::test]
fn a_tab_follows_how_its_server_is_doing_and_goes_when_the_server_is_removed(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    ui.fake.emit("event.devservice.changed", changed("api", json!({"name": "api", "state": "crashed", "exitCode": 7})));
    ui.settle();
    assert!(ui.has("logs-status"));
    ui.fake.emit("event.devservice.changed", changed("api", Value::Null));
    ui.settle();
    assert!(!ui.has("logs-status") && ui.has("empty-state"));
}

#[gpui_kit::test]
fn a_server_started_again_by_anyone_empties_and_reuses_its_tab(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "old run")]);
    ui.fake.emit("event.devservice.changed", changed("api", json!({"name": "api", "state": "stopped"})));
    ui.settle();
    assert!(ui.has(("logs-line", 0usize)), "a stopped server keeps its logs");

    // The CLI or an agent starts it again: the run before is gone from the engine, and so from the tab.
    ui.fake.reply("devservice.logs", output("w1", "api", &[("stdout", "new run"), ("stdout", "listening")]));
    ui.fake.forget_calls();
    ui.fake.emit("event.devservice.changed", changed("api", json!({"name": "api", "state": "running"})));
    ui.settle();
    assert_eq!(ui.fake.sent("devservice.logs"), [json!({"workspaceId": "w1", "name": "api", "last": 1000})]);
    assert!(ui.has(("logs-line", 1usize)) && !ui.has(("logs-line", 2usize)));
}
