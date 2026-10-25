//! The Services screen: the workspace's dev servers, and what is listening on this machine with which of it
//! has a `.localhost` name.
//!
//! The engine runs the dev servers (`devservice.*`) and finds the listeners (`services.discover`); this screen
//! lists the dev servers with their state, and the listeners beside the mapping that points at each port,
//! and offers to map the ones that have none or to stop what holds a port.

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, ElementId, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    UniformListScrollHandle, Window, div, uniform_list,
};
use serde_json::{Value, json};
use std::ops::Range;

use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        dialogs::{PromptField, confirm_danger, prompt},
        page,
    },
    engine::{call, call_in, describe, failure},
    i18n::{t, t_with},
    model::{logs::RunStatus, vault::VaultState},
};

/// What the screen asks the window for.
pub enum ServicesEvent {
    /// Show the output of dev server `name` in the terminal dock. `fresh` is for a server just started: what the
    /// dock holds of it is of the run before.
    ShowOutput { name: String, status: RunStatus, fresh: bool },
}

#[derive(Debug, Clone, PartialEq)]
struct Service {
    port: u16,
    address: String,
    pid: Option<u64>,
    process: String,
}

impl Service {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            port: u16::try_from(value.get("port")?.as_u64()?).ok()?,
            address: value.get("address").and_then(Value::as_str).unwrap_or_default().to_owned(),
            pid: value.get("pid").and_then(Value::as_u64),
            // Older engines called the process `name`.
            process: value
                .get("processName")
                .or_else(|| value.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        })
    }
}

/// The port a mapping's target points at: `localhost:3000`, `http://127.0.0.1:3000/x`, `3000`.
fn target_port(target: &str) -> Option<u16> {
    let without_scheme = target.split("://").last().unwrap_or(target);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    authority.rsplit(':').next().and_then(|port| port.trim().parse().ok())
}

#[derive(Debug, Clone, PartialEq)]
enum DevState {
    /// `listening` is only known for a service with a port.
    Running {
        listening: bool,
    },
    Stopped,
    Crashed(Option<i64>),
}

/// A dev server as `devservice.list` shows it: its definition and how its latest run is going.
#[derive(Debug, Clone, PartialEq)]
struct DevServer {
    name: String,
    cmd: String,
    port: Option<u16>,
    cwd: String,
    vault: String,
    log_file: String,
    /// The `.localhost` name the engine points at the port the server opens.
    mapping: String,
    /// The port the engine found the server's processes on, once it pointed the mapping there.
    mapped_port: Option<u16>,
    /// Kept so an edit can send it back: `devservice.define` drops every key it is not given.
    env: Option<Value>,
    state: DevState,
}

impl DevServer {
    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        Some(Self {
            name: value.get("name")?.as_str()?.to_owned(),
            cmd: text("cmd"),
            port: value.get("port").and_then(Value::as_u64).and_then(|port| u16::try_from(port).ok()),
            cwd: text("cwd"),
            vault: text("vault"),
            log_file: text("logFile"),
            mapping: text("mapping"),
            mapped_port: value.get("mappedPort").and_then(Value::as_u64).and_then(|port| u16::try_from(port).ok()),
            env: value.get("env").filter(|env| !env.is_null()).cloned(),
            state: match value.get("state").and_then(Value::as_str) {
                Some("running") => {
                    DevState::Running { listening: value.get("listening").and_then(Value::as_bool).unwrap_or(false) }
                }
                Some("crashed") => DevState::Crashed(value.get("exitCode").and_then(Value::as_i64)),
                _ => DevState::Stopped,
            },
        })
    }

    fn running(&self) -> bool {
        matches!(self.state, DevState::Running { .. })
    }

    fn run_status(&self) -> RunStatus {
        match self.state {
            DevState::Running { .. } => RunStatus::Running,
            DevState::Stopped => RunStatus::Stopped,
            DevState::Crashed(code) => RunStatus::Crashed(code),
        }
    }

    /// Started, but nothing answers on its port yet.
    fn starting(&self) -> bool {
        self.port.is_some() && self.state == (DevState::Running { listening: false })
    }
}

