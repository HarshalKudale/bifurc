//! Record and replay (cassettes) in the Capture toolbar. Whether one is active comes from the model, which the
//! engine keeps current through `event.cassette.changed`; the buttons here only send the commands.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::DialogFooter,
    h_flex, v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use serde_json::{Value, json};

use super::CaptureScreen;
use crate::{
    components::dialogs::{PromptField, prompt},
    engine::{call, call_in, failure},
    i18n::{t, t_with},
};

/// The cassette chosen in the replay dialog.
struct Pick {
    names: Vec<(String, String)>,
    picked: Option<usize>,
    passthrough: bool,
}

/// The `unmatched` lines of a stopped replay, like the CLI's `✗` lines, capped.
fn unmatched_lines(stopped: &Value) -> Vec<String> {
    const SHOWN: usize = 5;
    let list = stopped["unmatched"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut lines: Vec<String> = list
        .iter()
        .take(SHOWN)
        .map(|u| format!("✗ {} {}", u["method"].as_str().unwrap_or(""), u["url"].as_str().unwrap_or("")))
        .collect();
    if list.len() > SHOWN {
        lines.push(t_with("cassettes.more", &[("count", &(list.len() - SHOWN))]));
    }
    lines
}

fn warnings(started: &Value) -> Vec<String> {
    started["warnings"].as_array().into_iter().flatten().filter_map(|w| w.as_str().map(str::to_owned)).collect()
}

impl CaptureScreen {
    pub(super) fn render_cassette_buttons(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let (recording, replaying) = {
            let model = self.model.read(cx);
            (model.recording().cloned(), model.replaying().cloned())
        };
        let busy = recording.is_some() || replaying.is_some();
        let record = match &recording {
            Some(r) => Button::new("capture-record")
                .small()
                .danger()
                .label(t_with("cassettes.stop_recording", &[("count", &r.interactions)]))
                .on_click(cx.listener(|this, _, _, cx| this.stop_recording(cx))),
            None => Button::new("capture-record")
                .small()
                .outline()
                .label(t("cassettes.record"))
                .tooltip(t("cassettes.record_tooltip"))
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.ask_record(window, cx))),
        };
        let replay = match &replaying {
            Some(r) => Button::new("capture-replay")
                .small()
                .danger()
                .label(if r.unmatched > 0 {
                    t_with("cassettes.stop_replay_unmatched", &[("count", &r.unmatched)])
                } else {
                    t("cassettes.stop_replay").to_owned()
                })
                .on_click(cx.listener(|this, _, _, cx| this.stop_replay(cx))),
            None => Button::new("capture-replay")
                .small()
                .outline()
                .label(t("cassettes.replay"))
                .tooltip(t("cassettes.replay_tooltip"))
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.ask_replay(window, cx))),
        };
        h_flex().gap_2().child(record).child(replay).into_any_element()
    }

    fn ask_record(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("cassettes.record_title"),
            t("cassettes.record"),
            vec![
                PromptField::new(t("cassettes.field.name"), t("cassettes.field.name_placeholder")),
                PromptField::new(t("cassettes.field.hosts"), t("cassettes.field.hosts_placeholder")),
            ],
            |this, values, _, cx| {
                let hosts: Vec<&str> = values[1].split([',', ' ']).filter(|h| !h.is_empty()).collect();
                this.record_start(&values[0], hosts, cx);
            },
        );
    }

    fn record_start(&mut self, name: &str, hosts: Vec<&str>, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "name": name, "hosts": hosts });
        call(&client, cx, "cassette.recordStart", payload, |this, outcome, cx| match failure(&outcome) {
            Some(message) => this.report(t("cassettes.error.record"), message, cx),
            None => this.show_warnings(&outcome.unwrap_or_default(), cx),
        });
    }

    fn show_warnings(&mut self, started: &Value, cx: &mut Context<Self>) {
        for warning in warnings(started) {
            self.model.update(cx, |model, cx| model.notice(warning, cx));
        }
    }

    fn stop_recording(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "cassette.recordStop", json!({}), |this, outcome, cx| match failure(&outcome) {
            Some(message) => this.report(t("cassettes.error.stop_record"), message, cx),
            None => {
                let stopped = outcome.unwrap_or_default();
                let text = t_with(
                    "cassettes.recorded",
                    &[
                        ("name", &stopped["name"].as_str().unwrap_or("")),
                        ("count", &stopped["interactions"].as_u64().unwrap_or(0)),
                    ],
                );
                this.model.update(cx, |model, cx| model.notice(text, cx));
            }
        });
    }

    fn stop_replay(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "cassette.replayStop", json!({}), |this, outcome, cx| match failure(&outcome) {
            Some(message) => this.report(t("cassettes.error.stop_replay"), message, cx),
            None => {
                let stopped = outcome.unwrap_or_default();
                let mut text = t_with(
                    "cassettes.replayed",
                    &[
                        ("name", &stopped["name"].as_str().unwrap_or("")),
                        ("served", &stopped["served"].as_u64().unwrap_or(0)),
                        ("unmatched", &stopped["unmatchedCount"].as_u64().unwrap_or(0)),
                    ],
                );
                for line in unmatched_lines(&stopped) {
                    text.push('\n');
                    text.push_str(&line);
                }
                this.model.update(cx, |model, cx| model.notice(text, cx));
            }
        });
    }

    /// Lists the workspace's cassettes, then asks which to replay.
    fn ask_replay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id() });
        call_in(&client, window, cx, "cassette.list", payload, |this, outcome, window, cx| {
            if let Some(message) = failure(&outcome) {
                return this.report(t("cassettes.error.list"), message, cx);
            }
            let listed = outcome.unwrap_or_default();
            let names: Vec<(String, String)> = listed["cassettes"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .map(|c| {
                    let name = c["name"].as_str().unwrap_or("").to_owned();
                    let count = c["interactions"].as_u64().unwrap_or(0);
                    (name, t_with("cassettes.interactions", &[("count", &count)]))
                })
                .collect();
            let pick = cx.new(|_| Pick { names, picked: None, passthrough: false });
            this.open_replay_dialog(pick, window, cx);
        });
    }

    fn open_replay_dialog(&mut self, pick: Entity<Pick>, window: &mut Window, cx: &mut Context<Self>) {
        let screen = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            window.open_dialog(cx, move |dialog, _, cx| {
                let muted = cx.theme().muted_foreground;
                let (names, picked, passthrough) = {
                    let p = pick.read(cx);
                    (p.names.clone(), p.picked, p.passthrough)
                };
                let rows = if names.is_empty() {
                    div().text_sm().text_color(muted).child(t("cassettes.none")).into_any_element()
                } else {
                    v_flex()
                        .id("cassette-list")
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .gap_1()
                        .children(names.iter().enumerate().map(|(ix, (name, detail))| {
                            let pick = pick.clone();
                            Button::new(("cassette", ix))
                                .ghost()
                                .small()
                                .w_full()
                                .selected(picked == Some(ix))
                                .label(format!("{name}  ·  {detail}"))
                                .on_click(move |_, window, cx| {
                                    pick.update(cx, |p, cx| {
                                        p.picked = Some(ix);
                                        cx.notify();
                                    });
                                    window.refresh();
                                })
                        }))
                        .into_any_element()
                };
                let chosen = picked.and_then(|ix| names.get(ix)).map(|n| n.0.clone());
                let (screen, pick_for_box) = (screen.clone(), pick.clone());
                dialog
                    .title(t("cassettes.replay_title"))
                    .w_96()
                    .child(
                        v_flex().gap_3().child(rows).child(
                            Checkbox::new("cassette-passthrough")
                                .small()
                                .label(t("cassettes.passthrough"))
                                .tooltip(t("cassettes.passthrough_tooltip"))
                                .checked(passthrough)
                                .on_click(move |checked: &bool, window, cx| {
                                    let checked = *checked;
                                    pick_for_box.update(cx, |p, cx| {
                                        p.passthrough = checked;
                                        cx.notify();
                                    });
                                    window.refresh();
                                }),
                        ),
                    )
                    .footer(
                        DialogFooter::new()
                            .child(
                                Button::new("replay-cancel")
                                    .outline()
                                    .label(t("components.cancel"))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("replay-confirm")
                                    .primary()
                                    .label(t("cassettes.replay"))
                                    .disabled(chosen.is_none())
                                    .on_click(move |_, window, cx| {
                                        let Some(name) = chosen.clone() else { return };
                                        screen.update(cx, |this, cx| this.replay_start(&name, passthrough, cx)).ok();
                                        window.close_dialog(cx);
                                    }),
                            ),
                    )
            });
        });
    }

    fn replay_start(&mut self, name: &str, passthrough: bool, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "name": name, "passthrough": passthrough });
        call(&client, cx, "cassette.replayStart", payload, |this, outcome, cx| match failure(&outcome) {
            Some(message) => this.report(t("cassettes.error.replay"), message, cx),
            None => this.show_warnings(&outcome.unwrap_or_default(), cx),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmatched_lines_are_capped() {
        let one = json!({"method": "GET", "url": "https://a.test/x"});
        let stopped = json!({"unmatched": vec![one; 7]});
        let lines = unmatched_lines(&stopped);
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0], "✗ GET https://a.test/x");
    }
}
