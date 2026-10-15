//! The Audit log screen: the workspace's git history, one row per change.
//!
//! The engine does the history walking, filtering and paging; this screen sends the filters it shows and
//! renders the page that comes back. A row expands to the diff of what its commit changed, which the engine
//! writes on demand, and the page is read again whenever the engine says the history moved.

use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Value, json};

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{binary::save_to_file, commit_diffs::CommitDiffs, page},
    engine::{call, describe, failure, fetch_artifact},
    i18n::{t, t_with},
};

const PAGE_SIZE: usize = 100;
const MS_PER_DAY: i64 = 86_400_000;

const ENTITIES: [(&str, &str); 8] = [
    ("mock", "audit.entity.mock"),
    ("mapping", "audit.entity.mapping"),
    ("rule", "audit.entity.proxy_rule"),
    ("environment", "audit.entity.environment"),
    ("request", "audit.entity.request"),
    ("wsConnection", "audit.entity.websocket"),
    ("folder", "audit.entity.folder"),
    ("workspace", "audit.entity.workspace"),
];

const ACTIONS: [(&str, &str); 3] =
    [("create", "audit.action.created"), ("update", "audit.action.updated"), ("delete", "audit.action.deleted")];

/// Days since 1970-01-01 of a civil date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The civil date (year, month, day) of a count of days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

/// Epoch milliseconds (UTC) of a `YYYY-MM-DD` date: its first millisecond, or its last one with `end_of_day`.
/// Anything that is not a real date is no bound.
fn parse_date(text: &str, end_of_day: bool) -> Option<i64> {
    let mut parts = text.trim().split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    let all_digits = [year, month, day].iter().all(|part| part.bytes().all(|b| b.is_ascii_digit()));
    if !all_digits {
        return None;
    }
    let (year, month, day): (i64, i64, i64) = (year.parse().ok()?, month.parse().ok()?, day.parse().ok()?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let start = days_from_civil(year, month, day) * MS_PER_DAY;
    Some(if end_of_day { start + MS_PER_DAY - 1 } else { start })
}

/// `2026-10-02 14:03 UTC`.
fn absolute_time(ts: i64) -> String {
    let days = ts.div_euclid(MS_PER_DAY);
    let minutes = ts.rem_euclid(MS_PER_DAY) / 60_000;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02} UTC", minutes / 60, minutes % 60)
}

/// How long before `now` a change was made, in the largest whole unit; a date once it is a month old.
pub(crate) fn relative_time(now: i64, ts: i64) -> String {
    let seconds = (now - ts).div_euclid(1000);
    match seconds {
        ..60 => t("audit.time.just_now").to_owned(),
        60..3600 => t_with("audit.time.minutes_ago", &[("count", &(seconds / 60))]),
        3600..86_400 => t_with("audit.time.hours_ago", &[("count", &(seconds / 3600))]),
        86_400..2_592_000 => t_with("audit.time.days_ago", &[("count", &(seconds / 86_400))]),
        _ => absolute_time(ts)[..10].to_owned(),
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// `1–100 of 230`.
fn page_label(offset: usize, shown: usize, total: usize) -> String {
    t_with(
        "audit.footer.page_label",
        &[("from", &(offset + 1)), ("to", &(offset + shown)), ("total", &total)],
    )
}

fn entity_label(entity: &str) -> &'static str {
    match entity {
        "mock" => t("audit.tag.mock"),
        "mapping" => t("audit.tag.map"),
        "rule" => t("audit.tag.rule"),
        "environment" => t("audit.tag.env"),
        "request" => t("audit.tag.req"),
        "wsConnection" => t("audit.tag.ws"),
        "folder" => t("audit.tag.folder"),
        "workspace" => t("audit.tag.ws"),
        // A commit of several files, or one made outside the app: it is about no one item.
        "" => t("audit.tag.commit"),
        _ => "?",
    }
}

fn action_label(action: &str) -> &'static str {
    match action {
        "create" => t("audit.action.created"),
        "update" => t("audit.action.updated"),
        "delete" => t("audit.action.deleted"),
        _ => t("audit.action.changed"),
    }
}

/// The changed fields as one line: the first three, then how many more.
fn fields_summary(fields: &[String]) -> String {
    let mut line = fields.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if fields.len() > 3 {
        line.push_str(&format!(" +{}", fields.len() - 3));
    }
    line
}

