//! The Mail screen: the engine's local mail sink. An SMTP server on 127.0.0.1 keeps the last mails it was
//! handed in memory, so an app under development can send sign-up, one-time-code and reset mails without a
//! real provider.
//!
//! The sink belongs to the engine, not to a workspace. The list is read once (`mail.list`, which carries only a
//! summary of each mail) and extended by `event.mail.changed`; the body of one mail, with the code and link
//! the engine found in it, is fetched by id (`mail.get`) when it is selected. The engine keeps 200 mails, so a
//! mail that was selected can be gone by the time it is asked for.

use bifurc_engine_client::ClientError;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    list::ListItem,
    notification::Notification,
    tab::{Tab, TabBar},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, TestSupportExt as _, Window, div,
};
use serde_json::{Value, json};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        Syntax,
        dialogs::confirm_danger,
        master_detail::{list_rows, master_detail},
        page,
    },
    engine::{call, call_in, failure},
    i18n::{t, t_with},
    model::{git::date_time_utc, traffic::clock_utc},
};

/// The engine keeps this many mails; asking for that many is asking for all of them.
const KEPT: usize = 200;

/// What the list shows of a mail.
#[derive(Debug, Clone, PartialEq)]
struct Summary {
    id: String,
    /// Epoch milliseconds.
    timestamp: u64,
    from: String,
    to: Vec<String>,
    subject: String,
}

impl Summary {
    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            timestamp: value.get("timestamp").and_then(Value::as_i64).unwrap_or_default().max(0) as u64,
            from: text("from"),
            to: value
                .get("to")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|to| to.as_str().map(str::to_owned))
                .collect(),
            subject: text("subject"),
        })
    }

    fn recipients(&self) -> String {
        self.to.join(", ")
    }

    /// Whether the filter text, lower-cased, appears in the sender, a recipient or the subject.
    fn matches(&self, needle: &str) -> bool {
        needle.is_empty()
            || [&self.from, &self.subject, &self.recipients()].iter().any(|field| field.to_lowercase().contains(needle))
    }
}

/// What the sink says about itself (`mail.status`).
struct Sink {
    port: u64,
    running: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Text,
    Html,
}

const PARTS: [Part; 2] = [Part::Text, Part::Html];

/// A fetched mail.
struct Shown {
    mail: Summary,
    text: String,
    html: String,
    otp: Option<String>,
    link: Option<String>,
}

impl Shown {
    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str);
        Some(Self {
            mail: Summary::from_value(value)?,
            text: text("text").unwrap_or_default().to_owned(),
            html: text("html").unwrap_or_default().to_owned(),
            otp: text("otp").map(str::to_owned),
            link: text("link").map(str::to_owned),
        })
    }
}

enum Detail {
    Idle,
    Loading,
    /// The sink dropped the mail to make room for newer ones.
    Gone,
    Failed(String),
    Ready(Box<Shown>),
}

pub struct MailScreen {
    model: Entity<AppModel>,
    /// Newest first.
    mails: Vec<Summary>,
    sink: Option<Sink>,
    /// The sink has been read at least once.
    loaded: bool,
    filter: Entity<InputState>,
    selected: Option<String>,
    detail: Detail,
    part: Part,
    body: Entity<EditorState>,
    _subscriptions: Vec<Subscription>,
}

