//! The Webhooks screen: the engine's webhook receivers, the requests they got, and sample events to send.
//!
//! A listener is a port on 127.0.0.1 that records every request it gets (and can forward it). The listeners
//! belong to the engine, not to a workspace: `webhook.list` names every one, and this screen shows the open
//! workspace's, kept current by `event.webhook.changed`. The app starts them detached, so they outlive its
//! session. What they received is a Capture of origin `webhook`, embedded below (it also replays them and
//! saves them as requests). A sample event (`webhook.fixtures`) is sent through `webhook.trigger`, signed
//! with a vault's secret when one is named; the engine reads that secret itself once the vault is unsealed.

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    Window, div, px,
};
use serde_json::{Value, json};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        dialogs::{PromptField, prompt},
        page,
    },
    engine::{call, call_in, describe, failure},
    i18n::{t, t_with},
    model::{traffic, vault::VaultState},
    screens::capture::CaptureScreen,
};

/// A listener of the open workspace.
#[derive(Debug, Clone, PartialEq)]
struct Listener {
    id: String,
    port: u64,
    name: Option<String>,
    forward: Option<String>,
    hits: u64,
}

impl Listener {
    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            id: text("id")?,
            port: value.get("port").and_then(Value::as_u64)?,
            name: text("name"),
            forward: text("forward"),
            hits: value.get("hits").and_then(Value::as_u64).unwrap_or_default(),
        })
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// The listeners of `workspace` in a `webhook.list` answer (or `event.webhook.changed` payload).
fn listeners_of(payload: &Value, workspace: &str) -> Vec<Listener> {
    payload
        .get("listeners")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|listener| listener.get("workspaceId").and_then(Value::as_str) == Some(workspace))
        .filter_map(Listener::from_value)
        .collect()
}

/// A sample event the engine can build.
#[derive(Debug, Clone, PartialEq)]
struct Fixture {
    name: String,
    description: String,
}

impl Fixture {
    /// The sender the fixture imitates: the part of its name before the first dot, capitalised.
    fn group(&self) -> String {
        let head = self.name.split('.').next().unwrap_or_default();
        let mut letters = head.chars();
        letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
    }
}

pub struct WebhooksScreen {
    model: Entity<AppModel>,
    listeners: Vec<Listener>,
    /// The listeners have been read for the workspace in front; before that the section says nothing.
    loaded: bool,
    fixtures: Vec<Fixture>,
    /// The requests the listeners got.
    received: Entity<CaptureScreen>,
    _subscriptions: Vec<Subscription>,
}