/// One history row as the engine reports it.
#[derive(Clone)]
struct Entry {
    commit: String,
    ts: i64,
    action: String,
    entity: String,
    entity_name: String,
    /// The commit's subject, when the user wrote it.
    message: String,
    actor: String,
    fields: Vec<String>,
}

impl Entry {
    fn parse(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        Some(Self {
            commit: value.get("commitHash")?.as_str()?.to_owned(),
            ts: value.get("ts").and_then(Value::as_i64).unwrap_or_default(),
            action: text("action"),
            entity: text("entity"),
            entity_name: text("entityName"),
            message: text("message"),
            actor: text("actor"),
            fields: value
                .get("changedFields")
                .and_then(Value::as_array)
                .map(|fields| fields.iter().filter_map(|f| f.as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
        })
    }
}

pub struct AuditScreen {
    model: Entity<AppModel>,
    search: Entity<InputState>,
    from: Entity<InputState>,
    to: Entity<InputState>,
    entity: Option<&'static str>,
    action: Option<&'static str>,
    entries: Vec<Entry>,
    total: usize,
    offset: usize,
    loaded: bool,
    /// The workspace the list on screen belongs to.
    loaded_for: String,
    /// Bumped by every load; an answer carrying an older number is stale.
    generation: u64,
    expanded: HashSet<String>,
    diffs: Entity<CommitDiffs>,
    exporting: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl AuditScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("audit.filter.search_placeholder")));
        let from = cx.new(|cx| InputState::new(window, cx).placeholder(t("audit.filter.from_placeholder")));
        let to = cx.new(|cx| InputState::new(window, cx).placeholder(t("audit.filter.to_placeholder")));
        let mut subscriptions = vec![cx.subscribe(&model, |this, model, event: &ModelEvent, cx| match event {
            ModelEvent::WorkspaceSwitched => this.reload(0, cx),
            ModelEvent::ConfigChanged if model.read(cx).workspace_id() != this.loaded_for => this.reload(0, cx),
            // A commit was made or pulled: the page is read again, and what is open stays open.
            ModelEvent::HistoryChanged => this.fetch(this.offset, cx),
            _ => {}
        })];
        let diffs = cx.new(|_| CommitDiffs::new(model.clone()));
        subscriptions.push(cx.observe(&diffs, |_, _, cx| cx.notify()));
        for input in [&search, &from, &to] {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.reload(0, cx);
                }
            }));
        }
        let mut screen = Self {
            model,
            search,
            from,
            to,
            entity: None,
            action: None,
            entries: Vec::new(),
            total: 0,
            offset: 0,
            loaded: false,
            loaded_for: String::new(),
            generation: 0,
            expanded: HashSet::new(),
            diffs,
            exporting: None,
            _subscriptions: subscriptions,
        };
        screen.reload(0, cx);
        screen
    }

    fn report(&mut self, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(message, cx));
    }

    fn any_filter(&self, cx: &Context<Self>) -> bool {
        self.entity.is_some()
            || self.action.is_some()
            || !self.search.read(cx).value().trim().is_empty()
            || !self.from.read(cx).value().trim().is_empty()
            || !self.to.read(cx).value().trim().is_empty()
    }

    /// Shows the page at `offset` under the current filters, with every change closed.
    fn reload(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.fetch(offset, cx);
    }

    /// Asks the engine for the page at `offset` under the current filters.
    fn fetch(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace = model.workspace_id().to_owned();
        if workspace != self.loaded_for {
            self.diffs.update(cx, |diffs, _| diffs.clear());
        }

        let mut payload = json!({ "workspaceId": workspace, "limit": PAGE_SIZE, "offset": offset });
        if let Some(entity) = self.entity {
            payload["entity"] = json!(entity);
        }
        if let Some(action) = self.action {
            payload["action"] = json!(action);
        }
        let search = self.search.read(cx).value().trim().to_owned();
        if !search.is_empty() {
            payload["search"] = json!(search);
        }
        if let Some(from) = parse_date(&self.from.read(cx).value(), false) {
            payload["fromTs"] = json!(from);
        }
        if let Some(to) = parse_date(&self.to.read(cx).value(), true) {
            payload["toTs"] = json!(to);
        }
        cx.notify();
        call(&client, cx, "audit.list", payload, move |this, outcome, cx| {
            if this.generation != generation || this.model.read(cx).workspace_id() != workspace {
                return;
            }
            this.loaded = true;
            this.loaded_for = workspace;
            if let Some(message) = failure(&outcome) {
                this.report(t_with("audit.error.load", &[("message", &message)]), cx);
                cx.notify();
                return;
            }
            if let Ok(result) = outcome {
                this.entries = result
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|entries| entries.iter().filter_map(Entry::parse).collect())
                    .unwrap_or_default();
                this.total = result.get("total").and_then(Value::as_u64).unwrap_or_default() as usize;
                this.offset = offset;
            }
            cx.notify();
        });
    }

    fn set_entity(&mut self, entity: Option<&'static str>, cx: &mut Context<Self>) {
        self.entity = entity;
        self.reload(0, cx);
    }

    fn set_action(&mut self, action: Option<&'static str>, cx: &mut Context<Self>) {
        self.action = action;
        self.reload(0, cx);
    }

    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.entity = None;
        self.action = None;
        for input in [&self.search, &self.from, &self.to] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
        self.reload(0, cx);
    }

    /// Opens or closes a change; opening one loads what it changed the first time.
    fn toggle(&mut self, commit: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(commit) {
            self.expanded.insert(commit.to_owned());
            self.diffs.update(cx, |diffs, cx| diffs.load(commit, cx));
        }
        cx.notify();
    }

    fn export(&mut self, format: &'static str, cx: &mut Context<Self>) {
        if self.exporting.is_some() {
            return;
        }
        self.exporting = Some(format);
        cx.notify();
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace = model.workspace_id().to_owned();
        let payload = json!({ "format": format, "workspaceId": workspace });
        call(&client.clone(), cx, "audit.export", payload, move |this, outcome, cx| match outcome {
            Ok(artifact) => fetch_artifact(this, &client, cx, &workspace, &artifact, |this, fetched, cx| {
                this.finish_export(fetched, cx)
            }),
            Err(error) => this.finish_export(Err(describe(&error)), cx),
        });
    }

    /// Ends an export: asks where to save it, or says why there is nothing to save.
    fn finish_export(&mut self, result: Result<(Vec<u8>, String), String>, cx: &mut Context<Self>) {
        match result {
            Ok((bytes, name)) => {
                self.exporting = None;
                save_to_file(bytes, name, cx, |this, outcome, cx| match outcome {
                    Ok(path) => this.report(t_with("audit.export.saved_to", &[("path", &path.display())]), cx),
                    Err(message) => this.report(t_with("audit.error.save", &[("message", &message)]), cx),
                });
            }
            Err(message) => {
                self.exporting = None;
                self.report(t_with("audit.error.export", &[("message", &message)]), cx);
            }
        }
        cx.notify();
    }

    fn filter_menu(
        &self,
        id: &'static str,
        label: String,
        all: &'static str,
        options: &'static [(&'static str, &'static str)],
        current: Option<&'static str>,
        cx: &mut Context<Self>,
        select: fn(&mut Self, Option<&'static str>, &mut Context<Self>),
    ) -> impl IntoElement {
        let this = cx.weak_entity();
        Button::new(id).outline().small().label(label).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
            let all_view = this.clone();
            let menu = menu.item(
                PopupMenuItem::new(t(all))
                    .checked(current.is_none())
                    .on_click(move |_, _, cx| drop(all_view.update(cx, |this, cx| select(this, None, cx)))),
            );
            options.iter().fold(menu, |menu, &(value, label)| {
                let view = this.clone();
                menu.item(
                    PopupMenuItem::new(t(label))
                        .checked(current == Some(value))
                        .on_click(move |_, _, cx| drop(view.update(cx, |this, cx| select(this, Some(value), cx)))),
                )
            })
        })
    }

    fn render_filters(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let entity_label = self
            .entity
            .and_then(|e| ENTITIES.iter().find(|(value, _)| *value == e))
            .map_or(t("audit.filter.all_items").to_owned(), |(_, label)| t(label).to_owned());
        let action_label = self
            .action
            .and_then(|a| ACTIONS.iter().find(|(value, _)| *value == a))
            .map_or(t("audit.filter.all_actions").to_owned(), |(_, label)| t(label).to_owned());
        h_flex()
            .flex_none()
            .flex_wrap()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(self.filter_menu(
                "audit-entity",
                entity_label,
                "audit.filter.all_items",
                &ENTITIES,
                self.entity,
                cx,
                Self::set_entity,
            ))
            .child(self.filter_menu(
                "audit-action",
                action_label,
                "audit.filter.all_actions",
                &ACTIONS,
                self.action,
                cx,
                Self::set_action,
            ))
            .child(
                div().w_56().child(
                    Input::new(&self.search).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ),
            )
            .child(div().w_40().child(Input::new(&self.from).small()))
            .child(div().text_color(theme.muted_foreground).child("–"))
            .child(div().w_40().child(Input::new(&self.to).small()))
            .when_some(self.any_filter(cx).then_some(()), |bar, _| {
                bar.child(
                    Button::new("audit-clear")
                        .ghost()
                        .small()
                        .label(t("audit.filter.clear"))
                        .on_click(cx.listener(|this, _, window, cx| this.clear_filters(window, cx))),
                )
            })
            .into_any_element()
    }

    fn render_detail(&self, entry: &Entry, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let header = div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .font_family(theme.mono_font_family.clone())
            .child(t_with(
                "audit.commit_line",
                &[("commit", &entry.commit), ("actor", &entry.actor), ("time", &absolute_time(entry.ts))],
            ));
        let body = CommitDiffs::render(&self.diffs, &entry.commit, cx);
        v_flex()
            .w_full()
            .gap_2()
            .px_4()
            .pb_3()
            .pl_10()
            .border_b_1()
            .border_color(theme.border)
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_row(&self, entry: &Entry, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let open = self.expanded.contains(&entry.commit);
        let tag = match entry.action.as_str() {
            "create" => Tag::success(),
            "update" => Tag::warning(),
            _ => Tag::danger(),
        };
        let icon = if open { IconName::ChevronDown } else { IconName::ChevronRight };
        let chevron = div().size_4().flex_none().child(Icon::new(icon).small().text_color(theme.muted_foreground));
        let clicked = entry.commit.clone();
        // A message the user gave the commit leads; what it was about follows it.
        let (title, about) = if entry.message.is_empty() {
            (entry.entity_name.clone(), String::new())
        } else {
            (entry.message.clone(), entry.entity_name.clone())
        };
        let row = h_flex()
            .id(gpui_kit::ElementId::Name(format!("audit-row-{}", entry.commit).into()))
            .w_full()
            .gap_3()
            .px_4()
            .py_2()
            .text_sm()
            .items_center()
            .child(chevron)
            .child(div().w_20().flex_none().child(tag.small().child(action_label(&entry.action))))
            .child(div().w_16().flex_none().child(Tag::secondary().small().child(entity_label(&entry.entity))))
            .child(
                div()
                    .w_64()
                    .flex_none()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child(fields_summary(&entry.fields)),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(div().min_w_0().overflow_hidden().text_ellipsis().child(title))
                    .child(div().flex_none().text_color(theme.muted_foreground).child(about)),
            )
            .child(
                div()
                    .w_32()
                    .flex_none()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(theme.muted_foreground)
                    .child(entry.actor.clone()),
            )
            .child(
                div()
                    .w_24()
                    .flex_none()
                    .text_right()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(relative_time(now, entry.ts)),
            );
        let row = row
            .cursor_pointer()
            .hover(|style| style.bg(theme.muted))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(&clicked, cx)));
        let mut block = v_flex().w_full().border_b_1().border_color(theme.border).child(row);
        if open {
            block = block.child(self.render_detail(entry, cx));
        }
        block.into_any_element()
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (offset, total) = (self.offset, self.total);
        let shown = self.entries.len();
        h_flex()
            .flex_none()
            .justify_between()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(page_label(offset, shown, total))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("audit-prev")
                            .small()
                            .icon(IconName::ChevronLeft)
                            .label(t("audit.footer.prev"))
                            .disabled(offset == 0)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.reload(offset.saturating_sub(PAGE_SIZE), cx)),
                            ),
                    )
                    .child(
                        Button::new("audit-next")
                            .small()
                            .icon(IconName::ChevronRight)
                            .label(t("audit.footer.next"))
                            .disabled(offset + PAGE_SIZE >= total)
                            .on_click(cx.listener(move |this, _, _, cx| this.reload(offset + PAGE_SIZE, cx))),
                    ),
            )
            .into_any_element()
    }
}

