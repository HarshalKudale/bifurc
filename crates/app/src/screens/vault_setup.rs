//! Making a vault, and setting a linked one up on this device: a short walk in a dialog.
//!
//! A new vault starts with where its values are kept: on this device, sealed with a passphrase, or in a secret
//! manager, which the engine pulls from through the manager's own program. Then its details (a name and a
//! passphrase, or the source in the manager and an optional token for this device), and for a linked vault a
//! check that pulls the source once and names what it holds. A linked vault's secrets are never edited here:
//! they are changed in the manager. Setting up a linked vault that came with the workspace (or whose source a
//! sync changed) is the same walk without the first step: it ends by approving the vault on this device.
//!
//! Every rule (what a provider needs, what a name may be) is the engine's; this view collects text and shows
//! what came back. A token typed here goes to the engine, which keeps it on this device only.

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    dialog::DialogButtonProps,
    form::{Field, Form},
    h_flex,
    input::{Input, InputState},
    link::Link,
    list::ListItem,
    spinner::Spinner,
    stepper::{Stepper, StepperItem},
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, rems,
};
use serde_json::{Map, Value, json};

use crate::{
    app_model::AppModel,
    engine::{call, call_in, failure},
    i18n::{t, t_with},
    model::vault::{ProviderInfo, VaultInfo, field_label},
};

/// The dialog's width in rems: room for a secret's name beside its reference.
const WIDTH: f32 = 40.;

/// What the walk is for.
#[derive(Clone)]
pub enum Purpose {
    Create,
    /// Set this linked vault up on this device, and approve it.
    Setup(VaultInfo),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Storage,
    Details,
    Secrets,
}

impl Step {
    fn label(self) -> &'static str {
        t(match self {
            Step::Storage => "vault_setup.step.storage",
            Step::Details => "vault_setup.step.details",
            Step::Secrets => "vault_setup.step.check",
        })
    }
}

type DoneHandler = Rc<dyn Fn(&str, &mut Window, &mut App)>;

pub struct VaultSetup {
    model: Entity<AppModel>,
    purpose: Purpose,
    providers: Option<Vec<ProviderInfo>>,
    provider: Option<String>,
    step: Step,
    name: Entity<InputState>,
    passphrase: Entity<InputState>,
    repeat: Entity<InputState>,
    /// The chosen provider's settings, by key.
    fields: Vec<(String, Entity<InputState>)>,
    credential: Entity<InputState>,
    /// The names a check pulled from the source.
    found: Option<Vec<String>>,
    checking: bool,
    busy: bool,
    error: Option<String>,
    on_done: Option<DoneHandler>,
}