/// The `devservice.define` payload for what the dialog collected. `Err` is the key of what is wrong with it.
fn dev_definition(workspace: &str, name: &str, fields: &[String], env: Option<&Value>) -> Result<Value, &'static str> {
    let [cmd, port, cwd, vault, log_file, mapping] = fields else { return Err("services.dev.invalid.name") };
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("services.dev.invalid.name");
    }
    let mut definition = json!({ "workspaceId": workspace, "name": name, "cmd": cmd });
    if !port.is_empty() {
        let port: u16 = port.parse().ok().filter(|port| *port > 0).ok_or("services.dev.invalid.port")?;
        definition["port"] = json!(port);
    }
    let label = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !mapping.is_empty() && !mapping.trim_end_matches(".localhost").split('.').all(label) {
        return Err("services.dev.invalid.mapping");
    }
    for (key, value) in [("cwd", cwd), ("vault", vault), ("logFile", log_file), ("mapping", mapping)] {
        if !value.is_empty() {
            definition[key] = json!(value);
        }
    }
    if let Some(env) = env {
        definition["env"] = env.clone();
    }
    Ok(definition)
}

enum Listing {
    Loading,
    Loaded(Vec<Service>),
    Failed(String),
}

pub struct ServicesScreen {
    model: Entity<AppModel>,
    listing: Listing,
    refreshing: bool,
    dev: Vec<DevServer>,
    /// The list has been read for the workspace in front; before that the section says nothing.
    dev_loaded: bool,
    dev_error: Option<String>,
    filter: Entity<InputState>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ServicesEvent> for ServicesScreen {}

impl ServicesScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(t("services.filter.placeholder")));
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&model, |this, _, event: &ModelEvent, cx| match event {
                ModelEvent::WorkspaceSwitched => {
                    this.dev.clear();
                    this.dev_loaded = false;
                    this.refresh_dev(cx);
                }
                // The screen is built before the workspace is known; the first snapshot is when it can load.
                ModelEvent::ConfigChanged if !this.dev_loaded => this.refresh_dev(cx),
                ModelEvent::DevServiceChanged(payload) => this.apply_dev_change(payload, cx),
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
            listing: Listing::Loading,
            refreshing: false,
            dev: Vec::new(),
            dev_loaded: false,
            dev_error: None,
            filter,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        this.refresh_dev(cx);
        this
    }

    pub fn status_line(&self) -> String {
        let listeners = match &self.listing {
            Listing::Loaded(services) => match services.len() {
                1 => t_with("services.status.count_one", &[("count", &1)]),
                count => t_with("services.status.count_other", &[("count", &count)]),
            },
            _ => return String::new(),
        };
        match self.dev.iter().filter(|service| service.running()).count() {
            0 => listeners,
            1 => format!("{listeners} \u{b7} {}", t_with("services.status.dev_one", &[("count", &1)])),
            count => format!("{listeners} \u{b7} {}", t_with("services.status.dev_other", &[("count", &count)])),
        }
    }

    fn workspace(&self, cx: &gpui_kit::App) -> String {
        self.model.read(cx).workspace_id().to_owned()
    }

    fn notice(&mut self, text: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(text, cx));
    }

    fn report_dev(&mut self, name: &str, message: String, cx: &mut Context<Self>) {
        self.notice(t_with("services.dev.error.action", &[("name", &name), ("message", &message)]), cx);
    }

    /// Reads the workspace's dev servers. A failure keeps the list there is.
    pub fn refresh_dev(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        if workspace.is_empty() {
            return;
        }
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "devservice.list", json!({ "workspaceId": workspace }), move |this, outcome, cx| {
            // The answer to a workspace that has been left since.
            if this.workspace(cx) != workspace {
                return;
            }
            match outcome {
                Ok(result) => {
                    this.dev = result["services"]
                        .as_array()
                        .map(|list| list.iter().filter_map(DevServer::from_value).collect())
                        .unwrap_or_default();
                    this.dev_error = None;
                }
                Err(error) => {
                    this.dev_error = Some(t_with("services.dev.error.list", &[("message", &describe(&error))]));
                }
            }
            this.dev_loaded = true;
            cx.notify();
        });
    }

    /// `event.devservice.changed`: one service, or `null` once it is removed. Other workspaces' are not shown.
    fn apply_dev_change(&mut self, payload: &Value, cx: &mut Context<Self>) {
        if payload.get("workspaceId").and_then(Value::as_str) != Some(self.model.read(cx).workspace_id()) {
            return;
        }
        let Some(name) = payload.get("name").and_then(Value::as_str) else { return };
        self.dev.retain(|service| service.name != name);
        if let Some(service) = payload.get("service").and_then(DevServer::from_value) {
            self.dev.push(service);
            self.dev.sort_by(|a, b| a.name.cmp(&b.name));
        }
        cx.notify();
    }

    /// Shows the output of `name` in the terminal dock.
    fn show_output(&mut self, name: String, fresh: bool, cx: &mut Context<Self>) {
        let Some(service) = self.dev.iter().find(|service| service.name == name) else { return };
        let status = service.run_status();
        cx.emit(ServicesEvent::ShowOutput { name, status, fresh });
    }

    /// Starts `name`. A service with a vault starts with its secrets, which the engine reads itself once the
    /// vault is unsealed.
    fn start(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(service) = self.dev.iter().find(|service| service.name == name) else { return };
        if service.vault.is_empty() {
            return self.up(name, cx);
        }
        let vault = service.vault.clone();
        let found = self.model.read(cx).vaults().iter().find(|v| v.name.eq_ignore_ascii_case(&vault)).cloned();
        let Some(found) = found else {
            let message = t_with("services.dev.invalid.vault", &[("name", &vault)]);
            return self.report_dev(&name, message, cx);
        };
        if found.state == VaultState::Unsealed {
            return self.up(name, cx);
        }
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t_with("services.dev.unseal_title", &[("vault", &found.name), ("name", &name)]),
            t("services.dev.unseal"),
            vec![PromptField::new(t("vaults.passphrase"), "").secret()],
            move |this, values, _, cx| {
                let payload = json!({ "workspaceId": this.workspace(cx), "id": found.id, "passphrase": values[0] });
                let client = this.model.read(cx).client().clone();
                let name = name.clone();
                call(&client, cx, "vault.unseal", payload, move |this, outcome, cx| match failure(&outcome) {
                    Some(message) => this.report_dev(&name, message, cx),
                    None => this.up(name, cx),
                });
            },
        );
    }

    /// The engine refuses a port another process holds, and its message names the holder.
    fn up(&mut self, name: String, cx: &mut Context<Self>) {
        let payload = json!({ "workspaceId": self.workspace(cx), "name": name });
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "devservice.up", payload, move |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                return this.report_dev(&name, message, cx);
            }
            // The answer is the server as started; the list may not have heard of it yet.
            let started = outcome.ok().and_then(|result| result.get("service").and_then(DevServer::from_value));
            let status = started.map_or(RunStatus::Running, |service| service.run_status());
            cx.emit(ServicesEvent::ShowOutput { name, status, fresh: true });
        });
    }

    fn stop(&mut self, name: String, cx: &mut Context<Self>) {
        let payload = json!({ "workspaceId": self.workspace(cx), "name": name });
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "devservice.down", payload, move |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report_dev(&name, message, cx);
            }
        });
    }

    fn restart(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let payload = json!({ "workspaceId": self.workspace(cx), "name": name });
        let client = self.model.read(cx).client().clone();
        call_in(&client, window, cx, "devservice.down", payload, move |this, outcome, window, cx| {
            match failure(&outcome) {
                Some(message) => this.report_dev(&name, message, cx),
                None => this.start(name, window, cx),
            }
        });
    }

    fn confirm_remove(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t_with("services.dev.remove.title", &[("name", &name)]),
            t("services.dev.remove.body"),
            t("services.dev.button.remove"),
            move |this, _, cx| {
                let payload = json!({ "workspaceId": this.workspace(cx), "name": name });
                let client = this.model.read(cx).client().clone();
                let name = name.clone();
                call(&client, cx, "devservice.remove", payload, move |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report_dev(&name, message, cx);
                    }
                });
            },
        );
    }

    /// Adds a dev server, or edits `editing`. The name is asked only when adding: it is the file's name, so an
    /// edit keeps it. `draft` is what a refused dialog held, so the user does not retype it.
    fn prompt_dev(
        &mut self,
        editing: Option<DevServer>,
        draft: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut fields = vec![
            PromptField::new(t("services.dev.field.cmd"), t("services.dev.field.cmd_placeholder")),
            PromptField::new(t("services.dev.field.port"), t("services.dev.field.port_placeholder")).optional(),
            PromptField::new(t("services.dev.field.cwd"), t("services.dev.field.cwd_placeholder")).optional(),
            PromptField::new(t("services.dev.field.vault"), t("services.dev.field.vault_placeholder")).optional(),
            PromptField::new(t("services.dev.field.log_file"), t("services.dev.field.log_file_placeholder")).optional(),
            PromptField::new(t("services.dev.field.mapping"), t("services.dev.field.mapping_placeholder")).optional(),
        ];
        let mut initial = match &editing {
            Some(service) => vec![
                service.cmd.clone(),
                service.port.map(|port| port.to_string()).unwrap_or_default(),
                service.cwd.clone(),
                service.vault.clone(),
                service.log_file.clone(),
                service.mapping.clone(),
            ],
            None => vec![String::new(); 6],
        };
        if editing.is_none() {
            fields.insert(0, PromptField::new(t("services.dev.field.name"), t("services.dev.field.name_placeholder")));
            initial.insert(0, String::new());
        }
        let initial = draft.unwrap_or(initial);
        let fields = fields.into_iter().zip(initial).map(|(field, value)| field.initial(value)).collect();
        let title = match &editing {
            Some(service) => t_with("services.dev.prompt.edit_title", &[("name", &service.name)]),
            None => t("services.dev.prompt.add_title").to_owned(),
        };
        prompt(
            cx.weak_entity(),
            window,
            cx,
            title,
            t("services.button.save"),
            fields,
            move |this, values, window, cx| {
                let (name, rest) = match &editing {
                    Some(service) => (service.name.as_str(), &values[..]),
                    None => (values[0].as_str(), &values[1..]),
                };
                let mut rest = rest.to_vec();
                // The vault is kept as the workspace spells it.
                let named = this
                    .model
                    .read(cx)
                    .vaults()
                    .iter()
                    .find(|v| v.name.eq_ignore_ascii_case(&rest[3]))
                    .map(|v| v.name.clone());
                match named {
                    Some(vault) => rest[3] = vault,
                    None if !rest[3].is_empty() => {
                        let message = t_with("services.dev.invalid.vault", &[("name", &rest[3])]);
                        this.notice(message, cx);
                        return this.prompt_dev(editing.clone(), Some(values), window, cx);
                    }
                    None => {}
                }
                let env = editing.as_ref().and_then(|service| service.env.as_ref());
                match dev_definition(&this.workspace(cx), name, &rest, env) {
                    // The dialog has closed; it opens again with what was typed.
                    Err(key) => {
                        this.notice(t(key).to_owned(), cx);
                        this.prompt_dev(editing.clone(), Some(values), window, cx);
                    }
                    Ok(payload) => {
                        let name = name.to_owned();
                        let client = this.model.read(cx).client().clone();
                        call(&client, cx, "devservice.define", payload, move |this, outcome, cx| {
                            if let Some(message) = failure(&outcome) {
                                this.report_dev(&name, message, cx);
                            }
                        });
                    }
                }
            },
        );
    }

    fn confirm_free_port(&mut self, service: Service, window: &mut Window, cx: &mut Context<Self>) {
        let port = service.port;
        let holder = match service.pid {
            Some(pid) => t_with("services.free.holder", &[("process", &service.process), ("pid", &pid)]),
            None => service.process,
        };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t_with("services.free.title", &[("holder", &holder), ("port", &port)]),
            t("services.free.body"),
            t("services.free.button"),
            move |this, _, cx| {
                let client = this.model.read(cx).client().clone();
                let payload = json!({ "port": port, "kill": true });
                call(&client, cx, "devservice.freePort", payload, move |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.notice(t_with("services.free.error", &[("port", &port), ("message", &message)]), cx);
                    }
                    this.refresh(cx);
                });
            },
        );
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "services.discover", json!({}), |this, outcome, cx| {
            this.refreshing = false;
            this.listing = match outcome {
                Ok(result) => {
                    let mut services: Vec<Service> = result
                        .as_array()
                        .map(|list| list.iter().filter_map(Service::from_value).collect())
                        .unwrap_or_default();
                    services.sort_by_key(|service| service.port);
                    services.dedup_by_key(|service| service.port);
                    Listing::Loaded(services)
                }
                Err(error) => Listing::Failed(describe(&error)),
            };
            cx.notify();
        });
    }

    /// Asks for a name and creates a mapping to `port`.
    fn prompt_map(&mut self, port: u16, window: &mut Window, cx: &mut Context<Self>) {
        let target = format!("localhost:{port}");
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t_with("services.prompt.map.title", &[("port", &port)]),
            t("services.button.create"),
            vec![
                PromptField::new(t("services.field.subdomain"), t("services.field.subdomain_placeholder")),
                PromptField::new(t("services.field.target"), "localhost:3000").initial(target),
            ],
            |this, values, _, cx| {
                let name = values[0].to_lowercase();
                let domain = if name.ends_with(".localhost") { name } else { format!("{name}.localhost") };
                let model = this.model.read(cx);
                let client = model.client().clone();
                let payload = json!({
                    "kind": "mappings",
                    "workspaceId": model.workspace_id(),
                    "entity": { "domain": domain, "target": values[1], "enabled": true },
                });
                call(&client, cx, "entity.create", payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.model.update(cx, |model, cx| {
                            model.notice(t_with("services.error.create", &[("message", &message)]), cx)
                        });
                    }
                });
            },
        );
    }

    fn render_rows(&self, services: &[Service], cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        // The name each mapped port answers to.
        let mapped: Vec<(u16, String)> = self
            .model
            .read(cx)
            .config()
            .mappings
            .iter()
            .filter_map(|m| Some((target_port(&m.target)?, m.domain.clone())))
            .collect();
        let shown: Vec<Service> = services
            .iter()
            .filter(|service| {
                needle.is_empty()
                    || service.process.to_lowercase().contains(&needle)
                    || service.port.to_string().contains(&needle)
            })
            .cloned()
            .collect();
        if shown.is_empty() {
            return page::empty(t("services.empty.no_matches.title"), t("services.empty.no_matches.description"));
        }
        let count = shown.len();
        let row_theme = theme.clone();
        // Only the rows in view are built; the list measures one row and scrolls by that height.
        let rows = uniform_list(
            "service-rows",
            count,
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let theme = &row_theme;
                range
                    .filter_map(|ix| shown.get(ix))
                    .map(|service| {
                        let port = service.port;
                        let holder = service.clone();
                        let name =
                            mapped.iter().find(|(mapped_port, _)| *mapped_port == port).map(|(_, name)| name.clone());
                        // A list row is as wide as its content unless told otherwise, and the columns only
                        // line up with the header when every row spans the list.
                        h_flex()
                            .id(("service", usize::from(port)))
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
                                    .w_20()
                                    .flex_none()
                                    .font_family(theme.mono_font_family.clone())
                                    .child(port.to_string()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(service.process.clone()),
                            )
                            .child(
                                div()
                                    .w_24()
                                    .flex_none()
                                    .text_color(theme.muted_foreground)
                                    .child(service.pid.map(|pid| pid.to_string()).unwrap_or_default()),
                            )
                            .child(
                                div()
                                    .w_40()
                                    .flex_none()
                                    .text_color(theme.muted_foreground)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(service.address.clone()),
                            )
                            .child(
                                div().w_64().flex_none().child(match name {
                                    Some(name) => h_flex()
                                        .gap_2()
                                        .child(Tag::success().small().child(t("services.tag.mapped")))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .font_family(theme.mono_font_family.clone())
                                                .child(name),
                                        )
                                        .into_any_element(),
                                    None => Button::new(("map-service", usize::from(port)))
                                        .small()
                                        .label(t("services.button.map"))
                                        .tooltip(t("services.tooltip.map"))
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| this.prompt_map(port, window, cx)),
                                        )
                                        .into_any_element(),
                                }),
                            )
                            .child(
                                div().w_8().flex_none().child(
                                    Button::new(("free-port", usize::from(port)))
                                        .small()
                                        .ghost()
                                        .icon(IconName::Ban)
                                        .tooltip(t("services.free.tooltip"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.confirm_free_port(holder.clone(), window, cx)
                                        })),
                                ),
                            )
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
                    .child(div().w_20().flex_none().child(t("services.column.port")))
                    .child(div().flex_1().child(t("services.column.process")))
                    .child(div().w_24().flex_none().child(t("services.column.pid")))
                    .child(div().w_40().flex_none().child(t("services.column.address")))
                    .child(div().w_64().flex_none().child(t("services.column.mapping")))
                    .child(div().w_8().flex_none()),
            )
            .child(rows)
            .into_any_element()
    }
}

