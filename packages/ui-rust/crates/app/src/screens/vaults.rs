//! The Vaults screen: named groups of encrypted secrets.
//!
//! This screen creates vaults and fills them, and shows their secrets by *name*; no value is ever shown,
//! because the engine never sends one. A vault must be unsealed with its passphrase before secrets can be
//! added or used in requests (`{{vault.SECRET}}`), and it stays unsealed until it is sealed or the engine stops.
//! Starting a program with a vault's secrets is done from the command line
//! (`bifurc vault run <vault> -- <command>`), not here.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    list::ListItem,
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Value, json};

use crate::{
    app_model::AppModel,
    i18n::{t, t_with},
    components::{
        EntityAction, EntityBar,
        dialogs::{PromptField, confirm_danger, prompt},
        entity_bar, page,
    },
    engine::{call, failure},
    model::vault::{VaultInfo, VaultState},
};

pub struct VaultsScreen {
    model: Entity<AppModel>,
    selected: Option<String>,
    _subscription: Subscription,
}

impl VaultsScreen {
    pub fn new(model: Entity<AppModel>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&model, |this, _, cx| {
            this.keep_selection(cx);
            cx.notify();
        });
        let mut this = Self { model, selected: None, _subscription: subscription };
        this.keep_selection(cx);
        this
    }

    /// Keeps the detail pane on a vault that exists.
    fn keep_selection(&mut self, cx: &gpui_kit::App) {
        let vaults = self.model.read(cx).vaults();
        if !self.selected.as_deref().is_some_and(|id| vaults.iter().any(|v| v.id == id)) {
            self.selected = vaults.first().map(|v| v.id.clone());
        }
    }

    fn vault(&self, cx: &gpui_kit::App) -> Option<VaultInfo> {
        let id = self.selected.as_deref()?;
        self.model.read(cx).vaults().iter().find(|v| v.id == id).cloned()
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(t_with("vaults.report", &[("what", &what), ("message", &message)]), cx));
    }

    /// Runs a vault command; a refusal (a wrong passphrase, a bad name) is shown as a notice. The list
    /// refreshes itself on the engine's `vault.changed`.
    fn run(&mut self, action: &'static str, payload: Value, what: &'static str, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        call(&client, cx, action, payload, move |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(what, message, cx);
            }
        });
    }

    fn workspace(&self, cx: &gpui_kit::App) -> String {
        self.model.read(cx).workspace_id().to_owned()
    }

    pub fn prompt_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("vaults.new_vault"),
            t("vaults.create"),
            vec![
                PromptField::new(t("vaults.name"), "api_keys"),
                PromptField::new(t("vaults.passphrase"), t("vaults.passphrase_hint")).secret(),
                PromptField::new(t("vaults.repeat_passphrase"), "").secret(),
            ],
            |this, values, _, cx| {
                if values[1] != values[2] {
                    this.report(t("vaults.error_create"), t("vaults.passphrases_differ").to_owned(), cx);
                    return;
                }
                let payload =
                    json!({ "workspaceId": this.workspace(cx), "name": values[0], "passphrase": values[1] });
                let client = this.model.read(cx).client().clone();
                call(&client, cx, "vault.create", payload, |this, outcome, cx| match failure(&outcome) {
                    Some(message) => this.report(t("vaults.error_create"), message, cx),
                    None => {
                        if let Some(id) = outcome.ok().and_then(|r| r.get("id").and_then(Value::as_str).map(str::to_owned)) {
                            this.selected = Some(id);
                            cx.notify();
                        }
                    }
                });
            },
        );
    }

    fn prompt_unseal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t_with("vaults.unseal_title", &[("name", &vault.name)]),
            t("vaults.unseal"),
            vec![PromptField::new(t("vaults.passphrase"), "").secret()],
            move |this, values, _, cx| {
                let payload = json!({ "workspaceId": this.workspace(cx), "id": vault.id, "passphrase": values[0] });
                this.run("vault.unseal", payload, t("vaults.error_unseal"), cx);
            },
        );
    }

    fn seal(&mut self, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        let payload = json!({ "workspaceId": self.workspace(cx), "id": vault.id });
        self.run("vault.seal", payload, t("vaults.error_seal"), cx);
    }

    pub fn prompt_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("vaults.rename_title"),
            t("vaults.rename"),
            vec![PromptField::new(t("vaults.name"), "api_keys").initial(vault.name.clone())],
            move |this, values, _, cx| {
                let payload = json!({ "workspaceId": this.workspace(cx), "id": vault.id, "name": values[0] });
                this.run("vault.rename", payload, t("vaults.error_rename"), cx);
            },
        );
    }

    fn prompt_change_passphrase(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t_with("vaults.passphrase_title", &[("name", &vault.name)]),
            t("vaults.change"),
            vec![
                PromptField::new(t("vaults.current_passphrase"), "").secret(),
                PromptField::new(t("vaults.new_passphrase"), t("vaults.passphrase_hint")).secret(),
            ],
            move |this, values, _, cx| {
                let payload = json!({
                    "workspaceId": this.workspace(cx),
                    "id": vault.id,
                    "passphrase": values[0],
                    "newPassphrase": values[1],
                });
                this.run("vault.changePassphrase", payload, t("vaults.error_change_passphrase"), cx);
            },
        );
    }

    /// A new secret, or (with `name`) a new value for an existing one.
    fn prompt_secret(&mut self, name: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        let mut fields = Vec::new();
        if name.is_none() {
            fields.push(PromptField::new(t("vaults.name"), "STRIPE_KEY"));
        }
        fields.push(PromptField::new(t("vaults.value"), "").secret());
        let title = match &name {
            Some(name) => t_with("vaults.secret_title_new", &[("name", name)]),
            None => t_with("vaults.secret_title_add", &[("name", &vault.name)]),
        };
        prompt(cx.weak_entity(), window, cx, title, t("vaults.save"), fields, move |this, values, _, cx| {
            let (secret, value) = match &name {
                Some(name) => (name.clone(), values[0].clone()),
                None => (values[0].clone(), values[1].clone()),
            };
            let payload = json!({ "workspaceId": this.workspace(cx), "id": vault.id, "name": secret, "value": value });
            this.run("vault.setSecret", payload, t("vaults.error_save_secret"), cx);
        });
    }

    fn confirm_remove_secret(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t_with("vaults.remove_title", &[("name", &name)]),
            t("vaults.remove_body"),
            t("vaults.remove"),
            move |this, _, cx| {
                let payload = json!({ "workspaceId": this.workspace(cx), "id": vault.id, "name": name });
                this.run("vault.deleteSecret", payload, t("vaults.error_remove_secret"), cx);
            },
        );
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t_with("vaults.delete_title", &[("name", &vault.name)]),
            t("vaults.delete_body"),
            t("vaults.delete"),
            move |this, _, cx| {
                let payload = json!({ "workspaceId": this.workspace(cx), "id": vault.id });
                this.run("vault.delete", payload, t("vaults.error_delete"), cx);
            },
        );
    }

    /// What the footer asks for.
    pub fn entity_action(&mut self, action: EntityAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        match action {
            EntityAction::Sync => self.model.update(cx, |model, cx| model.publish(vec![vault.path.clone()], cx)),
            EntityAction::Revert => {
                confirm_danger(
                    cx.weak_entity(),
                    window,
                    cx,
                    t_with("vaults.revert_title", &[("name", &vault.name)]),
                    t("vaults.revert_body"),
                    t("vaults.revert"),
                    move |this, _, cx| {
                        let path = vault.path.clone();
                        this.model.update(cx, |model, cx| model.revert_paths(vec![path], cx));
                    },
                );
            }
            EntityAction::Delete => self.confirm_delete(window, cx),
            EntityAction::Save | EntityAction::Close | EntityAction::Move(_) => {}
        }
    }

    fn state_tag(vault: &VaultInfo) -> Tag {
        match vault.state {
            VaultState::Sealed => Tag::secondary().small().child(t("vaults.sealed")),
            VaultState::Unsealed => Tag::success().small().child(t("vaults.unsealed")),
        }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let vaults = self.model.read(cx).vaults().to_vec();
        v_flex()
            .id("vault-list")
            .w_64()
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .p_2()
            .gap_1()
            .children(vaults.into_iter().map(|vault| {
                let id = vault.id.clone();
                ListItem::new(gpui_kit::ElementId::Name(format!("vault-{}", vault.id).into()))
                    .selected(self.selected.as_deref() == Some(vault.id.as_str()))
                    .px_2()
                    .py_1()
                    .text_sm()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(vault.name.clone()))
                            .child(Self::state_tag(&vault)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(id.clone());
                        cx.notify();
                    }))
            }))
    }

    fn render_secret(&self, vault: &VaultInfo, name: &str, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let sealed = vault.state == VaultState::Sealed;
        let reference = vault.reference(name);
        let (replace_name, remove_name) = (name.to_owned(), name.to_owned());
        let copy = reference.clone();
        h_flex()
            .id(gpui_kit::ElementId::Name(format!("secret-{name}").into()))
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().font_family(theme.mono_font_family.clone()).child(name.to_owned()))
                    .child(div().text_xs().text_color(theme.muted_foreground).child(reference)),
            )
            .child(
                Button::new(gpui_kit::ElementId::Name(format!("copy-{name}").into()))
                    .ghost()
                    .small()
                    .icon(IconName::Copy)
                    .tooltip(t("vaults.copy_tooltip"))
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))),
            )
            .child(
                Button::new(gpui_kit::ElementId::Name(format!("replace-{name}").into()))
                    .ghost()
                    .small()
                    .label(t("vaults.new_value"))
                    .disabled(sealed)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.prompt_secret(Some(replace_name.clone()), window, cx)
                    })),
            )
            .child(
                Button::new(gpui_kit::ElementId::Name(format!("remove-{name}").into()))
                    .ghost()
                    .small()
                    .label(t("vaults.remove"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.confirm_remove_secret(remove_name.clone(), window, cx)
                    })),
            )
            .into_any_element()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(vault) = self.vault(cx) else {
            return page::empty_with_action(
                t("vaults.no_vault"),
                t("vaults.no_vault_body"),
                Button::new("empty-new-vault")
                    .primary()
                    .small()
                    .label(t("vaults.new_vault"))
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
            );
        };
        let theme = cx.theme().clone();
        let sealed = vault.state == VaultState::Sealed;
        let model = self.model.read(cx);
        // A vault's file is not an entity in the engine's maps, so its state is looked up by path.
        let bar = EntityBar { git: model.git().files.get(&vault.path).copied(), ..EntityBar::of(model, Some(&vault.id)) };

        let toolbar = h_flex()
            .flex_none()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(Self::state_tag(&vault))
            .child(div().flex_1().min_w_0().text_sm().text_color(theme.muted_foreground).child(if sealed {
                t("vaults.state_sealed_hint")
            } else {
                t("vaults.state_unsealed_hint")
            }))
            .child(if sealed {
                Button::new("unseal")
                    .primary()
                    .small()
                    .label(t("vaults.unseal_button"))
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_unseal(window, cx)))
                    .into_any_element()
            } else {
                Button::new("seal")
                    .outline()
                    .small()
                    .label(t("vaults.seal"))
                    .tooltip(t("vaults.seal_tooltip"))
                    .on_click(cx.listener(|this, _, _, cx| this.seal(cx)))
                    .into_any_element()
            })
            .child(
                Button::new("rename-vault")
                    .ghost()
                    .small()
                    .label(t("vaults.rename_vault"))
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_rename(window, cx))),
            )
            .child(
                Button::new("change-passphrase")
                    .ghost()
                    .small()
                    .label(t("vaults.change_passphrase"))
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_change_passphrase(window, cx))),
            );

        let secrets_header = h_flex()
            .flex_none()
            .px_4()
            .py_2()
            .gap_2()
            .child(div().flex_1().child(page::section_label(t_with("vaults.secrets_count", &[("count", &vault.secrets.len())]), cx)))
            .child(
                Button::new("add-secret")
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .label(t("vaults.add_secret"))
                    .disabled(sealed)
                    .tooltip(if sealed { t("vaults.unseal_first_tooltip") } else { t("vaults.add_secret_tooltip") })
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_secret(None, window, cx))),
            );

        let rows = if vault.secrets.is_empty() {
            page::empty(t("vaults.no_secrets"), t("vaults.no_secrets_body"))
        } else {
            v_flex()
                .id("secret-rows")
                .size_full()
                .overflow_y_scroll()
                .children(vault.secrets.iter().map(|name| self.render_secret(&vault, name, cx)))
                .into_any_element()
        };
        let usage = div().flex_none().px_4().py_2().text_xs().text_color(theme.muted_foreground).child(t_with("vaults.usage", &[("name", &vault.name)]));

        v_flex()
            .size_full()
            .child(toolbar)
            .child(secrets_header)
            .child(div().flex_1().min_h_0().child(rows))
            .child(usage)
            .child(entity_bar(bar, cx, |this, action, window, cx| this.entity_action(action, window, cx)))
            .into_any_element()
    }
}

impl Render for VaultsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            t("vaults.title"),
            t("vaults.description"),
            Button::new("new-vault")
                .small()
                .icon(IconName::Plus)
                .label(t("vaults.new_vault"))
                .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
            cx,
        );
        let body = h_flex()
            .size_full()
            .items_stretch()
            .child(self.render_list(cx))
            .child(div().flex_1().min_w_0().child(self.render_detail(cx)));
        page::screen(header, body, cx)
    }
}