impl VaultSetup {
    pub fn new(model: Entity<AppModel>, purpose: Purpose, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &'static str, masked: bool, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).masked(masked))
        };
        let name = input(t("vault_setup.placeholder.name"), false, window, cx);
        let passphrase = input(t("vault_setup.placeholder.passphrase"), true, window, cx);
        let repeat = input("", true, window, cx);
        let credential = input("", true, window, cx);
        let (provider, step) = match &purpose {
            Purpose::Create => (None, Step::Storage),
            Purpose::Setup(vault) => (Some(vault.provider.clone()), Step::Details),
        };
        let mut this = Self {
            model,
            purpose,
            providers: None,
            provider,
            step,
            name,
            passphrase,
            repeat,
            fields: Vec::new(),
            credential,
            found: None,
            checking: false,
            busy: false,
            error: None,
            on_done: None,
        };
        this.load_providers(window, cx);
        this
    }

    /// What to do with the vault's id once it was made or set up.
    pub fn on_done(mut self, on_done: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_done = Some(Rc::new(on_done));
        self
    }

    fn client(&self, cx: &App) -> bifurc_engine_client::EngineClient {
        self.model.read(cx).client().clone()
    }

    fn load_providers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.client(cx);
        call_in(&client, window, cx, "vault.providers", json!({}), |this, outcome, window, cx| {
            match outcome {
                Ok(value) => {
                    this.providers = Some(ProviderInfo::list_from_value(&value));
                    if let Some(id) = this.provider.clone() {
                        this.choose(&id, window, cx);
                    }
                }
                Err(_) => this.error = failure(&outcome),
            }
            cx.notify();
        });
    }

    fn chosen(&self) -> Option<&ProviderInfo> {
        let id = self.provider.as_deref()?;
        self.providers.as_ref()?.iter().find(|p| p.id == id)
    }

    fn is_local(&self) -> bool {
        self.chosen().is_some_and(ProviderInfo::is_local)
    }

    fn steps(&self) -> Vec<Step> {
        match (&self.purpose, self.is_local()) {
            (Purpose::Setup(_), _) => vec![Step::Details, Step::Secrets],
            (Purpose::Create, true) => vec![Step::Storage, Step::Details],
            (Purpose::Create, false) => vec![Step::Storage, Step::Details, Step::Secrets],
        }
    }

    /// Picks a provider: its settings get fields, filled from the vault being set up.
    fn choose(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.provider = Some(id.to_owned());
        self.found = None;
        self.error = None;
        let Some(provider) = self.chosen().cloned() else { return };
        let saved: Vec<(String, String)> = match &self.purpose {
            Purpose::Setup(vault) => vault.settings.clone(),
            Purpose::Create => Vec::new(),
        };
        self.fields = provider
            .fields
            .iter()
            .map(|field| {
                let initial = saved.iter().find(|(k, _)| *k == field.key).map(|(_, v)| v.clone()).unwrap_or_default();
                let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
                (field.key.clone(), input)
            })
            .collect();
        cx.notify();
    }

    fn text(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_owned()
    }

    fn settings(&self, cx: &App) -> Value {
        let map: Map<String, Value> = self
            .fields
            .iter()
            .map(|(key, input)| (key.clone(), json!(Self::text(input, cx))))
            .filter(|(_, value)| value.as_str() != Some(""))
            .collect();
        Value::Object(map)
    }

    /// Why the step cannot be left yet, if it cannot.
    fn problem(&self, cx: &App) -> Option<&'static str> {
        match self.step {
            Step::Storage => self.provider.is_none().then(|| t("vault_setup.problem.choose")),
            Step::Details => {
                let creating = matches!(self.purpose, Purpose::Create);
                if creating && Self::text(&self.name, cx).is_empty() {
                    return Some(t("vault_setup.problem.name"));
                }
                if self.is_local() {
                    let (pass, repeat) = (self.passphrase.read(cx).value(), self.repeat.read(cx).value());
                    if pass.chars().count() < 8 {
                        return Some(t("vault_setup.problem.passphrase_short"));
                    }
                    if pass != repeat {
                        return Some(t("vault_setup.problem.passphrases_differ"));
                    }
                    return None;
                }
                let provider = self.chosen()?;
                let missing = self.fields.iter().any(|(key, input)| {
                    provider.fields.iter().any(|f| f.key == *key && f.required) && Self::text(input, cx).is_empty()
                });
                if missing {
                    return Some(t("vault_setup.problem.setting"));
                }
                let has_token = match &self.purpose {
                    Purpose::Setup(vault) => vault.has_credential,
                    Purpose::Create => false,
                };
                (provider.credential_required && !has_token && Self::text(&self.credential, cx).is_empty())
                    .then(|| t("vault_setup.problem.token"))
            }
            Step::Secrets => None,
        }
    }

    /// The main action: the next step, or the vault itself.
    pub fn advance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.problem(cx).is_some() {
            return;
        }
        let steps = self.steps();
        let index = steps.iter().position(|step| *step == self.step).unwrap_or(0);
        match steps.get(index + 1) {
            Some(next) => {
                self.step = *next;
                self.error = None;
                self.focus(window, cx);
                cx.notify();
            }
            None => self.finish(window, cx),
        }
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        let steps = self.steps();
        if let Some(index) = steps.iter().position(|step| *step == self.step).filter(|index| *index > 0) {
            self.step = steps[index - 1];
            self.error = None;
            cx.notify();
        }
    }

    fn workspace(&self, cx: &App) -> String {
        self.model.read(cx).workspace_id().to_owned()
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        let token = Self::text(&self.credential, cx);
        let (action, payload) = match &self.purpose {
            Purpose::Setup(vault) => {
                let mut payload = json!({ "workspaceId": workspace, "id": vault.id, "settings": self.settings(cx) });
                // Left empty, the token this device already keeps stays.
                if !token.is_empty() {
                    payload["credential"] = json!(token);
                }
                ("vault.configure", payload)
            }
            Purpose::Create if self.is_local() => (
                "vault.create",
                json!({
                    "workspaceId": workspace, "name": Self::text(&self.name, cx),
                    "passphrase": self.passphrase.read(cx).value().to_string(),
                }),
            ),
            Purpose::Create => (
                "vault.create",
                json!({
                    "workspaceId": workspace, "name": Self::text(&self.name, cx), "provider": self.provider,
                    "settings": self.settings(cx), "credential": token,
                }),
            ),
        };
        self.busy = true;
        self.error = None;
        cx.notify();
        let client = self.client(cx);
        call_in(&client, window, cx, action, payload, |this, outcome, window, cx| {
            this.busy = false;
            match failure(&outcome) {
                Some(message) => this.error = Some(message),
                None => {
                    let id = outcome.ok().and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_owned));
                    if let (Some(id), Some(on_done)) = (id, this.on_done.clone()) {
                        on_done(&id, window, cx);
                    }
                    window.close_dialog(cx);
                }
            }
            cx.notify();
        });
    }

    /// Pulls the source as the walk has it now, without keeping anything; only names come back.
    fn check(&mut self, cx: &mut Context<Self>) {
        let mut payload =
            json!({ "workspaceId": self.workspace(cx), "provider": self.provider, "settings": self.settings(cx) });
        let token = Self::text(&self.credential, cx);
        if !token.is_empty() {
            payload["credential"] = json!(token);
        }
        // This device's token, when none was typed.
        if let Purpose::Setup(vault) = &self.purpose {
            payload["id"] = json!(vault.id);
        }
        self.checking = true;
        self.found = None;
        self.error = None;
        cx.notify();
        let client = self.client(cx);
        call(&client, cx, "vault.test", payload, |this, outcome, cx| {
            this.checking = false;
            match (failure(&outcome), outcome) {
                (Some(message), _) => this.error = Some(message),
                (None, Ok(value)) => {
                    let names = value.get("names").and_then(Value::as_array).cloned().unwrap_or_default();
                    this.found = Some(names.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect());
                }
                (None, Err(_)) => {}
            }
            cx.notify();
        });
    }

    fn render_steps(&self, cx: &mut Context<Self>) -> AnyElement {
        let steps = self.steps();
        let current = steps.iter().position(|step| *step == self.step).unwrap_or(0);
        let targets = steps.clone();
        Stepper::new("vault-setup-steps")
            .small()
            .selected_index(current)
            .items(
                steps
                    .iter()
                    .enumerate()
                    .map(|(index, step)| StepperItem::new().disabled(index > current || self.busy).child(step.label())),
            )
            .on_click(cx.listener(move |this, index: &usize, _, cx| {
                if *index < current && !this.busy {
                    this.step = targets[*index];
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    fn render_storage(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(providers) = self.providers.clone() else {
            return h_flex().gap_2().child(Spinner::new().small()).child(t("vault_setup.loading")).into_any_element();
        };
        let rows = providers.into_iter().map(|provider| {
            let id = provider.id.clone();
            let description = if provider.is_local() {
                t("vault_setup.storage.local_description").to_owned()
            } else {
                t_with("vault_setup.storage.linked_description", &[("program", &provider.program)])
            };
            let label =
                if provider.is_local() { t("vault_setup.storage.local").to_owned() } else { provider.label.clone() };
            ListItem::new(gpui_kit::ElementId::Name(format!("provider-{}", provider.id).into()))
                .selected(self.provider.as_deref() == Some(provider.id.as_str()))
                .px_2()
                .py_1()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_sm().child(label))
                                .child(div().text_xs().text_color(theme.muted_foreground).child(description)),
                        )
                        .when(!provider.installed, |row| {
                            row.child(Tag::secondary().small().child(t("vault_setup.storage.not_installed")))
                        }),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.choose(&id, window, cx)))
        });
        let missing = self.chosen().filter(|p| !p.installed).cloned();
        v_flex()
            .gap_3()
            .child(div().text_sm().text_color(theme.muted_foreground).child(t("vault_setup.storage.intro")))
            .child(v_flex().gap_1().children(rows))
            .when_some(missing, |this, provider| {
                let text = t_with(
                    "vault_setup.storage.missing",
                    &[("provider", &provider.label), ("program", &provider.program)],
                );
                this.child(
                    v_flex().gap_1().child(Alert::warning("provider-missing", text)).child(
                        Link::new("provider-install")
                            .href(provider.install_url.clone())
                            .child(t("vault_setup.storage.install")),
                    ),
                )
            })
            .into_any_element()
    }

    fn render_details(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(provider) = self.chosen().cloned() else {
            return h_flex().gap_2().child(Spinner::new().small()).child(t("vault_setup.loading")).into_any_element();
        };
        let mut form = Form::new();
        if matches!(self.purpose, Purpose::Create) {
            form = form.child(
                Field::new()
                    .label(t("vault_setup.field.name"))
                    .description(t("vault_setup.field.name_help"))
                    .child(Input::new(&self.name).py_1p5()),
            );
        }
        if provider.is_local() {
            form = form
                .child(
                    Field::new()
                        .label(t("vault_setup.field.passphrase"))
                        .description(t("vault_setup.field.passphrase_help"))
                        .child(Input::new(&self.passphrase).py_1p5().mask_toggle()),
                )
                .child(
                    Field::new()
                        .label(t("vault_setup.field.repeat"))
                        .child(Input::new(&self.repeat).py_1p5().mask_toggle()),
                );
        } else {
            for (key, input) in &self.fields {
                let required = provider.fields.iter().any(|f| f.key == *key && f.required);
                form = form
                    .child(Field::new().label(field_label(key)).required(required).child(Input::new(input).py_1p5()));
            }
            if provider.takes_credential {
                let kept = matches!(&self.purpose, Purpose::Setup(vault) if vault.has_credential);
                let help = if kept {
                    t("vault_setup.field.token_kept").to_owned()
                } else if provider.credential_required {
                    t("vault_setup.field.token_required").to_owned()
                } else {
                    t_with("vault_setup.field.token_help", &[("program", &provider.program)])
                };
                form = form.child(
                    Field::new()
                        .label(t("vault_setup.field.token"))
                        .required(provider.credential_required && !kept)
                        .description(help)
                        .child(Input::new(&self.credential).py_1p5().mask_toggle()),
                );
            }
        }
        let note = if provider.is_local() {
            t("vault_setup.details.local_note").to_owned()
        } else {
            t_with(
                if provider.takes_credential {
                    "vault_setup.details.linked_note"
                } else {
                    "vault_setup.details.linked_note_no_token"
                },
                &[("provider", &provider.label)],
            )
        };
        v_flex()
            .gap_3()
            .child(form)
            .child(div().text_xs().text_color(theme.muted_foreground).child(note))
            .into_any_element()
    }

    /// What pulling the source once found: its secrets' names, never a value.
    fn render_secrets(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let label = self.chosen().map(|p| p.label.clone()).unwrap_or_default();
        let intro = match &self.purpose {
            Purpose::Setup(vault) if !vault.approved => t_with("vault_setup.check.review", &[("provider", &label)]),
            _ => t_with("vault_setup.check.intro", &[("provider", &label)]),
        };
        let found = self.found.as_ref().map(|names| {
            if names.is_empty() {
                div().text_sm().text_color(theme.muted_foreground).child(t("vault_setup.check.none")).into_any_element()
            } else {
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child(t_with("vault_setup.check.found", &[("count", &names.len())])))
                    .child(v_flex().id("vault-setup-found").max_h(rems(14.)).overflow_y_scroll().children(
                        names.iter().map(|name| {
                            div().text_sm().font_family(theme.mono_font_family.clone()).child(name.clone())
                        }),
                    ))
                    .into_any_element()
            }
        });
        v_flex()
            .gap_3()
            .child(div().text_sm().text_color(theme.muted_foreground).child(intro))
            .child(
                h_flex().child(
                    Button::new("check-source")
                        .outline()
                        .small()
                        .label(t("vault_setup.check.button"))
                        .loading(self.checking)
                        .disabled(self.checking)
                        .on_click(cx.listener(|this, _, _, cx| this.check(cx))),
                ),
            )
            .children(found)
            .into_any_element()
    }

    fn render_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let steps = self.steps();
        let last = steps.last() == Some(&self.step);
        let label = match (&self.purpose, last) {
            (_, false) => t("vault_setup.action.next"),
            (Purpose::Create, true) => t("vault_setup.action.create"),
            (Purpose::Setup(_), true) => t("vault_setup.action.approve"),
        };
        let problem = self.problem(cx);
        h_flex()
            .w_full()
            .gap_2()
            .when(steps.first() != Some(&self.step), |this| {
                this.child(
                    Button::new("vault-setup-back")
                        .outline()
                        .label(t("vault_setup.action.back"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
                )
            })
            .child(div().flex_1().min_w_0().text_xs().text_color(cx.theme().muted_foreground).children(problem))
            .child(
                Button::new("vault-setup-cancel")
                    .outline()
                    .label(t("vault_setup.action.cancel"))
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                Button::new("vault-setup-next")
                    .primary()
                    .label(label)
                    .loading(self.busy)
                    .disabled(problem.is_some() || self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.advance(window, cx))),
            )
            .into_any_element()
    }

    /// Puts the keyboard in the step's first field.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let target = match (self.step, &self.purpose) {
            (Step::Storage, _) => None,
            (Step::Details, Purpose::Create) => Some(&self.name),
            (Step::Details, Purpose::Setup(_)) => {
                self.fields.first().map(|(_, input)| input).or(Some(&self.credential))
            }
            (Step::Secrets, _) => None,
        };
        if let Some(target) = target {
            target.update(cx, |input, cx| input.focus(window, cx));
        }
    }
}