impl Render for AuditScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let exporting = self.exporting;
        let export_button = |id: &'static str, format: &'static str, label: &'static str, cx: &mut Context<Self>| {
            Button::new(id)
                .small()
                .icon(IconName::FileText)
                .label(label)
                .loading(exporting == Some(format))
                .disabled(exporting.is_some())
                .on_click(cx.listener(move |this, _, _, cx| this.export(format, cx)))
        };
        let actions = h_flex()
            .gap_2()
            .child(export_button("audit-export-json", "json", "JSON", cx))
            .child(export_button("audit-export-csv", "csv", "CSV", cx))
            .child(
                Button::new("audit-refresh").small().icon(IconName::RefreshCw).tooltip(t("audit.page.reload_tooltip")).on_click(
                    cx.listener(|this, _, _, cx| {
                        let offset = this.offset;
                        this.reload(offset, cx);
                    }),
                ),
            );
        let subtitle = if self.total == 1 {
            t_with("audit.page.subtitle_one", &[("count", &self.total)])
        } else {
            t_with("audit.page.subtitle_other", &[("count", &self.total)])
        };
        let header = page::header(t("audit.page.title"), subtitle, actions, cx);

        let body: AnyElement = if !self.loaded {
            page::loading(t("audit.page.loading"))
        } else {
            let list: AnyElement = if self.entries.is_empty() {
                if self.any_filter(cx) {
                    page::empty(t("audit.empty.no_matches.title"), t("audit.empty.no_matches.body"))
                } else {
                    page::empty(t("audit.empty.no_history.title"), t("audit.empty.no_history.body"))
                }
            } else {
                let now = now_ms();
                let rows: Vec<AnyElement> = self.entries.iter().map(|entry| self.render_row(entry, now, cx)).collect();
                v_flex().id("audit-rows").size_full().overflow_y_scroll().children(rows).into_any_element()
            };
            let mut column =
                v_flex().size_full().child(self.render_filters(cx)).child(div().flex_1().min_h_0().child(list));
            if self.total > PAGE_SIZE {
                column = column.child(self.render_footer(cx));
            }
            column.into_any_element()
        };
        page::screen(header, body, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_becomes_the_start_or_end_of_that_utc_day() {
        assert_eq!(parse_date("1970-01-02", false), Some(MS_PER_DAY));
        assert_eq!(parse_date("2026-10-02", false), Some(1_790_899_200_000));
        assert_eq!(parse_date("2026-10-02", true), Some(1_790_899_200_000 + MS_PER_DAY - 1));
        assert_eq!(parse_date(" 2024-02-29 ", false), Some(1_709_164_800_000));
    }

    #[test]
    fn text_that_is_not_a_date_is_no_bound() {
        for text in [
            "",
            "2026-10",
            "2026-13-01",
            "2026-02-30",
            "2023-02-29",
            "26-10-02",
            "2026/10/02",
            "2026-10-02-1",
            "abcd-10-02",
        ] {
            assert_eq!(parse_date(text, false), None, "{text}");
        }
    }

    #[test]
    fn civil_dates_round_trip_through_day_counts() {
        for days in [-800_000, -1, 0, 59, 60, 11_016, 20_000, 400_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn an_absolute_time_reads_as_utc_to_the_minute() {
        assert_eq!(absolute_time(0), "1970-01-01 00:00 UTC");
        assert_eq!(absolute_time(1_790_899_200_000 + (14 * 60 + 3) * 60_000 + 59_999), "2026-10-02 14:03 UTC");
    }

    #[test]
    fn a_relative_time_uses_the_largest_whole_unit() {
        let now = 1_790_899_200_000;
        assert_eq!(relative_time(now, now + 5_000), "just now");
        assert_eq!(relative_time(now, now - 59_000), "just now");
        assert_eq!(relative_time(now, now - 5 * 60_000), "5m ago");
        assert_eq!(relative_time(now, now - 2 * 3_600_000), "2h ago");
        assert_eq!(relative_time(now, now - 3 * MS_PER_DAY), "3d ago");
        assert_eq!(relative_time(now, now - 40 * MS_PER_DAY), "2026-08-23");
    }

    #[test]
    fn the_page_label_counts_from_one() {
        assert_eq!(page_label(0, 100, 230), "1–100 of 230");
        assert_eq!(page_label(200, 30, 230), "201–230 of 230");
    }

    #[test]
    fn changed_fields_are_summarised_to_three_and_a_count() {
        let fields = |n: usize| (1..=n).map(|i| format!("f{i}")).collect::<Vec<_>>();
        assert_eq!(fields_summary(&fields(2)), "f1, f2");
        assert_eq!(fields_summary(&fields(5)), "f1, f2, f3 +2");
        assert_eq!(fields_summary(&[]), "");
    }
}
