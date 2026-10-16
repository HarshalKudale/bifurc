//! The Settings tab of an HTTP-like request: TLS verification, redirects and timeout, each either left to
//! the app-wide default (Settings > Requests) or set for this request alone.

use gpui_kit::component::{
    ActiveTheme as _, IndexPath, h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div,
};

use crate::{
    app_model::AppModel,
    i18n::{t, t_with},
    model::http::RequestSettings,
};

pub struct SettingsChanged;

type Choice = Entity<SelectState<Vec<&'static str>>>;

pub struct SettingsForm {
    model: Entity<AppModel>,
    verify_tls: Choice,
    follow_redirects: Choice,
    timeout: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsChanged> for SettingsForm {}

/// Default, On, Off: the labels in the order of [`choice_of`].
fn choices() -> Vec<&'static str> {
    vec![t("session.settings.default"), t("session.settings.on"), t("session.settings.off")]
}

fn choice_of(state: &Choice, cx: &gpui_kit::App) -> Option<bool> {
    let labels = choices();
    match state.read(cx).selected_value() {
        Some(label) if *label == labels[1] => Some(true),
        Some(label) if *label == labels[2] => Some(false),
        _ => None,
    }
}

fn select_choice(state: &Choice, value: Option<bool>, window: &mut Window, cx: &mut gpui_kit::App) {
    let label = choices()[match value {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    }];
    state.update(cx, |state, cx| state.set_selected_value(&label, window, cx));
}

impl SettingsForm {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let choice = |window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| SelectState::new(choices(), Some(IndexPath::default()), window, cx))
        };
        let (verify_tls, follow_redirects) = (choice(window, cx), choice(window, cx));
        let timeout = cx.new(|cx| InputState::new(window, cx).placeholder(t("session.settings.default")));
        let mut subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&timeout, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.emit(SettingsChanged);
                }
            }),
        ];
        for state in [&verify_tls, &follow_redirects] {
            subscriptions
                .push(cx.subscribe(state, |_, _, _: &SelectEvent<Vec<&'static str>>, cx| cx.emit(SettingsChanged)));
        }
        Self { model, verify_tls, follow_redirects, timeout, _subscriptions: subscriptions }
    }

    pub fn value(&self, cx: &gpui_kit::App) -> RequestSettings {
        RequestSettings {
            verify_tls: choice_of(&self.verify_tls, cx),
            follow_redirects: choice_of(&self.follow_redirects, cx),
            timeout_ms: self.timeout.read(cx).value().trim().parse().ok(),
        }
    }

    pub fn set(&mut self, settings: &RequestSettings, window: &mut Window, cx: &mut Context<Self>) {
        select_choice(&self.verify_tls, settings.verify_tls, window, cx);
        select_choice(&self.follow_redirects, settings.follow_redirects, window, cx);
        let timeout = settings.timeout_ms.map(|ms| ms.to_string()).unwrap_or_default();
        self.timeout.update(cx, |state, cx| state.set_value(timeout, window, cx));
    }
}

fn row(title: &'static str, hint: String, control: impl IntoElement, cx: &gpui_kit::App) -> impl IntoElement {
    h_flex()
        .gap_3()
        .items_center()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().child(title))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(SharedString::from(hint))),
        )
        .child(div().w_40().flex_none().child(control))
}

impl Render for SettingsForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let defaults = self.model.read(cx).config().request_defaults.clone();
        let on_off = |on: bool| if on { t("session.settings.on") } else { t("session.settings.off") };
        let timeout = match defaults.timeout_ms {
            0 => t("session.settings.no_timeout").to_owned(),
            ms => t_with("session.settings.timeout_value", &[("ms", &ms)]),
        };
        v_flex()
            .size_full()
            .p_3()
            .gap_3()
            .child(row(
                t("session.settings.verify_tls"),
                t_with("session.settings.default_is", &[("value", &on_off(defaults.verify_tls))]),
                Select::new(&self.verify_tls),
                cx,
            ))
            .child(row(
                t("session.settings.follow_redirects"),
                t_with("session.settings.default_is", &[("value", &on_off(defaults.follow_redirects))]),
                Select::new(&self.follow_redirects),
                cx,
            ))
            .child(row(
                t("session.settings.timeout"),
                t_with("session.settings.default_is", &[("value", &timeout)]),
                Input::new(&self.timeout),
                cx,
            ))
    }
}