impl WebhooksScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let received = cx.new(|cx| CaptureScreen::received(model.clone(), window, cx));
        let subscriptions = vec![
            cx.observe(&received, |_, _, cx| cx.notify()),
            cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
                ModelEvent::WebhookChanged(payload) => {
                    this.listeners = listeners_of(payload, this.model.read(cx).workspace_id());
                    this.loaded = true;
                    cx.notify();
                }
                ModelEvent::WorkspaceSwitched => {
                    this.listeners.clear();
                    this.loaded = false;
                    this.refresh(window, cx);
                }
                // The screen is built before the workspace is known; the first snapshot is when it can load.
                ModelEvent::ConfigChanged if !this.loaded => this.refresh(window, cx),
                ModelEvent::EngineRestarted => this.refresh(window, cx),
                // The hit counts ride on `webhook.list` only, so a request that arrived asks for them again.
                ModelEvent::Traffic(payload)
                    if traffic::parse_event(payload).iter().any(|entry| entry.origin.as_deref() == Some("webhook")) =>
                {
                    this.refresh_listeners(cx)
                }
                _ => {}
            }),
        ];
        let mut this = Self {
            model,
            listeners: Vec::new(),
            loaded: false,
            fixtures: Vec::new(),
            received,
            _subscriptions: subscriptions,
        };
        this.refresh_listeners(cx);
        this.refresh_fixtures(cx);
        this
    }

    /// The embedded Capture, whose saves the shell opens like Capture's own.
    pub fn received(&self) -> &Entity<CaptureScreen> {
        &self.received
    }

    /// One line for the footer.
    pub fn status_line(&self) -> String {
        let count = self.listeners.len();
        t_with(
            if count == 1 { "webhooks.status.count_one" } else { "webhooks.status.count_other" },
            &[("count", &count)],
        )
    }

    fn workspace(&self, cx: &gpui_kit::App) -> String {
        self.model.read(cx).workspace_id().to_owned()
    }

    fn notice(&mut self, text: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(text, cx));
    }

    /// Reads everything again: the screen was brought to the front, or the workspace changed.
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_listeners(cx);
        self.refresh_fixtures(cx);
        self.received.update(cx, |received, cx| received.reload(window, cx));
    }

    fn refresh_listeners(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        if workspace.is_empty() {
            return;
        }
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "webhook.list", json!({}), move |this, outcome, cx| {
            // The answer to a workspace that has been left since.
            if this.workspace(cx) != workspace {
                return;
            }
            match outcome {
                Ok(result) => {
                    this.listeners = listeners_of(&result, &workspace);
                    this.loaded = true;
                }
                Err(error) => {
                    let what = t_with("webhooks.error.list", &[("message", &describe(&error))]);
                    this.notice(what, cx);
                }
            }
            cx.notify();
        });
    }

    fn refresh_fixtures(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "webhook.fixtures", json!({}), |this, outcome, cx| {
            if let Ok(result) = outcome {
                this.fixtures = result
                    .get("fixtures")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|fixture| {
                        Some(Fixture {
                            name: fixture.get("name")?.as_str()?.to_owned(),
                            description: fixture
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        })
                    })
                    .collect();
                cx.notify();
            }
        });
    }

    /// Asks for a listener's name, port and forward URL, all optional. `draft` is what a refused dialog held.
    fn prompt_listen(&mut self, draft: Option<Vec<String>>, window: &mut Window, cx: &mut Context<Self>) {
        let initial = draft.unwrap_or_else(|| vec![String::new(); 3]);
        let fields = vec![
            PromptField::new(t("webhooks.field.name"), t("webhooks.field.name_placeholder")).optional(),
            PromptField::new(t("webhooks.field.port"), t("webhooks.field.port_placeholder")).optional(),
            PromptField::new(t("webhooks.field.forward"), t("webhooks.field.forward_placeholder")).optional(),
        ];
        let fields = fields.into_iter().zip(initial).map(|(field, value)| field.initial(value)).collect();
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("webhooks.listen.title"),
            t("webhooks.listen.confirm"),
            fields,
            |this, values, window, cx| {
                let port = match values[1].as_str() {
                    "" => None,
                    text => match text.parse::<u16>() {
                        Ok(port) => Some(port),
                        // The dialog has closed; it opens again with what was typed.
                        Err(_) => {
                            this.notice(t("webhooks.error.port").to_owned(), cx);
                            return this.prompt_listen(Some(values), window, cx);
                        }
                    },
                };
                // Detached: a listener the app's session owned would die with it.
                let mut payload = json!({ "workspaceId": this.workspace(cx), "detach": true });
                if !values[0].is_empty() {
                    payload["name"] = json!(values[0]);
                }
                if let Some(port) = port {
                    payload["port"] = json!(port);
                }
                if !values[2].is_empty() {
                    payload["forward"] = json!(values[2]);
                }
                let client = this.model.read(cx).client().clone();
                call(&client, cx, "webhook.listen", payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.notice(t_with("webhooks.error.listen", &[("message", &message)]), cx);
                    }
                });
            },
        );
    }

    fn stop(&mut self, id: String, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "webhook.stop", json!({ "id": id }), |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.notice(t_with("webhooks.error.stop", &[("message", &message)]), cx);
            }
        });
    }

    fn copy(text: String, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        window.push_notification(Notification::success(t("webhooks.copied")), cx);
    }

    /// Asks where to send `fixture` and which vault signs it, both optional: with no target the engine picks
    /// the workspace's one listener.
    fn prompt_send(&mut self, fixture: String, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t_with("webhooks.send.title", &[("fixture", &fixture)]),
            t("webhooks.send.confirm"),
            vec![
                PromptField::new(t("webhooks.field.target"), t("webhooks.field.target_placeholder")).optional(),
                PromptField::new(t("webhooks.field.vault"), t("webhooks.field.vault_placeholder")).optional(),
            ],
            move |this, values, window, cx| this.send(fixture.clone(), values, window, cx),
        );
    }

    /// Sends the sample event. A signing vault that is sealed is unsealed first, with the passphrase asked for.
    fn send(&mut self, fixture: String, values: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let mut payload = json!({ "workspaceId": self.workspace(cx), "fixture": fixture });
        match values[0].as_str() {
            "" => {}
            url if url.starts_with("http://") || url.starts_with("https://") => payload["to"] = json!(url),
            name => payload["name"] = json!(name),
        }
        if values[1].is_empty() {
            return self.trigger(payload, window, cx);
        }
        let found = self.model.read(cx).vaults().iter().find(|v| v.name.eq_ignore_ascii_case(&values[1])).cloned();
        let Some(found) = found else {
            return self.notice(t_with("webhooks.error.vault", &[("name", &values[1])]), cx);
        };
        payload["vault"] = json!(found.name);
        if found.state == VaultState::Unsealed {
            return self.trigger(payload, window, cx);
        }
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t_with("webhooks.unseal.title", &[("vault", &found.name)]),
            t("webhooks.unseal.confirm"),
            vec![PromptField::new(t("vaults.passphrase"), "").secret()],
            move |this, values, window, cx| {
                let unseal = json!({ "workspaceId": this.workspace(cx), "id": found.id, "passphrase": values[0] });
                let client = this.model.read(cx).client().clone();
                let payload = payload.clone();
                call_in(&client, window, cx, "vault.unseal", unseal, move |this, outcome, window, cx| {
                    match failure(&outcome) {
                        Some(message) => this.notice(t_with("webhooks.error.send", &[("message", &message)]), cx),
                        None => this.trigger(payload, window, cx),
                    }
                });
            },
        );
    }

    fn trigger(&mut self, payload: Value, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "webhook.trigger", payload, |this, outcome, window, cx| match failure(&outcome) {
            Some(message) => this.notice(t_with("webhooks.error.send", &[("message", &message)]), cx),
            None => {
                let status = outcome.ok().and_then(|sent| sent.get("status")?.as_u64()).unwrap_or_default();
                let text = t_with("webhooks.sent", &[("status", &status)]);
                let note = if status < 400 { Notification::success(text) } else { Notification::warning(text) };
                window.push_notification(note, cx);
            }
        });
    }

    fn render_listener(&self, listener: &Listener, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let key = |part: &str| ElementId::Name(format!("webhook-{part}-{}", listener.id).into());
        let (url, id) = (listener.url(), listener.id.clone());
        let mut route = vec![listener.url()];
        route.extend(listener.forward.as_ref().map(|forward| t_with("webhooks.listener.forward", &[("url", forward)])));
        h_flex()
            .id(key("listener"))
            .test_support()
            .w_full()
            .gap_3()
            .px_4()
            .py_2()
            .text_sm()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .w_40()
                    .flex_none()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(listener.name.clone().unwrap_or_else(|| t("webhooks.listener.unnamed").to_owned())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child(route.join("  ")),
            )
            .child(
                div()
                    .w_24()
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(t_with("webhooks.listener.hits", &[("count", &listener.hits)])),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(
                        Button::new(key("copy"))
                            .small()
                            .ghost()
                            .icon(IconName::Copy)
                            .tooltip(t("webhooks.listener.copy"))
                            .on_click(cx.listener(move |_, _, window, cx| Self::copy(url.clone(), window, cx))),
                    )
                    .child(
                        Button::new(key("stop"))
                            .small()
                            .icon(IconName::Square)
                            .label(t("webhooks.listener.stop"))
                            .on_click(cx.listener(move |this, _, _, cx| this.stop(id.clone(), cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_listeners(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let body = if !self.loaded {
            div().into_any_element()
        } else if self.listeners.is_empty() {
            div()
                .px_4()
                .py_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(t("webhooks.listeners.empty"))
                .into_any_element()
        } else {
            v_flex()
                .children(self.listeners.iter().map(|listener| self.render_listener(listener, cx)))
                .into_any_element()
        };
        v_flex()
            .flex_none()
            .child(section_heading(t("webhooks.listeners.title")))
            .child(div().id("webhook-listeners").max_h(px(180.)).overflow_y_scroll().child(body))
            .into_any_element()
    }

    /// The sample events, under the name of the sender each imitates.
    fn render_fixtures(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let mut groups: Vec<String> = Vec::new();
        for fixture in &self.fixtures {
            if !groups.contains(&fixture.group()) {
                groups.push(fixture.group());
            }
        }
        let body = if self.fixtures.is_empty() {
            div()
                .px_4()
                .py_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(t("webhooks.samples.empty"))
                .into_any_element()
        } else {
            v_flex()
                .children(groups.into_iter().map(|group| {
                    v_flex().child(div().px_4().pt_1().child(page::section_label(group.clone(), cx))).children(
                        self.fixtures.iter().filter(|fixture| fixture.group() == group).map(|fixture| {
                            let name = fixture.name.clone();
                            h_flex()
                                .w_full()
                                .gap_3()
                                .px_4()
                                .py_1()
                                .text_sm()
                                .child(
                                    div()
                                        .w_72()
                                        .flex_none()
                                        .font_family(theme.mono_font_family.clone())
                                        .child(name.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .text_color(theme.muted_foreground)
                                        .child(fixture.description.clone()),
                                )
                                .child(
                                    Button::new(ElementId::Name(format!("webhook-send-{name}").into()))
                                        .small()
                                        .icon(IconName::Play)
                                        .label(t("webhooks.samples.send"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.prompt_send(name.clone(), window, cx)
                                        })),
                                )
                        }),
                    )
                }))
                .into_any_element()
        };
        v_flex()
            .flex_none()
            .child(section_heading(t("webhooks.samples.title")))
            .child(div().id("webhook-samples").max_h(px(180.)).overflow_y_scroll().child(body))
            .into_any_element()
    }
}

fn section_heading(title: &'static str) -> impl IntoElement {
    div().flex_none().px_4().pt_3().pb_1().text_sm().font_medium().child(title)
}

impl Render for WebhooksScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            t("webhooks.page.title"),
            t("webhooks.page.subtitle"),
            Button::new("webhook-listen")
                .small()
                .icon(IconName::Plus)
                .label(t("webhooks.listen.button"))
                .on_click(cx.listener(|this, _, window, cx| this.prompt_listen(None, window, cx))),
            cx,
        );
        let body = v_flex()
            .size_full()
            .child(self.render_listeners(cx))
            .child(self.render_fixtures(cx))
            .child(section_heading(t("webhooks.received.title")))
            .child(div().flex_1().min_h_0().child(self.received.clone()));
        page::screen(header, body, cx)
    }
}
