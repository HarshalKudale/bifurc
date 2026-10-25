//! The terminal dock: a tab for each dev server the app starts and for each terminal the user opens.

use serde_json::{Value, json};

use super::{open_empty, open_populated};
use crate::{shell::Screen, test_support::Ui};

fn output(workspace: &str, name: &str, lines: &[(&str, &str)]) -> Value {
    let lines: Vec<_> = lines.iter().map(|(stream, text)| json!({"ts": 5, "stream": stream, "text": text})).collect();
    json!({"workspaceId": workspace, "name": name, "lines": lines})
}

fn terminal_output(id: &str, lines: &[(&str, &str)]) -> Value {
    let lines: Vec<_> = lines.iter().map(|(stream, text)| json!({"ts": 5, "stream": stream, "text": text})).collect();
    json!({"id": id, "lines": lines})
}

fn terminal(id: &str, state: &str) -> Value {
    json!({"id": id, "title": "npm test", "cmd": "npm test", "cwd": "/home", "state": state, "pid": 9})
}

/// The Services screen with `api` already printing these lines, its row clicked so the dock shows it.
fn open_with_log(cx: &mut gpui_kit::TestAppContext, lines: &[(&str, &str)]) -> Ui {
    let mut ui = open_populated(cx);
    ui.fake.reply("devservice.logs", output("w1", "api", lines));
    ui.show(Screen::Services);
    assert!(!ui.has("terminal-dock"));
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
    assert!(!ui.has("terminal-dock"));
}

#[gpui_kit::test]
fn the_footer_button_shows_and_hides_the_dock(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    assert!(!ui.has("terminal-dock"));
    ui.click("terminal-toggle");
    assert!(ui.has("terminal-dock") && ui.has("empty-state") && ui.has("terminal-new"));
    ui.click("terminal-hide");
    assert!(!ui.has("terminal-dock"));
    ui.click("terminal-toggle");
    ui.click("terminal-toggle");
    assert!(!ui.has("terminal-dock"));
}

#[gpui_kit::test]
fn starting_a_dev_server_opens_its_tab_in_the_dock_with_the_output_so_far(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply("devservice.up", json!({"service": {"name": "worker", "cmd": "node worker.js", "state": "running"}}));
    ui.fake.reply("devservice.logs", output("w1", "worker", &[("stdout", "booting"), ("stderr", "Error: boom")]));
    ui.show(Screen::Services);
    assert!(!ui.has("terminal-dock"));
    ui.fake.forget_calls();
    ui.click("dev-start-worker");
    assert_eq!(ui.fake.sent("devservice.up"), [json!({"workspaceId": "w1", "name": "worker"})]);
    assert_eq!(ui.fake.sent("devservice.logs"), [json!({"workspaceId": "w1", "name": "worker", "last": 1000})]);
    assert!(ui.has("terminal-dock") && ui.has("terminal-tab-dev-worker"));
    assert!(
        ui.has(("terminal-line", 0usize)) && ui.has(("terminal-line", 1usize)) && !ui.has(("terminal-line", 2usize))
    );
    // The server is not stopped with the tab: closing it only stops following.
    ui.fake.forget_calls();
    ui.click("terminal-close-dev-worker");
    assert!(!ui.has("terminal-tab-dev-worker"));
    assert!(ui.fake.sent("devservice.down").is_empty() && ui.fake.sent("terminal.close").is_empty());
}

#[gpui_kit::test]
fn each_dev_server_gets_a_tab_of_its_own_and_a_row_click_brings_its_tab_back(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    assert_eq!(ui.fake.sent("devservice.logs"), [json!({"workspaceId": "w1", "name": "api", "last": 1000})]);
    assert!(ui.has("terminal-tab-dev-api") && ui.has(("terminal-line", 0usize)));
    ui.fake.reply("devservice.logs", output("w1", "web", &[("stdout", "vite"), ("stdout", "ready in 1s")]));
    ui.click("dev-row-web");
    assert!(ui.has("terminal-tab-dev-api") && ui.has("terminal-tab-dev-web"));
    assert!(ui.has(("terminal-line", 1usize)));
    // Back on the first tab, its own lines are there, and clicking its row again opens no second tab.
    ui.click("terminal-tab-dev-api");
    assert!(ui.has(("terminal-line", 0usize)) && !ui.has(("terminal-line", 1usize)));
    ui.click("dev-row-api");
    assert_eq!(ui.count("terminal-tab-dev-api"), 1);
}

#[gpui_kit::test]
fn a_server_without_output_says_so(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[]);
    assert!(ui.has("terminal-dock") && ui.has("empty-state") && !ui.has(("terminal-line", 0usize)));
}