impl MailScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(t("mail.filter.placeholder")));
        let body = cx.new(|cx| {
            let mut state =
                EditorState::new(window, cx).language(Syntax::Plain.language()).line_number(false).soft_wrap(true);
            state.set_readonly(true, cx);
            state
        });
        let subscriptions = vec![
            cx.subscribe(&model, |this, _, event: &ModelEvent, cx| match event {
                ModelEvent::MailChanged(payload) => this.apply_change(payload, cx),
                // The screen is built before the engine is known; the first snapshot is when it can load. A
                // restarted engine has an empty inbox.
                ModelEvent::ConfigChanged if !this.loaded => this.refresh(cx),
                ModelEvent::EngineRestarted => this.refresh(cx),
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
            mails: Vec::new(),
            sink: None,
            loaded: false,
            filter,
            selected: None,
            detail: Detail::Idle,
            part: Part::Text,
            body,
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        this
    }

    /// One line for the footer.
    pub fn status_line(&self) -> String {
        let count = self.mails.len();
        t_with(if count == 1 { "mail.status.count_one" } else { "mail.status.count_other" }, &[("count", &count)])
    }

    /// Reads the sink's state and its mails again. The port can change in Settings, so this also runs when
    /// the screen is brought to the front.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "mail.status", json!({}), |this, outcome, cx| {
            if let Ok(status) = outcome {
                this.sink = Some(Sink {
                    port: status.get("port").and_then(Value::as_u64).unwrap_or_default(),
                    running: status.get("running").and_then(Value::as_bool).unwrap_or_default(),
                    error: status.get("error").and_then(Value::as_str).map(str::to_owned),
                });
                cx.notify();
            }
        });
        call(&client, cx, "mail.list", json!({ "last": KEPT }), |this, outcome, cx| {
            match outcome {
                Ok(result) => {
                    this.mails = result
                        .get("mails")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Summary::from_value)
                        .collect();
                    this.loaded = true;
                    if this.selected.as_ref().is_some_and(|id| !this.mails.iter().any(|mail| &mail.id == id)) {
                        this.selected = None;
                        this.detail = Detail::Idle;
                    }
                }
                Err(error) => {
                    let what = format!("{} {}", t("mail.error.read"), crate::engine::describe(&error));
                    this.model.update(cx, |model, cx| model.notice(what, cx));
                }
            }
            cx.notify();
        });
    }

    /// `event.mail.changed`: a mail arrived, or the inbox was emptied. The list follows without a refetch.
    fn apply_change(&mut self, payload: &Value, cx: &mut Context<Self>) {
        if let Some(mail) = payload.get("received").and_then(Summary::from_value) {
            // The same mail can come both from the list read and from the event.
            if !self.mails.iter().any(|known| known.id == mail.id) {
                self.mails.insert(0, mail);
                self.mails.truncate(KEPT);
            }
        } else if payload.get("cleared").is_some() {
            self.mails.clear();
            self.selected = None;
            self.detail = Detail::Idle;
        }
        cx.notify();
    }

    fn confirm_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t("mail.clear.title"),
            t("mail.clear.body"),
            t("mail.clear.confirm"),
            |this, _, cx| this.clear(cx),
        );
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "mail.clear", json!({}), |this, outcome, cx| match failure(&outcome) {
            Some(message) => {
                let what = format!("{} {message}", t("mail.error.clear"));
                this.model.update(cx, |model, cx| model.notice(what, cx));
            }
            None => this.apply_change(&json!({ "cleared": 0 }), cx),
        });
    }

    fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(id.clone());
        self.detail = Detail::Loading;
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "mail.get", json!({ "id": id }), move |this, outcome, window, cx| {
            // An answer for a mail that is no longer selected is dropped.
            if this.selected.as_deref() != Some(id.as_str()) {
                return;
            }
            if let Err(ClientError::Engine(error)) = &outcome
                && error.code == "NOT_FOUND"
            {
                this.detail = Detail::Gone;
                return cx.notify();
            }
            this.detail = match (failure(&outcome), outcome.ok().as_ref().and_then(Shown::from_value)) {
                (Some(message), _) => Detail::Failed(message),
                (None, Some(shown)) => {
                    // A mail with only an HTML part opens on it.
                    let part = if shown.text.is_empty() && !shown.html.is_empty() { Part::Html } else { Part::Text };
                    this.show_part(part, &shown, window, cx);
                    Detail::Ready(Box::new(shown))
                }
                (None, None) => Detail::Failed(t("mail.error.no_answer").to_owned()),
            };
            cx.notify();
        });
        cx.notify();
    }

    /// Puts one part of the mail in the read-only editor. HTML is shown as source: GPUI can't draw it.
    fn show_part(&mut self, part: Part, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) {
        self.part = part;
        let (text, syntax) = match part {
            Part::Text => (shown.text.clone(), Syntax::Plain),
            Part::Html => (shown.html.clone(), Syntax::Markup),
        };
        self.body.update(cx, |state, cx| {
            state.set_highlighter(syntax.language(), cx);
            state.set_value(text, window, cx);
        });
    }

    fn copy(text: String, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        window.push_notification(Notification::success(t("mail.copied")), cx);
    }

    fn render_row(&self, mail: &Summary, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let id = mail.id.clone();
        ListItem::new(gpui_kit::ElementId::Name(format!("mail-{}", mail.id).into()))
            .selected(self.selected.as_deref() == Some(mail.id.as_str()))
            .w_full()
            .px_3()
            .py_2()
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(if mail.subject.is_empty() {
                                        t("mail.no_subject").to_owned()
                                    } else {
                                        mail.subject.clone()
                                    }),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(clock_utc(mail.timestamp)),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(t_with("mail.row.route", &[("from", &mail.from), ("to", &mail.recipients())])),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.select(id.clone(), window, cx)))
            .into_any_element()
    }

    fn render_list(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        let rows: Vec<_> =
            self.mails.iter().filter(|mail| mail.matches(&needle)).map(|mail| self.render_row(mail, cx)).collect();
        if rows.is_empty() {
            return page::empty(t("mail.empty.no_matches.title"), t("mail.empty.no_matches.body"));
        }
        list_rows("mail-rows").children(rows).into_any_element()
    }

    fn pair(name: &'static str, value: String, cx: &Context<Self>) -> gpui_kit::AnyElement {
        h_flex()
            .gap_3()
            .items_start()
            .text_sm()
            .child(div().w_16().flex_none().text_color(cx.theme().muted_foreground).child(name))
            .child(div().flex_1().min_w_0().child(value))
            .into_any_element()
    }

    /// The one-time code the engine found, with a button to copy it.
    fn render_otp(&self, otp: &str, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let code = otp.to_owned();
        h_flex()
            .id("mail-otp")
            .test_support()
            .gap_3()
            .items_center()
            .child(div().w_16().flex_none().text_sm().text_color(cx.theme().muted_foreground).child(t("mail.otp")))
            .child(Tag::secondary().child(div().font_family(cx.theme().mono_font_family.clone()).child(code.clone())))
            .child(
                Button::new("mail-copy-otp")
                    .ghost()
                    .small()
                    .icon(IconName::Copy)
                    .label(t("mail.copy"))
                    .on_click(cx.listener(move |_, _, window, cx| Self::copy(code.clone(), window, cx))),
            )
            .into_any_element()
    }

    fn render_link(&self, link: &str, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let (copied, opened) = (link.to_owned(), link.to_owned());
        // A link comes out of a mail's text: only a web address is handed to the system to open.
        let web = link.starts_with("http://") || link.starts_with("https://");
        h_flex()
            .id("mail-link")
            .test_support()
            .gap_3()
            .items_center()
            .child(div().w_16().flex_none().text_sm().text_color(cx.theme().muted_foreground).child(t("mail.link")))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .font_family(cx.theme().mono_font_family.clone())
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(link.to_owned()),
            )
            .child(
                Button::new("mail-copy-link")
                    .ghost()
                    .small()
                    .icon(IconName::Copy)
                    .label(t("mail.copy"))
                    .on_click(cx.listener(move |_, _, window, cx| Self::copy(copied.clone(), window, cx))),
            )
            .child(
                Button::new("mail-open-link")
                    .ghost()
                    .small()
                    .icon(IconName::ExternalLink)
                    .label(t("mail.open"))
                    .disabled(!web)
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&opened))),
            )
            .into_any_element()
    }

    fn render_shown(&self, shown: &Shown, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let subject =
            if shown.mail.subject.is_empty() { t("mail.no_subject").to_owned() } else { shown.mail.subject.clone() };
        let copy_from = self.body.clone();
        v_flex()
            .size_full()
            .child(
                v_flex()
                    .flex_none()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().font_semibold().child(subject))
                    .child(Self::pair(t("mail.field.from"), shown.mail.from.clone(), cx))
                    .child(Self::pair(t("mail.field.to"), shown.mail.recipients(), cx))
                    .child(Self::pair(
                        t("mail.field.date"),
                        t_with("mail.utc", &[("time", &date_time_utc(shown.mail.timestamp))]),
                        cx,
                    ))
                    .children(shown.otp.as_deref().map(|otp| self.render_otp(otp, cx)))
                    .children(shown.link.as_deref().map(|link| self.render_link(link, cx))),
            )
            .child(
                div().id("mail-part-tabs").test_support().flex_none().px_4().child(
                    TabBar::new("mail-parts")
                        .underline()
                        .small()
                        .selected_index(PARTS.iter().position(|part| *part == self.part).unwrap_or(0))
                        .on_click(cx.listener(|this, ix: &usize, window, cx| {
                            // The mail is taken out for the call: showing a part needs `&mut self` too.
                            if let Detail::Ready(shown) = std::mem::replace(&mut this.detail, Detail::Idle) {
                                this.show_part(PARTS[*ix], &shown, window, cx);
                                this.detail = Detail::Ready(shown);
                            }
                            cx.notify();
                        }))
                        .child(Tab::new().label(t("mail.tab.text")))
                        .child(Tab::new().label(t("mail.tab.html"))),
                ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        h_flex().flex_none().px_4().py_2().child(div().flex_1()).child(
                            Button::new("mail-copy-body")
                                .ghost()
                                .small()
                                .icon(IconName::Copy)
                                .label(t("mail.copy"))
                                .on_click(cx.listener(move |_, _, window, cx| {
                                    Self::copy(copy_from.read(cx).value().to_string(), window, cx)
                                })),
                        ),
                    )
                    .child(div().flex_1().min_h_0().child(Editor::new(&self.body).bordered(false).h_full())),
            )
            .into_any_element()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match &self.detail {
            Detail::Idle => page::empty(t("mail.detail.none.title"), t("mail.detail.none.body")),
            Detail::Loading => page::loading(t("mail.loading")),
            Detail::Gone => page::empty(t("mail.detail.gone.title"), t("mail.detail.gone.body")),
            Detail::Failed(message) => div()
                .p_4()
                .child(
                    Alert::error("mail-load-failed", SharedString::from(message.clone()))
                        .title(t("mail.error.load_title")),
                )
                .into_any_element(),
            Detail::Ready(shown) => self.render_shown(shown, cx),
        }
    }
}