fn section_heading(title: &'static str) -> impl IntoElement {
    div().flex_none().px_4().pt_3().pb_1().text_sm().font_medium().child(title)
}

impl ServicesScreen {
    /// The dev servers; clicking a row shows its output in the terminal dock.
    fn render_dev(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let body = if let Some(error) = &self.dev_error {
            div().px_4().py_2().text_sm().text_color(theme.danger).child(error.clone()).into_any_element()
        } else if !self.dev_loaded {
            div().into_any_element()
        } else if self.dev.is_empty() {
            h_flex()
                .gap_3()
                .px_4()
                .py_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(div().flex_1().child(t("services.dev.empty")))
                .child(
                    Button::new("dev-empty-add")
                        .small()
                        .icon(IconName::Plus)
                        .label(t("services.button.add_dev"))
                        .on_click(cx.listener(|this, _, window, cx| this.prompt_dev(None, None, window, cx))),
                )
                .into_any_element()
        } else {
            v_flex().children(self.dev.iter().map(|service| self.render_dev_row(service, cx))).into_any_element()
        };
        v_flex().flex_none().child(section_heading(t("services.dev.title"))).child(body).into_any_element()
    }

    fn render_dev_row(&self, service: &DevServer, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let key = |part: &str| ElementId::Name(format!("dev-{part}-{}", service.name).into());
        let tag = match &service.state {
            DevState::Running { .. } if service.starting() => {
                Tag::warning().small().child(t("services.dev.tag.starting"))
            }
            DevState::Running { .. } => Tag::success().small().child(t("services.dev.tag.running")),
            DevState::Stopped => Tag::secondary().small().child(t("services.dev.tag.stopped")),
            DevState::Crashed(code) => Tag::danger().small().child(t_with(
                "services.dev.tag.crashed",
                &[("code", &code.map_or("?".to_owned(), |code| code.to_string()))],
            )),
        };
        let (name, edited) = (service.name.clone(), service.clone());
        let power = if service.running() {
            let (stop, restart) = (name.clone(), name.clone());
            h_flex()
                .gap_1()
                .child(
                    Button::new(key("stop"))
                        .small()
                        .icon(IconName::Square)
                        .label(t("services.dev.button.stop"))
                        .on_click(cx.listener(move |this, _, _, cx| this.stop(stop.clone(), cx))),
                )
                .child(
                    Button::new(key("restart"))
                        .small()
                        .icon(IconName::RotateCw)
                        .tooltip(t("services.dev.tooltip.restart"))
                        .on_click(cx.listener(move |this, _, window, cx| this.restart(restart.clone(), window, cx))),
                )
        } else {
            let start = name.clone();
            h_flex().gap_1().child(
                Button::new(key("start"))
                    .small()
                    .icon(IconName::Play)
                    .label(t("services.dev.button.start"))
                    .on_click(cx.listener(move |this, _, window, cx| this.start(start.clone(), window, cx))),
            )
        };
        let remove = name.clone();
        let open = name.clone();
        h_flex()
            .id(key("row"))
            .test_support()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.show_output(open.clone(), false, cx)))
            .w_full()
            .gap_3()
            .px_4()
            .py_2()
            .text_sm()
            .border_b_1()
            .border_color(theme.border)
            .child(div().w_40().flex_none().overflow_hidden().text_ellipsis().child(name))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(theme.muted_foreground)
                    .font_family(theme.mono_font_family.clone())
                    .child(service.cmd.clone()),
            )
            .child(
                div()
                    .w_20()
                    .flex_none()
                    .font_family(theme.mono_font_family.clone())
                    .child(service.port.or(service.mapped_port).map(|port| port.to_string()).unwrap_or_default()),
            )
            .child(
                div()
                    .id(key("mapping"))
                    .test_support()
                    .w_48()
                    .flex_none()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(theme.muted_foreground)
                    .font_family(theme.mono_font_family.clone())
                    .child(service.mapping.clone()),
            )
            .child(div().w_32().flex_none().child(tag))
            .child(
                h_flex()
                    .id(key("actions"))
                    .flex_none()
                    .gap_1()
                    // The buttons act on their own; a click on one is not a click on the row behind it.
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(power)
                    .child(Button::new(key("edit")).small().label(t("services.dev.button.edit")).on_click(
                        cx.listener(move |this, _, window, cx| this.prompt_dev(Some(edited.clone()), None, window, cx)),
                    ))
                    .child(
                        Button::new(key("remove"))
                            .small()
                            .ghost()
                            .icon(IconName::Delete)
                            .tooltip(t("services.dev.tooltip.remove"))
                            .on_click(
                                cx.listener(move |this, _, window, cx| this.confirm_remove(remove.clone(), window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for ServicesScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            t("services.page.title"),
            t("services.page.description"),
            h_flex()
                .gap_2()
                .child(div().w_64().child(
                    Input::new(&self.filter).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ))
                .child(
                    Button::new("add-dev-server")
                        .small()
                        .icon(IconName::Plus)
                        .label(t("services.button.add_dev"))
                        .on_click(cx.listener(|this, _, window, cx| this.prompt_dev(None, None, window, cx))),
                )
                .child(
                    Button::new("refresh-services")
                        .small()
                        .icon(IconName::RefreshCw)
                        .label(t("services.button.refresh"))
                        .loading(self.refreshing)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
            cx,
        );
        let body = match &self.listing {
            Listing::Loading => page::loading(t("services.page.loading")),
            Listing::Failed(message) => page::empty(t("services.error.list"), message.clone()),
            Listing::Loaded(services) if services.is_empty() => {
                page::empty(t("services.empty.none.title"), t("services.empty.none.description"))
            }
            Listing::Loaded(services) => {
                let services = services.clone();
                self.render_rows(&services, cx)
            }
        };
        let body = v_flex()
            .size_full()
            .child(self.render_dev(cx))
            .child(section_heading(t("services.listeners.title")))
            .child(div().flex_1().min_h_0().child(body));
        page::screen(header, body, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_server_definition_needs_a_plain_name_and_a_real_port_and_keeps_its_env() {
        let fields = |port: &str| ["npm run dev", port, "", "Prod", "", "web"].map(str::to_owned);
        let env = json!({ "A": "1" });
        let made = dev_definition("w1", "web-1", &fields("3000"), Some(&env)).unwrap();
        assert_eq!(made["port"], 3000);
        assert_eq!(made["vault"], "Prod");
        assert_eq!(made["mapping"], "web");
        let bad_mapping = ["npm run dev", "", "", "", "", "my app"].map(str::to_owned);
        assert_eq!(dev_definition("w1", "web", &bad_mapping, None), Err("services.dev.invalid.mapping"));
        assert_eq!(made["env"], env);
        assert!(made.get("cwd").is_none());
        assert_eq!(dev_definition("w1", "web 1", &fields(""), None), Err("services.dev.invalid.name"));
        assert_eq!(dev_definition("w1", "", &fields(""), None), Err("services.dev.invalid.name"));
        for port in ["0", "65536", "abc"] {
            assert_eq!(dev_definition("w1", "web", &fields(port), None), Err("services.dev.invalid.port"));
        }
        assert!(dev_definition("w1", "web", &fields(""), None).unwrap().get("port").is_none());
    }

    #[test]
    fn the_port_is_read_from_any_way_a_target_is_written() {
        assert_eq!(target_port("localhost:3000"), Some(3000));
        assert_eq!(target_port("http://127.0.0.1:8081/path"), Some(8081));
        assert_eq!(target_port("3000"), Some(3000));
        assert_eq!(target_port("localhost"), None);
    }
}
