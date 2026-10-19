//! The workspace's own configuration, in pages: General (0-0), Sync (0-1), Cookies (0-2), Import and export
//! (0-3), the agent policy (0-4) and the danger zone (0-5).

use serde_json::json;

use super::{Ui, open_empty, open_populated};
use crate::shell::Screen;

#[gpui_kit::test]
fn the_screen_reads_the_workspaces_sync_state_and_cookies(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::WorkspaceConfig);
    assert_eq!(ui.fake.sent("sync.getState"), [json!({"workspaceId": "w1"})]);
    assert_eq!(ui.fake.sent("cookies.list"), [json!({"workspaceId": "w1"})]);
    assert!(ui.has("rename-workspace"));
}

#[gpui_kit::test]
fn an_unpublished_workspace_offers_to_publish(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_empty(cx);
    ui.show(Screen::WorkspaceConfig);
    ui.click("0-1");
    assert!(ui.has("publish-workspace"));
    assert!(!ui.has("check-git"), "git is installed");
    assert!(!ui.has("pull-remote") && !ui.has("push-remote"), "there is no remote to pull from or push to");
}

#[gpui_kit::test]
fn a_published_workspace_can_pull_and_push(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = Ui::open(cx, |fake| {
        fake.config(json!({"workspaces": [{
            "id": "w1", "name": "Work",
            "syncConfig": {"remote": "https://github.com/me/work.git", "repo": "me/work", "access": "owner"},
        }]}));
    });
    ui.show(Screen::WorkspaceConfig);
    ui.click("0-1");
    assert!(!ui.has("publish-workspace"));
    ui.fake.forget_calls();
    ui.click("pull-remote");
    assert_eq!(ui.fake.sent("sync.pull").len(), 1);
    ui.click("push-remote");
    assert_eq!(ui.fake.sent("sync.push").len(), 1);
}

#[gpui_kit::test]
fn without_git_the_screen_says_so_and_can_check_again(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = Ui::open(cx, |fake| {
        fake.reply("setup.getState", json!({"gitInstalled": false, "onboarded": true}));
    });
    ui.show(Screen::WorkspaceConfig);
    ui.click("0-1");
    assert!(ui.has("check-git"));
    assert!(!ui.has("publish-workspace"));
    ui.fake.forget_calls();
    ui.click("check-git");
    assert_eq!(ui.fake.sent("setup.getState"), [json!({})]);
}

#[gpui_kit::test]
fn clearing_cookies_is_an_engine_command_for_the_workspace(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.fake.reply(
        "cookies.list",
        json!({"clearOnRestart": true, "domains": [
            {"domain": "api.test", "cookies": [{"name": "sid", "value": "1", "path": "/"}]},
        ]}),
    );
    ui.show(Screen::WorkspaceConfig);
    ui.fake.emit("event.cookies.changed", json!({}));
    ui.settle();
    ui.click("0-2");
    ui.fake.forget_calls();
    ui.click("clear-all-cookies");
    assert_eq!(ui.fake.sent("cookies.clear"), [json!({"workspaceId": "w1"})]);
    ui.fake.forget_calls();
    ui.click(("clear-cookies", 0usize));
    assert_eq!(ui.fake.sent("cookies.clear"), [json!({"workspaceId": "w1", "domain": "api.test"})]);
}

#[gpui_kit::test]
fn the_last_workspace_cannot_be_deleted(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = open_populated(cx);
    ui.show(Screen::WorkspaceConfig);
    ui.click("0-5");
    ui.fake.forget_calls();
    ui.click("delete-workspace");
    ui.press("enter");
    assert!(ui.fake.sent("workspace.delete").is_empty(), "a single workspace stays");
}

#[gpui_kit::test]
fn another_workspace_is_deleted_after_asking(cx: &mut gpui_kit::TestAppContext) {
    let mut ui = Ui::open(cx, |fake| {
        fake.config(json!({"workspaces": [{"id": "w1", "name": "Work"}, {"id": "w2", "name": "Home"}]}));
    });
    ui.show(Screen::WorkspaceConfig);
    ui.click("0-5");
    ui.click("delete-workspace");
    assert!(ui.fake.sent("workspace.delete").is_empty(), "asked first");
    ui.press("enter");
    assert_eq!(ui.fake.sent("workspace.delete"), [json!({"id": "w1"})]);
}