#[gpui_kit::test]
fn output_events_reach_the_tab_of_that_server_only(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    ui.fake.emit("event.devservice.output", output("w1", "db", &[("stdout", "not ours")]));
    ui.fake.emit("event.devservice.output", output("w2", "api", &[("stdout", "other workspace")]));
    ui.settle();
    assert!(!ui.has(("terminal-line", 1usize)));
    ui.fake.emit("event.devservice.output", output("w1", "api", &[("stdout", "GET /"), ("stderr", "WARN slow")]));
    ui.settle();
    assert!(ui.has(("terminal-line", 2usize)) && !ui.has(("terminal-line", 3usize)));
}

#[gpui_kit::test]
fn the_search_and_errors_only_narrow_the_lines_shown(cx: &mut gpui_kit::TestAppContext) {
    let lines = [("stdout", "ready"), ("stdout", "GET /users"), ("stderr", "Error: boom"), ("stdout", "GET /orders")];
    let mut ui = open_with_log(cx, &lines);
    assert!(ui.has(("terminal-line", 3usize)));
    ui.click("terminal-search");
    ui.input("GET");
    assert!(ui.has(("terminal-line", 1usize)) && !ui.has(("terminal-line", 2usize)));
    ui.click("terminal-errors");
    assert!(ui.has("empty-state") && !ui.has(("terminal-line", 0usize)), "no error line has GET");
    ui.press("ctrl-a");
    ui.input("boom");
    assert!(ui.has(("terminal-line", 0usize)) && !ui.has(("terminal-line", 1usize)));
}

#[gpui_kit::test]
fn clearing_the_view_empties_it_without_asking_the_engine_and_new_output_still_arrives(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    ui.fake.forget_calls();
    ui.click("terminal-clear");
    assert!(!ui.has(("terminal-line", 0usize)));
    assert!(ui.fake.calls().is_empty());
    ui.fake.emit("event.devservice.output", output("w1", "api", &[("stdout", "again")]));
    ui.settle();
    assert!(ui.has(("terminal-line", 0usize)) && !ui.has(("terminal-line", 1usize)));
}

#[gpui_kit::test]
fn a_tab_follows_how_its_server_is_doing_and_goes_when_the_server_is_removed(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_with_log(cx, &[("stdout", "ready")]);
    let changed = |service: Value| json!({"workspaceId": "w1", "name": "api", "service": service});
    ui.fake.emit("event.devservice.changed", changed(json!({"name": "api", "state": "crashed", "exitCode": 7})));
    ui.settle();
    assert!(ui.has("terminal-status"));
    ui.fake.emit("event.devservice.changed", changed(Value::Null));
    ui.settle();
    assert!(!ui.has("terminal-tab-dev-api"));
}

#[gpui_kit::test]
fn a_terminal_of_your_own_runs_a_command_and_takes_lines_while_it_runs(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.fake.reply("terminal.open", json!({"terminal": terminal("t1", "running")}));
    ui.fake.reply("terminal.logs", terminal_output("t1", &[("stdout", "> npm test")]));
    ui.click("terminal-toggle");
    ui.click("terminal-new");
    ui.fake.forget_calls();
    ui.answer_prompt(&["npm test", ""]);
    assert_eq!(ui.fake.sent("terminal.open"), [json!({"cmd": "npm test"})]);
    assert!(ui.has("terminal-tab-term-t1") && ui.has(("terminal-line", 0usize)));
    assert!(ui.has("terminal-input"));

    ui.fake.emit("event.terminal.output", terminal_output("t1", &[("stdout", "3 passed")]));
    ui.settle();
    assert!(ui.has(("terminal-line", 1usize)));

    ui.fake.forget_calls();
    ui.click("terminal-input");
    ui.input("rs");
    ui.press("enter");
    assert_eq!(ui.fake.sent("terminal.input"), [json!({"id": "t1", "text": "rs"})]);

    // Once it ends there is nothing to type to.
    ui.fake.emit("event.terminal.changed", json!({"id": "t1", "terminal": terminal("t1", "exited")}));
    ui.settle();
    assert!(!ui.has("terminal-input") && ui.has("terminal-tab-term-t1"));

    ui.fake.forget_calls();
    ui.click("terminal-close-term-t1");
    assert_eq!(ui.fake.sent("terminal.close"), [json!({"id": "t1"})]);
    assert!(!ui.has("terminal-tab-term-t1"));
}

#[gpui_kit::test]
fn a_terminal_without_a_command_is_the_shell(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.click("terminal-toggle");
    ui.click("terminal-new");
    ui.fake.forget_calls();
    ui.answer_prompt(&["", "/work"]);
    assert_eq!(ui.fake.sent("terminal.open"), [json!({"cwd": "/work"})]);
}

#[gpui_kit::test]
fn terminals_the_engine_already_has_get_their_tabs_back(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = crate::test_support::Ui::open(cx, |fake| {
        super::fixtures::populated(fake);
        fake.reply("terminal.list", json!({"terminals": [terminal("t7", "running")]}));
        fake.reply("terminal.logs", terminal_output("t7", &[("stdout", "hello")]));
    });
    ui.click("terminal-toggle");
    assert!(ui.has("terminal-tab-term-t7") && ui.has(("terminal-line", 0usize)));
}
