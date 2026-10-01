//! The Capture screen: the traffic the proxy has seen, newest first, and one exchange in detail, much like
//! a browser's Network tab.
//!
//! The history is the engine's (a window opened later still sees what passed) and is extended live by
//! `event.log.entry`. Both carry only a light summary of each exchange; the headers and bodies of one are
//! fetched by id (`capture.get`) when it is selected. Pausing stops the engine recording
//! (`capture.setPaused`); nothing passes into the history until it is resumed. An exchange can be turned into a saved request or a mock: the engine builds the entity
//! from the stored entry (`capture.buildRequest`, `capture.buildMock`) and this screen saves what it built.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    list::ListItem,
    notification::Notification,
    resizable::{resizable_panel, v_resizable},
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    UniformListScrollHandle, Window, div, px, uniform_list,
};
use serde_json::{Value, json};
use std::ops::Range;

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        Syntax,
        binary::{BinaryBody, save_to_file, suggested_file_name},
        page,
    },
    engine::{call, call_in, describe, failure},
    model::{
        http::{pretty_json, status_text},
        media::{self, BodyClass},
        traffic::{self, Exchange, RESPONSE_BODY_LIMIT, TrafficEntry, body_class, clock_utc},
    },
};

/// What happened on this screen that the window should act on.
#[derive(Debug, Clone)]
pub enum CaptureEvent {
    /// An exchange was saved as a REST request with this id.
    SavedRequest { id: String },
    /// An exchange was saved as a mock with this id.
    SavedMock { id: String },
}

impl EventEmitter<CaptureEvent> for CaptureScreen {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Request,
    Response,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Headers,
    Payload,
    Response,
}

const SECTIONS: [Section; 3] = [Section::Headers, Section::Payload, Section::Response];

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyView {
    Pretty,
    Raw,
}

/// One body of an exchange, prepared for showing.
enum Body {
    Empty,
    /// Text for the read-only editor: as received, and re-indented when that is possible.
    Text {
        raw: String,
        pretty: Option<String>,
    },
    /// An image or other data, decoded once here rather than per frame.
    Data(BinaryBody),
}

impl Body {
    /// The body, and how to highlight it when it is text.
    fn new(bytes: &[u8], content_type: &str) -> (Self, Syntax) {
        match body_class(content_type, bytes) {
            None => (Body::Empty, Syntax::Plain),
            Some(BodyClass::Text) => {
                let raw = String::from_utf8_lossy(bytes).into_owned();
                let mut syntax = Syntax::for_content_type(content_type);
                // A JSON body is recognised by its content too: servers often mislabel it.
                if syntax == Syntax::Plain && pretty_json(&raw).is_some() {
                    syntax = Syntax::Json;
                }
                let pretty = if syntax == Syntax::Json { pretty_json(&raw) } else { None };
                (Body::Text { raw, pretty }, syntax)
            }
            Some(_) => (Body::Data(BinaryBody::new(bytes.to_vec(), content_type)), Syntax::Plain),
        }
    }

    fn text(&self, view: BodyView) -> Option<String> {
        match self {
            Body::Text { pretty: Some(pretty), .. } if view == BodyView::Pretty => Some(pretty.clone()),
            Body::Text { raw, .. } => Some(raw.clone()),
            _ => None,
        }
    }

    fn can_pretty(&self) -> bool {
        matches!(self, Body::Text { pretty: Some(_), .. })
    }
}

/// A fetched exchange with its two bodies prepared.
struct Shown {
    exchange: Exchange,
    request: Body,
    response: Body,
}

enum Detail {
    Idle,
    Loading,
    Failed(String),
    Ready(Box<Shown>),
}

