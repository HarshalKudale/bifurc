//! What agents have started that the user should see in the footer: fault rules, cassette recording and
//! replay, and the approvals waiting for a decision, with the dialog that decides them.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};

use super::Shell;
use crate::{
    app_model::AppModel,
    i18n::{t, t_with},
};

impl Shell {
    /// The footer's indicators; each is there only while what it reports is.
    pub(super) fn render_agent_badges(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let model = self.model.read(cx);
        let faults: Vec<_> = model.faults().iter().map(|rule| (rule.id.clone(), rule.description.clone())).collect();
        let recording = model.recording().cloned();
        let replaying = model.replaying().cloned();
        let approvals = model.approvals().len();
        let mut badges = Vec::new();

        if approvals > 0 {
            badges.push(
                Button::new("approvals-open")
                    .warning()
                    .xsmall()
                    .label(t_with(
                        if approvals == 1 { "shell.agents.approvals_one" } else { "shell.agents.approvals_other" },
                        &[("count", &approvals)],
                    ))
                    .tooltip(t("shell.agents.approvals_tooltip"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_approvals(window, cx)))
                    .into_any_element(),
            );
        }
        if !faults.is_empty() {
            let model = self.model.clone();
            badges.push(
                Button::new("faults-menu")
                    .warning()
                    .outline()
                    .xsmall()
                    .label(t_with("shell.agents.faults", &[("count", &faults.len())]))
                    .tooltip(t("shell.agents.faults_tooltip"))
                    .dropdown_menu(move |menu, _, _| {
                        let menu = faults.iter().fold(menu, |menu, (id, description)| {
                            let (model, id) = (model.clone(), id.clone());
                            menu.item(
                                PopupMenuItem::new(t_with("shell.agents.clear_fault", &[("fault", description)]))
                                    .on_click(move |_, _, cx| model.update(cx, |m, cx| m.clear_faults(Some(&id), cx))),
                            )
                        });
                        let model = model.clone();
                        menu.separator().item(
                            PopupMenuItem::new(t("shell.agents.clear_all_faults"))
                                .on_click(move |_, _, cx| model.update(cx, |m, cx| m.clear_faults(None, cx))),
                        )
                    })
                    .into_any_element(),
            );
        }
        if let Some(recording) = recording {
            let model = self.model.clone();
            badges.push(
                Button::new("recording-menu")
                    .ghost()
                    .xsmall()
                    .label(t_with(
                        "shell.agents.recording",
                        &[("name", &recording.name), ("count", &recording.interactions)],
                    ))
                    .dropdown_menu(move |menu, _, _| {
                        let model = model.clone();
                        menu.item(
                            PopupMenuItem::new(t("shell.agents.stop_recording"))
                                .on_click(move |_, _, cx| model.update(cx, |m, cx| m.stop_recording(cx))),
                        )
                    })
                    .into_any_element(),
            );
        }
        if let Some(replay) = replaying {
            let model = self.model.clone();
            let button = Button::new("replay-menu").xsmall();
            // Requests no recording answers fail while it replays, so they are worth a second look.
            let button = if replay.unmatched > 0 {
                button.danger().outline().label(t_with(
                    "shell.agents.replaying_unmatched",
                    &[("name", &replay.name), ("count", &replay.unmatched)],
                ))
            } else {
                button.ghost().label(t_with("shell.agents.replaying", &[("name", &replay.name)]))
            };
            badges.push(
                button
                    .dropdown_menu(move |menu, _, _| {
                        let model = model.clone();
                        menu.item(
                            PopupMenuItem::new(t("shell.agents.stop_replay"))
                                .on_click(move |_, _, cx| model.update(cx, |m, cx| m.stop_replay(cx))),
                        )
                    })
                    .into_any_element(),
            );
        }
        badges
    }

    pub(super) fn open_approvals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let list = cx.new(|cx| ApprovalList::new(self.model.clone(), cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title(t("shell.agents.approvals_title")).w_128().child(list.clone())
        });
    }
}

/// The pending approvals, drawn from the model so that arrivals and decisions made elsewhere (the CLI, an
/// expiry) show as they happen.
struct ApprovalList {
    model: Entity<AppModel>,
    _subscription: Subscription,
}

impl ApprovalList {
    fn new(model: Entity<AppModel>, cx: &mut Context<Self>) -> Self {
        let _subscription = cx.observe(&model, |_, _, cx| cx.notify());
        Self { model, _subscription }
    }
}

impl Render for ApprovalList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        let approvals = self.model.read(cx).approvals().to_vec();
        if approvals.is_empty() {
            return div()
                .text_color(theme.muted_foreground)
                .child(t("shell.agents.approvals_empty"))
                .into_any_element();
        }
        v_flex()
            .id("approvals-list")
            .max_h_96()
            .overflow_y_scroll()
            .gap_3()
            .children(approvals.into_iter().map(|item| {
                let decide = |approve: bool| {
                    let (model, id) = (self.model.clone(), item.id.clone());
                    move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
                        model.update(cx, |m, cx| m.decide_approval(&id, approve, cx))
                    }
                };
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().font_semibold().child(item.by.clone()))
                            .child(div().child(format!("{} {}", item.method, item.url)))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(t_with("shell.agents.policy", &[("policy", &item.policy)])),
                            )
                            .child(div().text_sm().text_color(theme.muted_foreground).child(item.timing(now))),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new(format!("deny-{}", item.id))
                                    .danger()
                                    .outline()
                                    .small()
                                    .label(t("shell.agents.deny"))
                                    .on_click(decide(false)),
                            )
                            .child(
                                Button::new(format!("approve-{}", item.id))
                                    .primary()
                                    .small()
                                    .label(t("shell.agents.approve"))
                                    .on_click(decide(true)),
                            ),
                    )
            }))
            .into_any_element()
    }
}
