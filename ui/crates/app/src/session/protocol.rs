//! An open GraphQL, gRPC or SOAP request.
//!
//! These are listed in the explorer, opened by id and sent by id (`graphql.send`, `grpc.send`,
//! `soap.send`), and their stored fields are shown here. Editing them is not part of the desktop UI yet: the
//! tab says so rather than offering controls that would not do the work. The engine already treats each as
//! a first-class saved request, so nothing about them is lost by opening and sending from here.

use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    description_list::DescriptionList,
    h_flex,
    notification::Notification,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use serde_json::{Map, Value, json};

use super::{LoadState, SessionEvent, exchange::Exchange};
use crate::{
    app_model::AppModel,
    engine::{call_in, describe},
    model::{RequestKind, http::BodyEncoding},
};

pub struct ProtocolSession {
    model: Entity<AppModel>,
    kind: RequestKind,
    workspace_id: String,
    id: String,
    entity: Map<String, Value>,
    load: LoadState,
    exchange: Entity<Exchange>,
    sending: bool,
}

impl EventEmitter<SessionEvent> for ProtocolSession {}

impl ProtocolSession {
    pub fn open_saved(
        model: Entity<AppModel>,
        kind: RequestKind,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        debug_assert!(kind.send_command().is_some(), "{} is not an HTTP-like request type", kind.label());
        let workspace_id = model.read(cx).workspace_id().to_owned();
        let exchange = cx.new(|cx| Exchange::new(window, cx));
        let this = Self {
            model,
            kind,
            workspace_id: workspace_id.clone(),
            id: id.clone(),
            entity: Map::new(),
            load: LoadState::Loading,
            exchange,
            sending: false,
        };
        let client = this.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": workspace_id, "kind": kind.entity_kind(), "id": id });
        call_in(&client, window, cx, "entity.load", payload, |this, outcome, _, cx| {
            match outcome.map_err(|e| describe(&e)).and_then(|v| {
                v.get("entity")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| "The request no longer exists.".to_owned())
            }) {
                Ok(entity) => {
                    this.entity = entity;
                    this.load = LoadState::Ready;
                }
                Err(message) => this.load = LoadState::Failed(message),
            }
            cx.emit(SessionEvent::Changed);
            cx.notify();
        });
        this
    }

    pub fn kind(&self) -> RequestKind {
        self.kind
    }

    pub fn saved_id(&self) -> &str {
        &self.id
    }

    pub fn exchange(&self) -> &Entity<Exchange> {
        &self.exchange
    }

    pub fn title(&self) -> SharedString {
        match self.entity.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            Some(name) => name.to_owned().into(),
            None => format!("{} request", self.kind.label()).into(),
        }
    }

    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.kind.send_command() else { return };
        if self.sending || self.load != LoadState::Ready {
            return;
        }
        self.sending = true;
        self.exchange.update(cx, |exchange, cx| exchange.begin(cx));
        cx.notify();
        let summary = format!("{} {}", self.kind.label(), self.title());
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.workspace_id, "id": self.id });
        call_in(&client, window, cx, action, payload, move |this, outcome, window, cx| {
            this.sending = false;
            match outcome {
                Ok(result) => {
                    let status = this
                        .exchange
                        .update(cx, |exchange, cx| exchange.finish(&result, BodyEncoding::Text, window, cx));
                    cx.emit(SessionEvent::Activity(match status {
                        Some(status) => format!("{summary} → {status}"),
                        None => format!("{summary} → done"),
                    }));
                }
                Err(error) => {
                    let message = describe(&error);
                    this.exchange.update(cx, |exchange, cx| exchange.fail(message.clone(), cx));
                    window
                        .push_notification(Notification::error(message.clone()).title("Couldn't send the request"), cx);
                    cx.emit(SessionEvent::Activity(format!("{summary} failed: {message}")));
                }
            }
            cx.notify();
        });
    }

    /// The stored fields worth showing, as `(label, value)`, for each kind.
    fn summary(&self) -> Vec<(&'static str, String)> {
        let text = |key: &str| self.entity.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let headers = |key: &str| {
            self.entity
                .get(key)
                .and_then(Value::as_object)
                .map(|map| map.len())
                .map(|n| format!("{n} header{}", if n == 1 { "" } else { "s" }))
                .unwrap_or_default()
        };
        match self.kind {
            RequestKind::GraphQl => vec![
                ("Endpoint", text("endpointUrl")),
                ("Operation", text("operationName")),
                ("Headers", headers("headers")),
                ("Query", text("query")),
                ("Variables", text("variables")),
            ],
            RequestKind::Grpc => vec![
                ("Server", text("serverAddress")),
                ("Service", text("serviceName")),
                ("Method", text("methodName")),
                ("Metadata", headers("metadata")),
                ("Message", text("requestBody")),
            ],
            RequestKind::Soap => vec![
                ("Endpoint", text("endpointUrl")),
                ("Action", text("soapAction")),
                ("Headers", headers("headers")),
                ("Envelope", text("body")),
            ],
            RequestKind::Rest | RequestKind::WebSocket => Vec::new(),
        }
    }
}

impl Render for ProtocolSession {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        match &self.load {
            LoadState::Loading => {
                return v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .bg(theme.background)
                    .child(Spinner::new())
                    .child("Loading request…")
                    .into_any_element();
            }
            LoadState::Failed(message) => {
                return div()
                    .size_full()
                    .p_4()
                    .bg(theme.background)
                    .child(
                        Alert::error("load-failed", SharedString::from(message.clone()))
                            .title("Couldn't open the request"),
                    )
                    .into_any_element();
            }
            LoadState::Ready => {}
        }

        v_flex()
            .size_full()
            .bg(theme.background)
            .gap_3()
            .p_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().text_lg().font_semibold().child(self.title()))
                    .child(
                        Button::new("send-saved")
                            .primary()
                            .label("Send")
                            .loading(self.sending)
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
            .child(Alert::info(
                "not-editable",
                SharedString::from(format!(
                    "{} requests can be opened and sent here, but not edited yet. Edit this one in the Electron app or with the bifurc CLI.",
                    self.kind.label()
                )),
            ))
            .child(
                div().id("protocol-fields").flex_1().min_h_0().overflow_y_scroll().child(
                    DescriptionList::vertical().children(
                        self.summary()
                            .into_iter()
                            .filter(|(_, value)| !value.is_empty())
                            .map(|(label, value)| gpui_kit::component::description_list::DescriptionItem::new(label).value(value)),
                    ),
                ),
            )
            .into_any_element()
    }
}