impl Render for MailScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let port = self.sink.as_ref().map_or(0, |sink| sink.port);
        let status = self.sink.as_ref().map(|sink| match sink.running {
            true => Tag::success().small().child(t("mail.tag.listening")),
            false => Tag::danger().small().child(t("mail.tag.stopped")),
        });
        let header = page::header(
            t("mail.page.title"),
            t_with("mail.page.subtitle", &[("port", &port)]),
            h_flex()
                .gap_2()
                .children(status)
                .child(div().id("mail-filter").test_support().w_72().child(
                    Input::new(&self.filter).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ))
                .child(
                    Button::new("mail-clear")
                        .small()
                        .label(t("mail.page.clear"))
                        .disabled(self.mails.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.confirm_clear(window, cx))),
                ),
            cx,
        );
        let error = self.sink.as_ref().and_then(|sink| sink.error.clone()).map(|message| {
            div().id("mail-error").test_support().flex_none().px_4().pt_3().child(
                Alert::error(
                    "mail-error-alert",
                    SharedString::from(format!("{message} {}", t("mail.error.change_port"))),
                )
                .title(t("mail.error.not_listening")),
            )
        });
        let body = if self.mails.is_empty() {
            page::empty(t("mail.empty.none.title"), t_with("mail.empty.none.body", &[("port", &port)]))
        } else {
            master_detail(Some(self.render_list(cx)), self.render_detail(cx), cx).into_any_element()
        };
        page::screen(header, v_flex().size_full().children(error).child(div().flex_1().min_h_0().child(body)), cx)
    }
}