impl Render for VaultSetup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.step {
            Step::Storage => self.render_storage(cx),
            Step::Details => self.render_details(cx),
            Step::Secrets => self.render_secrets(cx),
        };
        v_flex()
            .gap_4()
            .child(self.render_steps(cx))
            .child(body)
            .when_some(self.error.clone(), |this, error| this.child(Alert::error("vault-setup-error", error)))
            .child(self.render_actions(cx))
    }
}

/// Opens the walk in a dialog. `on_done` gets the id of the vault made or set up.
pub fn open_vault_setup(
    model: Entity<AppModel>,
    purpose: Purpose,
    window: &mut Window,
    cx: &mut App,
    on_done: impl Fn(&str, &mut Window, &mut App) + 'static,
) {
    let title: SharedString = match &purpose {
        Purpose::Create => t("vault_setup.title.create").into(),
        Purpose::Setup(vault) => t_with("vault_setup.title.setup", &[("name", &vault.name)]).into(),
    };
    // Deferred so a dialog opened from a menu or the palette opens after that closes.
    window.defer(cx, move |window, cx| {
        let view = cx.new(|cx| VaultSetup::new(model, purpose, window, cx).on_done(on_done));
        let focus = view.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let advance = view.clone();
            let rem = window.rem_size();
            // The view draws its own actions; Enter goes to its main one.
            dialog.title(title.clone()).w(rems(WIDTH).to_pixels(rem)).child(view.clone()).button_props(
                DialogButtonProps::default().on_ok(move |_, window, cx| {
                    advance.update(cx, |view, cx| view.advance(window, cx));
                    false
                }),
            )
        });
        window.defer(cx, move |window, cx| focus.update(cx, |view, cx| view.focus(window, cx)));
    });
}
