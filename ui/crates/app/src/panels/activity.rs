//! The activity panel: what the UI has done, and the traffic the engine's proxy has captured.
//!
//! Both are logs, newest first. Activity is this window's own record of sends, connections and failures;
//! Traffic is the engine's capture history (it is the engine's, so a window opened later still sees what
//! passed) extended live by `event.log.entry`.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    dock::{BasePanel, Panel, PanelEvent},
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
    h_flex,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::json;

use crate::{
    app_model::{AppModel, ModelEvent},
    engine::{call, describe},
    model::traffic::{self, TrafficEntry, clock_utc},
};

/// The activity log keeps this many lines; older ones fall off.
const MAX_ACTIVITY: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Activity,
    Traffic,
}

pub struct ActivityPanel {
    focus_handle: FocusHandle,
    model: Entity<AppModel>,
    section: Section,
    activity: Vec<String>,
    traffic: Vec<TrafficEntry>,
    _subscriptions: Vec<Subscription>,
}

impl ActivityPanel {
    pub fn new(model: Entity<AppModel>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&model, |this, _, event: &ModelEvent, cx| match event {
            ModelEvent::Traffic(payload) => {
                // A batch is oldest first; the list is newest first.
                let mut incoming = traffic::parse_event(payload);
                incoming.reverse();
                traffic::merge_live(&mut this.traffic, incoming);
                cx.notify();
            }
            ModelEvent::WorkspaceSwitched => this.reload_traffic(cx),
            ModelEvent::Notice(text) => this.push(text.clone(), cx),
            _ => {}
        });
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            model,
            section: Section::Activity,
            activity: Vec::new(),
            traffic: Vec::new(),
            _subscriptions: vec![subscription],
        };
        this.reload_traffic(cx);
        this
    }

    pub fn push(&mut self, line: String, cx: &mut Context<Self>) {
        self.activity.insert(0, line);
        self.activity.truncate(MAX_ACTIVITY);
        cx.notify();
    }

    fn reload_traffic(&mut self, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if model.workspace_id().is_empty() {
            return;
        }
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id() });
        call(&client, cx, "capture.list", payload, |this, outcome, cx| {
            match outcome {
                Ok(result) => this.traffic = traffic::parse_list(&result),
                Err(error) => this.push(format!("Couldn't read captured traffic. {}", describe(&error)), cx),
            }
            cx.notify();
        });
    }

    fn clear_traffic(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let payload = json!({ "workspaceId": self.model.read(cx).workspace_id() });
        call(&client, cx, "capture.clear", payload, |this, outcome, cx| match outcome {
            Ok(_) => {
                this.traffic.clear();
                cx.notify();
            }
            Err(error) => this.push(format!("Couldn't clear captured traffic. {}", describe(&error)), cx),
        });
    }

    fn render_activity(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        if self.activity.is_empty() {
            return empty("Nothing yet", "Sends, connections and failures are listed here.");
        }
        v_flex()
            .id("activity-lines")
            .size_full()
            .overflow_y_scroll()
            .p_2()
            .gap_1()
            .children(self.activity.iter().enumerate().map(|(ix, line)| {
                div().id(("activity-line", ix)).px_1().text_sm().text_color(theme.foreground).child(line.clone())
            }))
            .into_any_element()
    }

    fn render_traffic(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        if self.traffic.is_empty() {
            return empty("No traffic captured", "Requests that pass through the proxy are listed here.");
        }
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_3()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().w_20().flex_none().child("Time (UTC)"))
                    .child(div().w_16().flex_none().child("Method"))
                    .child(div().flex_1().min_w_0().child("URL"))
                    .child(div().w_16().flex_none().child("Status"))
                    .child(div().w_20().flex_none().text_right().child("Time taken")),
            )
            .child(v_flex().id("traffic-rows").flex_1().min_h_0().overflow_y_scroll().children(
                self.traffic.iter().map(|entry| {
                    let status = match entry.status {
                        Some(code) if (200..400).contains(&code) => Tag::success().child(code.to_string()),
                        Some(code) => Tag::danger().child(code.to_string()),
                        None => Tag::secondary().child("…"),
                    };
                    h_flex()
                        .id(gpui_kit::ElementId::Name(format!("traffic-{}", entry.id).into()))
                        .px_3()
                        .py_1()
                        .gap_3()
                        .child(
                            div()
                                .w_20()
                                .flex_none()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(clock_utc(entry.ts)),
                        )
                        .child(div().w_16().flex_none().child(entry.method.clone()))
                        .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(entry.url.clone()))
                        .child(div().w_16().flex_none().child(status))
                        .child(
                            div()
                                .w_20()
                                .flex_none()
                                .text_right()
                                .text_color(theme.muted_foreground)
                                .child(entry.duration_ms.map(|ms| format!("{ms} ms")).unwrap_or_default()),
                        )
                }),
            ))
            .into_any_element()
    }
}

fn empty(title: &'static str, description: &'static str) -> gpui_kit::AnyElement {
    Empty::new()
        .header(
            EmptyHeader::new()
                .title(EmptyTitle::new().child(title))
                .description(EmptyDescription::new().child(description)),
        )
        .into_any_element()
}

impl EventEmitter<PanelEvent> for ActivityPanel {}

impl Focusable for ActivityPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for ActivityPanel {
    fn panel_name(&self) -> &'static str {
        "ActivityPanel"
    }
}

impl Panel for ActivityPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "Activity"
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for ActivityPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = if self.section == Section::Activity { 0 } else { 1 };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .px_2()
                    .child(
                        div().flex_1().child(
                            TabBar::new("activity-sections")
                                .underline()
                                .small()
                                .selected_index(selected)
                                .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                    this.section = if *ix == 0 { Section::Activity } else { Section::Traffic };
                                    cx.notify();
                                }))
                                .child(Tab::new().label("Activity"))
                                .child(Tab::new().label(format!("Traffic ({})", self.traffic.len()))),
                        ),
                    )
                    .when(self.section == Section::Traffic, |this| {
                        this.child(
                            Button::new("clear-traffic")
                                .ghost()
                                .small()
                                .label("Clear")
                                .disabled(self.traffic.is_empty())
                                .on_click(cx.listener(|this, _, _, cx| this.clear_traffic(cx))),
                        )
                    }),
            )
            .child(div().flex_1().min_h_0().child(match self.section {
                Section::Activity => self.render_activity(cx),
                Section::Traffic => self.render_traffic(cx),
            }))
    }
}