pub struct CaptureScreen {
    model: Entity<AppModel>,
    entries: Vec<TrafficEntry>,
    /// The engine's history has been read at least once.
    loaded: bool,
    /// Mirrors the engine: while capture is paused the engine records nothing, and the list stops following.
    paused: bool,
    filter: Entity<InputState>,
    selected: Option<String>,
    detail: Detail,
    section: Section,
    response_view: BodyView,
    request_body: Entity<EditorState>,
    response_body: Entity<EditorState>,
    /// A save as request or mock is in flight.
    saving: bool,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

fn read_only_editor(window: &mut Window, cx: &mut Context<CaptureScreen>) -> Entity<EditorState> {
    cx.new(|cx| {
        let mut state =
            EditorState::new(window, cx).language(Syntax::Plain.language()).line_number(true).soft_wrap(true);
        state.set_readonly(true, cx);
        state
    })
}

impl CaptureScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by URL, method, status, type"));
        let request_body = read_only_editor(window, cx);
        let response_body = read_only_editor(window, cx);
        let subscriptions = vec![
            cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
                ModelEvent::Traffic(payload) if !this.paused => {
                    let active = this.model.read(cx).workspace_id().to_owned();
                    let incoming = traffic::only_workspace(traffic::parse_event(payload), &active);
                    // The selected exchange changed (its status landed, say): its detail is out of date.
                    let refresh =
                        this.selected.as_ref().filter(|id| incoming.iter().any(|entry| &entry.id == *id)).cloned();
                    // A batch is oldest first; the list is newest first.
                    traffic::merge_live(&mut this.entries, incoming);
                    if let Some(id) = refresh {
                        this.fetch(id, window, cx);
                    }
                    cx.notify();
                }
                ModelEvent::WorkspaceSwitched => {
                    this.selected = None;
                    this.detail = Detail::Idle;
                    this.reload(window, cx);
                }
                // The screen is built before the workspace is known; the first snapshot is when it can load.
                ModelEvent::ConfigChanged if !this.loaded => this.reload(window, cx),
                _ => {}
            }),
            cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        let mut this = Self {
            model,
            entries: Vec::new(),
            loaded: false,
            paused: false,
            filter,
            selected: None,
            detail: Detail::Idle,
            section: Section::Headers,
            response_view: BodyView::Pretty,
            request_body,
            response_body,
            saving: false,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.reload(window, cx);
        this
    }

    /// One line for the footer: how much is captured, how much of it the filter shows.
    pub fn status_line(&self, cx: &gpui_kit::App) -> String {
        let total = self.entries.len();
        let shown = self.visible(cx).len();
        let mut line = format!("{total} captured");
        if shown < total {
            line.push_str(&format!(" · {shown} shown"));
        }
        if self.paused {
            line.push_str(" · paused");
        }
        line
    }

    fn visible(&self, cx: &gpui_kit::App) -> Vec<&TrafficEntry> {
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        self.entries.iter().filter(|entry| entry.matches(&needle)).collect()
    }

    fn reload(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if model.workspace_id().is_empty() {
            return;
        }
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "summary": true });
        call(&client, cx, "capture.list", payload, |this, outcome, cx| {
            match outcome {
                Ok(result) => {
                    this.entries = traffic::parse_list(&result);
                    this.paused = result.get("paused").and_then(Value::as_bool).unwrap_or(false);
                    this.loaded = true;
                }
                Err(error) => this.report("Couldn't read captured traffic.", describe(&error), cx),
            }
            cx.notify();
        });
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
    }

    pub fn toggle_paused(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let resume = self.paused;
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "paused": !resume });
        call(&client, cx, "capture.setPaused", payload, move |this, outcome, cx| {
            match failure(&outcome) {
                Some(message) => {
                    let what = if resume { "Couldn't resume capture." } else { "Couldn't pause capture." };
                    this.report(what, message, cx);
                }
                None => {
                    if let Ok(result) = &outcome {
                        this.paused = result.get("paused").and_then(Value::as_bool).unwrap_or(!resume);
                    }
                }
            }
            cx.notify();
        });
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id() });
        call(&client, cx, "capture.clear", payload, |this, outcome, cx| match failure(&outcome) {
            Some(message) => this.report("Couldn't clear captured traffic.", message, cx),
            None => {
                this.entries.clear();
                this.selected = None;
                this.detail = Detail::Idle;
                cx.notify();
            }
        });
    }

    fn selected_entry(&self) -> Option<&TrafficEntry> {
        let id = self.selected.as_deref()?;
        self.entries.iter().find(|entry| entry.id == id)
    }

    fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = id.clone();
        self.response_view = BodyView::Pretty;
        match id {
            Some(id) => {
                self.detail = Detail::Loading;
                self.fetch(id, window, cx);
            }
            None => self.detail = Detail::Idle,
        }
        cx.notify();
    }

    /// Asks the engine for the whole exchange. An answer for an exchange that is no longer selected is dropped.
    fn fetch(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "id": id });
        call_in(&client, window, cx, "capture.get", payload, move |this, outcome, window, cx| {
            if this.selected.as_deref() != Some(id.as_str()) {
                return;
            }
            if let Some(message) = failure(&outcome) {
                this.detail = Detail::Failed(message);
                return cx.notify();
            }
            let exchange = outcome.ok().and_then(|result| Exchange::from_value(result.get("entry")?));
            match exchange {
                Some(exchange) => this.show(exchange, window, cx),
                None => {
                    this.detail = Detail::Failed("This exchange is no longer in the engine's history.".to_owned());
                }
            }
            cx.notify();
        });
    }

    /// Takes a fetched exchange and puts its bodies into the editors.
    fn show(&mut self, exchange: Exchange, window: &mut Window, cx: &mut Context<Self>) {
        let (request, request_syntax) = Body::new(&exchange.request_body, exchange.request_content_type());
        let (response, response_syntax) = Body::new(&exchange.response_body, exchange.response_content_type());
        if let Some(text) = request.text(BodyView::Pretty) {
            self.request_body.update(cx, |state, cx| {
                state.set_highlighter(request_syntax.language(), cx);
                state.set_value(text, window, cx);
            });
        }
        if let Some(text) = response.text(self.response_view) {
            self.response_body.update(cx, |state, cx| {
                state.set_highlighter(response_syntax.language(), cx);
                state.set_value(text, window, cx);
            });
        }
        self.detail = Detail::Ready(Box::new(Shown { exchange, request, response }));
    }

    fn set_response_view(&mut self, view: BodyView, window: &mut Window, cx: &mut Context<Self>) {
        if self.response_view == view {
            return;
        }
        self.response_view = view;
        if let Detail::Ready(shown) = &self.detail
            && let Some(text) = shown.response.text(view)
        {
            self.response_body.update(cx, |state, cx| state.set_value(text, window, cx));
        }
        cx.notify();
    }

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.take() else { return };
        self.detail = Detail::Idle;
        self.entries.retain(|entry| entry.id != id);
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "ids": [id] });
        call(&client, cx, "capture.remove", payload, |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report("Couldn't remove the exchange.", message, cx);
            }
        });
        cx.notify();
    }

    fn save_failed(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.saving = false;
        self.report(what, message, cx);
        cx.notify();
    }

    /// Saves the selected exchange as a request or as a mock. The engine builds the entity from the entry it
    /// holds; what it built is created as it is, named after the exchange when it has no name (the method is shown beside the name already).
    fn save_as(&mut self, mock: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(entry) = self.selected_entry() else { return };
        let id = entry.id.clone();
        let name = entry.name();
        let (build, field, kind, done) = if mock {
            ("capture.buildMock", "mock", "mocks", "Saved as a mock.")
        } else {
            ("capture.buildRequest", "request", "requests", "Saved as a request.")
        };
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace_id = model.workspace_id().to_owned();
        let create_client = client.clone();
        self.saving = true;
        cx.notify();
        let payload = json!({ "workspaceId": workspace_id, "id": id });
        call_in(&client, window, cx, build, payload, move |this, outcome, window, cx| {
            if let Some(message) = failure(&outcome) {
                return this.save_failed("Couldn't build it from this exchange.", message, cx);
            }
            let entity = outcome.ok().and_then(|result| result.get(field).cloned()).filter(Value::is_object);
            let Some(mut entity) = entity else {
                return this.save_failed(
                    "Couldn't build it from this exchange.",
                    "The engine returned nothing.".into(),
                    cx,
                );
            };
            if let Some(object) = entity.as_object_mut() {
                let unnamed = object.get("name").and_then(Value::as_str).is_none_or(str::is_empty);
                if unnamed {
                    object.insert("name".to_owned(), Value::String(name));
                }
            }
            let payload = json!({ "kind": kind, "workspaceId": workspace_id, "entity": entity });
            call_in(&create_client, window, cx, "entity.create", payload, move |this, outcome, window, cx| {
                if let Some(message) = failure(&outcome) {
                    return this.save_failed("Couldn't save it.", message, cx);
                }
                let created =
                    outcome.ok().and_then(|result| result.get("entity")?.get("id")?.as_str().map(str::to_owned));
                let Some(created) = created else {
                    return this.save_failed("Couldn't save it.", "The engine returned nothing.".into(), cx);
                };
                this.saving = false;
                window.push_notification(Notification::success(done), cx);
                cx.emit(if mock {
                    CaptureEvent::SavedMock { id: created }
                } else {
                    CaptureEvent::SavedRequest { id: created }
                });
                cx.notify();
            });
        });
    }

    /// Asks where to save the body of one side and writes it there.
    fn save_body(&mut self, side: Side, cx: &mut Context<Self>) {
        let Detail::Ready(shown) = &self.detail else { return };
        let (body, name, content_type) = match side {
            Side::Request => (&shown.request, "request", shown.exchange.request_content_type()),
            Side::Response => (&shown.response, "response", shown.exchange.response_content_type()),
        };
        let Body::Data(binary) = body else { return };
        let file_name = suggested_file_name(&shown.exchange.url, name, content_type);
        save_to_file(binary.bytes().to_vec(), file_name, cx, |this, outcome, cx| match outcome {
            Ok(path) => this.report("Saved to", format!("{}.", path.display()), cx),
            Err(message) => this.report("Couldn't save the file.", message, cx),
        });
    }

    fn status_tag(status: Option<u16>) -> Tag {
        match status {
            Some(code) if (200..400).contains(&code) => Tag::success().small().child(code.to_string()),
            Some(code) => Tag::danger().small().child(code.to_string()),
            None => Tag::secondary().small().child("Pending"),
        }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        // Positions in `entries`, so the rows in view can be read in place instead of copied.
        let visible: Vec<usize> =
            self.entries.iter().enumerate().filter(|(_, entry)| entry.matches(&needle)).map(|(ix, _)| ix).collect();
        if self.entries.is_empty() {
            return page::empty(
                "No traffic captured",
                if self.paused {
                    "Capture is paused. Nothing is being recorded until you resume."
                } else {
                    "Requests that pass through the proxy are listed here as they happen."
                },
            );
        }
        if visible.is_empty() {
            return page::empty("No matches", "No captured exchange matches the filter.");
        }
        let count = visible.len();
        let row_theme = theme.clone();
        // Only the rows in view are built; the list measures one row and scrolls by that height.
        let rows = uniform_list(
            "capture-rows",
            count,
            cx.processor(move |this, range: Range<usize>, _, cx| {
                let theme = &row_theme;
                range
                    .filter_map(|ix| this.entries.get(*visible.get(ix)?))
                    .map(|entry| {
                        let id = entry.id.clone();
                        ListItem::new(gpui_kit::ElementId::Name(format!("capture-{}", entry.id).into()))
                            .selected(this.selected.as_deref() == Some(entry.id.as_str()))
                            .w_full()
                            .px_4()
                            .py_1()
                            .text_sm()
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_2()
                                            .overflow_hidden()
                                            .child(
                                                div().min_w_0().overflow_hidden().text_ellipsis().child(entry.name()),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                                    .child(entry.host()),
                                            ),
                                    )
                                    .child(div().w_16().flex_none().child(entry.method.clone()))
                                    .child(h_flex().w_20().flex_none().child(Self::status_tag(entry.status)))
                                    .child(
                                        div()
                                            .w_16()
                                            .flex_none()
                                            .text_color(theme.muted_foreground)
                                            .child(entry.type_label()),
                                    )
                                    .child(
                                        div()
                                            .w_24()
                                            .flex_none()
                                            .text_color(theme.muted_foreground)
                                            .child(entry.via_label()),
                                    )
                                    .child(
                                        div().w_20().flex_none().text_right().text_color(theme.muted_foreground).child(
                                            entry.size.map(|size| media::size_label(size as usize)).unwrap_or_default(),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .w_20()
                                            .flex_none()
                                            .text_right()
                                            .text_color(theme.muted_foreground)
                                            .child(entry.duration_ms.map(|ms| format!("{ms} ms")).unwrap_or_default()),
                                    )
                                    .child(
                                        div()
                                            .w_20()
                                            .flex_none()
                                            .text_right()
                                            .text_color(theme.muted_foreground)
                                            .child(clock_utc(entry.ts)),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| this.select(Some(id.clone()), window, cx)))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().child("Name"))
                    .child(div().w_16().flex_none().child("Method"))
                    .child(div().w_20().flex_none().child("Status"))
                    .child(div().w_16().flex_none().child("Type"))
                    .child(div().w_24().flex_none().child("Handled by"))
                    .child(div().w_20().flex_none().text_right().child("Size"))
                    .child(div().w_20().flex_none().text_right().child("Time taken"))
                    .child(div().w_20().flex_none().text_right().child("Time (UTC)")),
            )
            .child(rows)
            .into_any_element()
    }

    /// A name and its value, name muted.
    fn pair(
        name: impl Into<SharedString>,
        value: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        h_flex()
            .gap_3()
            .items_start()
            .text_sm()
            .child(div().w_48().flex_none().text_color(cx.theme().muted_foreground).child(name.into()))
            .child(div().flex_1().min_w_0().child(value.into()))
            .into_any_element()
    }

    fn pairs(
        label: &'static str,
        rows: &[(String, String)],
        empty: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_1()
            .child(page::section_label(label, cx))
            .children((rows.is_empty()).then(|| div().text_sm().text_color(muted).child(empty)))
            .children(rows.iter().map(|(name, value)| Self::pair(name.clone(), value.clone(), cx)))
            .into_any_element()
    }

    fn render_headers(&self, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let exchange = &shown.exchange;
        let status = match exchange.status {
            Some(code) if status_text(code).is_empty() => code.to_string(),
            Some(code) => format!("{code} {}", status_text(code)),
            None => "Pending".to_owned(),
        };
        let via = self.selected_entry().map_or("", TrafficEntry::via_label);
        let handled_by = match (via.is_empty(), &exchange.target) {
            (false, Some(target)) => format!("{via} · {target}"),
            (false, None) => via.to_owned(),
            (true, Some(target)) => target.clone(),
            (true, None) => String::new(),
        };
        let mut general = vec![
            Self::pair("URL", exchange.url.clone(), cx),
            Self::pair("Method", exchange.method.clone(), cx),
            Self::pair("Status", status, cx),
        ];
        if !handled_by.is_empty() {
            general.push(Self::pair("Handled by", handled_by, cx));
        }
        if let Some(ms) = exchange.duration_ms {
            general.push(Self::pair("Time taken", format!("{ms} ms"), cx));
        }
        v_flex()
            .id("capture-headers")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .gap_4()
            .child(v_flex().gap_1().child(page::section_label("General", cx)).children(general))
            .child(Self::pairs("Response headers", &exchange.response_headers, "No headers were captured.", cx))
            .child(Self::pairs("Request headers", &exchange.request_headers, "No headers were captured.", cx))
            .into_any_element()
    }

    fn render_payload(&self, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let query = shown.exchange.query();
        let query_block = (!query.is_empty()).then(|| {
            div().id("capture-query").flex_none().max_h(px(160.)).overflow_y_scroll().child(Self::pairs(
                "Query parameters",
                &query,
                "",
                cx,
            ))
        });
        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .children(query_block)
            .child(page::section_label("Request body", cx))
            .child(div().flex_1().min_h_0().child(self.render_body(Side::Request, shown, cx)))
            .into_any_element()
    }

    fn render_response(&self, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let muted = cx.theme().muted_foreground;
        let truncated = shown.exchange.response_body.len() == RESPONSE_BODY_LIMIT;
        v_flex()
            .size_full()
            .child(div().flex_1().min_h_0().child(self.render_body(Side::Response, shown, cx)))
            .children(truncated.then(|| {
                div()
                    .flex_none()
                    .px_4()
                    .py_2()
                    .text_xs()
                    .text_color(muted)
                    .child("Only the first 512 KB of the response is kept.")
            }))
            .into_any_element()
    }

    /// One body: text in the read-only editor, an image or other data by itself, or a note that there is none.
    fn render_body(&self, side: Side, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let (body, editor, key) = match side {
            Side::Request => (&shown.request, &self.request_body, "request"),
            Side::Response => (&shown.response, &self.response_body, "response"),
        };
        match body {
            Body::Empty => page::empty(
                "No body",
                match side {
                    Side::Request => "The request carried no body.",
                    Side::Response => "The response carried no body.",
                },
            ),
            Body::Text { .. } => {
                let copy_from = editor.clone();
                let toggle = (side == Side::Response && body.can_pretty()).then(|| {
                    TabBar::new("capture-body-view")
                        .segmented()
                        .small()
                        .selected_index(if self.response_view == BodyView::Pretty { 0 } else { 1 })
                        .on_click(cx.listener(|this, ix: &usize, window, cx| {
                            this.set_response_view(if *ix == 0 { BodyView::Pretty } else { BodyView::Raw }, window, cx)
                        }))
                        .child(Tab::new().label("Pretty"))
                        .child(Tab::new().label("Raw"))
                });
                v_flex()
                    .size_full()
                    .child(
                        h_flex().flex_none().py_2().gap_2().children(toggle).child(div().flex_1()).child(
                            Button::new(SharedString::from(format!("capture-copy-{key}")))
                                .ghost()
                                .small()
                                .icon(IconName::Copy)
                                .label("Copy")
                                .on_click(move |_, _, cx| {
                                    let text = copy_from.read(cx).value().to_string();
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                }),
                        ),
                    )
                    .child(div().flex_1().min_h_0().child(Editor::new(editor).bordered(false).h_full()))
                    .into_any_element()
            }
            Body::Data(binary) => v_flex()
                .size_full()
                .child(
                    h_flex().flex_none().py_2().gap_2().child(div().flex_1()).child(
                        Button::new(SharedString::from(format!("capture-save-{key}-body")))
                            .ghost()
                            .small()
                            .label("Save to file…")
                            .on_click(cx.listener(move |this, _, _, cx| this.save_body(side, cx))),
                    ),
                )
                .child(div().flex_1().min_h_0().child(binary.render(cx)))
                .into_any_element(),
        }
    }

    fn render_sections(&self, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let selected = SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0);
        let content = match self.section {
            Section::Headers => self.render_headers(shown, cx),
            Section::Payload => self.render_payload(shown, cx),
            Section::Response => self.render_response(shown, cx),
        };
        v_flex()
            .size_full()
            .child(
                div().flex_none().px_4().child(
                    TabBar::new("capture-sections")
                        .underline()
                        .small()
                        .selected_index(selected)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.section = SECTIONS[*ix];
                            cx.notify();
                        }))
                        .child(Tab::new().label("Headers"))
                        .child(Tab::new().label("Payload"))
                        .child(Tab::new().label("Response")),
                ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }

    fn render_detail(&self, entry: &TrafficEntry, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let content = match &self.detail {
            Detail::Idle => div().into_any_element(),
            Detail::Loading => page::loading("Loading the exchange…"),
            Detail::Failed(message) => div()
                .p_4()
                .child(
                    Alert::error("capture-load-failed", SharedString::from(message.clone()))
                        .title("Couldn't load the exchange"),
                )
                .into_any_element(),
            Detail::Ready(shown) => self.render_sections(shown, cx),
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_none().text_sm().child(entry.method.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_family(theme.mono_font_family.clone())
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(entry.url.clone()),
                    )
                    .child(Self::status_tag(entry.status))
                    .child(
                        Button::new("capture-save-request")
                            .small()
                            .label("Save as request")
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.save_as(false, window, cx))),
                    )
                    .child(
                        Button::new("capture-save-mock")
                            .small()
                            .label("Save as mock")
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| this.save_as(true, window, cx))),
                    )
                    .child(
                        Button::new("capture-remove")
                            .ghost()
                            .small()
                            .icon(IconName::Delete)
                            .tooltip("Remove from the list")
                            .on_click(cx.listener(|this, _, _, cx| this.remove_selected(cx))),
                    )
                    .child(
                        Button::new("capture-close-detail")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .tooltip("Close the detail")
                            .on_click(cx.listener(|this, _, window, cx| this.select(None, window, cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}

impl Render for CaptureScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            "Capture",
            "Traffic through the proxy, newest first. The engine keeps the last 200 exchanges.",
            h_flex()
                .gap_2()
                .child(div().w_72().child(
                    Input::new(&self.filter).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ))
                .child(
                    Button::new("capture-pause")
                        .small()
                        .icon(if self.paused { IconName::Play } else { IconName::Pause })
                        .label(if self.paused { "Resume" } else { "Pause" })
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_paused(window, cx))),
                )
                .child(
                    Button::new("capture-clear")
                        .small()
                        .label("Clear")
                        .disabled(self.entries.is_empty())
                        .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                ),
            cx,
        );
        let detail = self.selected_entry().cloned().map(|entry| self.render_detail(&entry, cx));
        let has_detail = detail.is_some();
        // The split's size is a physical starting point the user resizes, not part of the rem scale.
        let body = v_resizable("capture-split").child(resizable_panel().child(self.render_list(cx))).child(
            resizable_panel().visible(has_detail).size(px(360.)).size_range(px(200.)..px(900.)).children(detail),
        );
        page::screen(header, body, cx)
    }
}
