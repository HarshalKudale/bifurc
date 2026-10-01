//! The Capture screen: the traffic the proxy has seen, newest first, and one exchange in detail.
//!
//! The history is the engine's (a window opened later still sees what passed) and is extended live by
//! `event.log.entry`. Pausing only stops this view from following; the engine keeps capturing. An exchange
//! can be turned into a saved request or a mock: the engine builds the entity from the entry
//! (`capture.buildRequest`, `capture.buildMock`) and this screen saves what it built.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _,
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
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle, Window, div, px, uniform_list,
};
use serde_json::{Value, json};
use std::ops::Range;

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{Syntax, page},
    engine::{call, call_in, describe, failure},
    model::{
        http::pretty_json,
        traffic::{self, CapturedBody, TrafficEntry, clock_utc},
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Request,
    Response,
}

impl Side {
    fn headers_key(self) -> &'static str {
        match self {
            Side::Request => "reqHeaders",
            Side::Response => "resHeaders",
        }
    }

    fn body_key(self) -> &'static str {
        match self {
            Side::Request => "reqBody",
            Side::Response => "resBody",
        }
    }
}

pub struct CaptureScreen {
    model: Entity<AppModel>,
    entries: Vec<TrafficEntry>,
    /// The engine's history has been read at least once.
    loaded: bool,
    /// While paused the list stops following live traffic; the engine keeps capturing.
    paused: bool,
    filter: Entity<InputState>,
    selected: Option<String>,
    side: Side,
    body: Entity<EditorState>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl CaptureScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by URL, method, status"));
        let body = cx.new(|cx| {
            let mut state =
                EditorState::new(window, cx).language(Syntax::Plain.language()).line_number(true).soft_wrap(true);
            state.set_readonly(true, cx);
            state
        });
        let subscriptions = vec![
            cx.subscribe_in(&model, window, |this, _, event: &ModelEvent, window, cx| match event {
                ModelEvent::Traffic(payload) if !this.paused => {
                    // A batch is oldest first; the list is newest first.
                    traffic::merge_live(&mut this.entries, traffic::parse_event(payload));
                    cx.notify();
                }
                ModelEvent::WorkspaceSwitched => {
                    this.selected = None;
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
            side: Side::Response,
            body,
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
        let payload = json!({ "workspaceId": model.workspace_id() });
        call(&client, cx, "capture.list", payload, |this, outcome, cx| {
            match outcome {
                Ok(result) => {
                    this.entries = traffic::parse_list(&result);
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

    fn toggle_paused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.paused = !self.paused;
        if !self.paused {
            // Catch up on what passed while the list was not following.
            self.reload(window, cx);
        }
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id() });
        call(&client, cx, "capture.clear", payload, |this, outcome, cx| match failure(&outcome) {
            Some(message) => this.report("Couldn't clear captured traffic.", message, cx),
            None => {
                this.entries.clear();
                this.selected = None;
                cx.notify();
            }
        });
    }

    fn selected_entry(&self) -> Option<&TrafficEntry> {
        let id = self.selected.as_deref()?;
        self.entries.iter().find(|entry| entry.id == id)
    }

    fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = id;
        self.show_body(window, cx);
        cx.notify();
    }

    /// Puts the selected exchange's body, for the side in view, into the read-only editor.
    fn show_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry() else { return };
        let content_type = entry
            .headers(self.side.headers_key())
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value)
            .unwrap_or_default();
        let (text, syntax) = match entry.body(self.side.body_key()) {
            CapturedBody::Text(text) => match pretty_json(&text) {
                Some(pretty) => (pretty, Syntax::Json),
                None => (text, Syntax::for_content_type(&content_type)),
            },
            CapturedBody::Empty | CapturedBody::Binary { .. } => (String::new(), Syntax::Plain),
        };
        self.body.update(cx, |state, cx| {
            state.set_highlighter(syntax.language(), cx);
            state.set_value(text, window, cx);
        });
    }

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.take() else { return };
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

    /// Saves the selected exchange as a request or as a mock. The engine builds the entity from the entry;
    /// what it built is created as it is.
    fn save_as(&mut self, mock: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_entry().map(|entry| entry.raw.clone()) else { return };
        let (build, field, kind, done) = if mock {
            ("capture.buildMock", "mock", "mocks", "Saved as a mock. Find it on the Mocks screen.")
        } else {
            ("capture.buildRequest", "request", "requests", "Saved as a request. Find it on the Requests screen.")
        };
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace_id = model.workspace_id().to_owned();
        let create_client = client.clone();
        call_in(&client, window, cx, build, json!({ "entry": entry }), move |this, outcome, window, cx| {
            let entity = match outcome {
                Ok(result) => result.get(field).cloned().filter(Value::is_object),
                Err(error) => return this.report("Couldn't build it from this exchange.", describe(&error), cx),
            };
            let Some(entity) = entity else {
                return this.report("Couldn't build it from this exchange.", "The engine returned nothing.".into(), cx);
            };
            let payload = json!({ "kind": kind, "workspaceId": workspace_id, "entity": entity });
            call_in(
                &create_client,
                window,
                cx,
                "entity.create",
                payload,
                move |this, outcome, window, cx| match failure(&outcome) {
                    Some(message) => this.report("Couldn't save it.", message, cx),
                    None => window.push_notification(Notification::success(done), cx),
                },
            );
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
                    "The list is paused. Resume to follow traffic through the proxy."
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
                                    .gap_3()
                                    .child(
                                        div()
                                            .w_16()
                                            .flex_none()
                                            .text_color(theme.muted_foreground)
                                            .child(clock_utc(entry.ts)),
                                    )
                                    .child(div().w_16().flex_none().child(entry.method.clone()))
                                    .child(h_flex().w_16().flex_none().child(Self::status_tag(entry.status)))
                                    .child(
                                        div()
                                            .w_20()
                                            .flex_none()
                                            .text_color(theme.muted_foreground)
                                            .child(entry.via_label()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(entry.url.clone()),
                                    )
                                    .child(
                                        div()
                                            .w_20()
                                            .flex_none()
                                            .text_right()
                                            .text_color(theme.muted_foreground)
                                            .child(entry.duration_ms.map(|ms| format!("{ms} ms")).unwrap_or_default()),
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
                    .child(div().w_16().flex_none().child("Time (UTC)"))
                    .child(div().w_16().flex_none().child("Method"))
                    .child(div().w_16().flex_none().child("Status"))
                    .child(div().w_20().flex_none().child("Handled by"))
                    .child(div().flex_1().child("URL"))
                    .child(div().w_20().flex_none().text_right().child("Time taken")),
            )
            .child(rows)
            .into_any_element()
    }

    fn render_detail(&self, entry: &TrafficEntry, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let headers = entry.headers(self.side.headers_key());
        let body = match entry.body(self.side.body_key()) {
            CapturedBody::Text(_) => Editor::new(&self.body).bordered(false).h_full().into_any_element(),
            CapturedBody::Empty => page::empty("No body", "This side of the exchange carried no body."),
            CapturedBody::Binary { bytes } => {
                page::empty("Binary body", format!("{bytes} bytes of data that can't be shown as text."))
            }
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
                            .on_click(cx.listener(|this, _, window, cx| this.save_as(false, window, cx))),
                    )
                    .child(
                        Button::new("capture-save-mock")
                            .small()
                            .label("Save as mock")
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
            .child(
                div().flex_none().px_4().child(
                    TabBar::new("capture-sides")
                        .underline()
                        .small()
                        .selected_index(if self.side == Side::Request { 0 } else { 1 })
                        .on_click(cx.listener(|this, ix: &usize, window, cx| {
                            this.side = if *ix == 0 { Side::Request } else { Side::Response };
                            this.show_body(window, cx);
                            cx.notify();
                        }))
                        .child(Tab::new().label("Request"))
                        .child(Tab::new().label("Response")),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(
                        v_flex()
                            .id("capture-headers")
                            .w_2_5()
                            .flex_none()
                            .overflow_y_scroll()
                            .p_4()
                            .gap_2()
                            .border_r_1()
                            .border_color(theme.border)
                            .child(page::section_label("Headers", cx))
                            .children(headers.is_empty().then(|| {
                                div().text_sm().text_color(theme.muted_foreground).child("No headers were captured.")
                            }))
                            .children(headers.into_iter().map(|(name, value)| {
                                v_flex()
                                    .text_sm()
                                    .child(div().text_color(theme.muted_foreground).child(name))
                                    .child(div().child(value))
                            })),
                    )
                    .child(div().flex_1().min_w_0().child(body)),
            )
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
