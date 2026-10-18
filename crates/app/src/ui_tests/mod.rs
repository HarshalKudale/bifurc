//! UI integration tests: the real shell and screens in a headless window, over a fake engine.

mod audit;
mod capture;
mod cassettes;
mod environments;
mod fixtures;
mod flows;
mod faults_policy;
mod mail;
mod mappings;
mod mocks;
mod onboarding;
mod proxy_rules;
mod requests;
mod services;
mod settings;
mod shell;
mod vaults;
mod webhooks;
mod workspace;

use gpui_kit::TestAppContext;

use crate::test_support::Ui;

/// The window over a workspace with nothing in it.
fn open_empty(cx: &mut TestAppContext) -> Ui {
    Ui::open(cx, |_| {})
}

/// The window over [`fixtures::config`] and the data behind every list screen.
fn open_populated(cx: &mut TestAppContext) -> Ui {
    Ui::open(cx, fixtures::populated)
}
