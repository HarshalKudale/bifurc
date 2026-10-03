//! The Vaults screen: named groups of encrypted secrets.
//!
//! This screen creates vaults and fills them, and shows their secrets by *name*; no value is ever shown,
//! because the engine never sends one. A vault must be unsealed with its passphrase before secrets can be
//! added or used in requests (`{{vault.SECRET}}`). Opening a vault for a program is done from the command
//! line (`bifurc unlock <vault> -- <command>`), not here.

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
        self.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
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
            "New vault",
            "Create",
            vec![
                PromptField::new("Name", "api_keys"),
                PromptField::new("Passphrase", "At least 8 characters").secret(),
                PromptField::new("Repeat the passphrase", "").secret(),
            ],
            |this, values, _, cx| {
                if values[1] != values[2] {
                    this.report("Couldn't create the vault.", "The passphrases differ.".to_owned(), cx);
                    return;
                }
                let payload =
                    json!({ "workspaceId": this.workspace(cx), "name": values[0], "passphrase": values[1] });
                let client = this.model.read(cx).client().clone();
                call(&client, cx, "vault.create", payload, |this, outcome, cx| match failure(&outcome) {
                    Some(message) => this.report("Couldn't create the vault.", message, cx),
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
            format!("Unseal “{}”", vault.name),
            "Unseal",
            vec![PromptField::new("Passphrase", "").secret()],
            move |this, values, _, cx| {
                let payload = json!({ "workspaceId": this.workspace(cx), "id": vault.id, "passphrase": values[0] });
                this.run("vault.unseal", payload, "Couldn't unseal the vault.", cx);
            },
        );
    }

    fn seal(&mut self, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        let payload = json!({ "workspaceId": self.workspace(cx), "id": vault.id });
        self.run("vault.seal", payload, "Couldn't seal the vault.", cx);
    }

    fn confirm_seal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        if vault.opened_by == 0 {
            return self.seal(cx);
        }
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Seal “{}”?", vault.name),
            "A program is running with this vault open. It is stopped when the vault is sealed.",
            "Seal and stop",
            |this, _, cx| this.seal(cx),
        );
    }

    fn prompt_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        prompt(
            cx.weak_entity(),
            window,
            cx,
            "Rename vault",
            "Rename",
            vec![PromptField::new("Name", "api_keys").initial(vault.name.clone())],
            move |this, values, _, cx| {
                let payload = json!({ "workspaceId": this.workspace(cx), "id": vault.id, "name": values[0] });
                this.run("vault.rename", payload, "Couldn't rename the vault.", cx);
            },
        );
    }

    fn prompt_change_passphrase(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        prompt(
            cx.weak_entity(),
            window,
            cx,
            format!("New passphrase for “{}”", vault.name),
            "Change",
            vec![
                PromptField::new("Current passphrase", "").secret(),
                PromptField::new("New passphrase", "At least 8 characters").secret(),
            ],
            move |this, values, _, cx| {
                let payload = json!({
                    "workspaceId": this.workspace(cx),
                    "id": vault.id,
                    "passphrase": values[0],
                    "newPassphrase": values[1],
                });
                this.run("vault.changePassphrase", payload, "Couldn't change the passphrase.", cx);
            },
        );
    }

    /// A new secret, or (with `name`) a new value for an existing one.
    fn prompt_secret(&mut self, name: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        let mut fields = Vec::new();
        if name.is_none() {
            fields.push(PromptField::new("Name", "STRIPE_KEY"));
        }
        fields.push(PromptField::new("Value", "").secret());
        let title = match &name {
            Some(name) => format!("New value for {name}"),
            None => format!("Add a secret to “{}”", vault.name),
        };
        prompt(cx.weak_entity(), window, cx, title, "Save", fields, move |this, values, _, cx| {
            let (secret, value) = match &name {
                Some(name) => (name.clone(), values[0].clone()),
                None => (values[0].clone(), values[1].clone()),
            };
            let payload = json!({ "workspaceId": this.workspace(cx), "id": vault.id, "name": secret, "value": value });
            this.run("vault.setSecret", payload, "Couldn't save the secret.", cx);
        });
    }

    fn confirm_remove_secret(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Remove {name}?"),
            "Requests that use it will send the reference unresolved.",
            "Remove",
            move |this, _, cx| {
                let payload = json!({ "workspaceId": this.workspace(cx), "id": vault.id, "name": name });
                this.run("vault.deleteSecret", payload, "Couldn't remove the secret.", cx);
            },
        );
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Delete vault “{}”?", vault.name),
            "Its secrets are deleted and cannot be recovered without a synced copy.",
            "Delete",
            move |this, _, cx| {
                let payload = json!({ "workspaceId": this.workspace(cx), "id": vault.id });
                this.run("vault.delete", payload, "Couldn't delete the vault.", cx);
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
                    format!("Revert “{}” to the last synced version?", vault.name),
                    "Secrets added or changed since then are discarded.",
                    "Revert",
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
            VaultState::Sealed => Tag::secondary().small().child("Sealed"),
            VaultState::Ready => Tag::success().small().child("Ready"),
            VaultState::Open => Tag::warning().small().child(if vault.opened_by > 1 {
                format!("Open · {} programs", vault.opened_by)
            } else {
                "Open".to_owned()
            }),
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
                    .tooltip("Copy the reference")
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))),
            )
            .child(
                Button::new(gpui_kit::ElementId::Name(format!("replace-{name}").into()))
                    .ghost()
                    .small()
                    .label("New value")
                    .disabled(sealed)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.prompt_secret(Some(replace_name.clone()), window, cx)
                    })),
            )
            .child(
                Button::new(gpui_kit::ElementId::Name(format!("remove-{name}").into()))
                    .ghost()
                    .small()
                    .label("Remove")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.confirm_remove_secret(remove_name.clone(), window, cx)
                    })),
            )
            .into_any_element()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(vault) = self.vault(cx) else {
            return page::empty_with_action(
                "No vault",
                "A vault keeps secrets encrypted in the workspace. Only the engine can read them.",
                Button::new("empty-new-vault")
                    .primary()
                    .small()
                    .label("New vault")
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
                "Enter the passphrase to use or edit its secrets."
            } else {
                "Unsealed: its secrets resolve in requests."
            }))
            .child(if sealed {
                Button::new("unseal")
                    .primary()
                    .small()
                    .label("Unseal…")
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_unseal(window, cx)))
                    .into_any_element()
            } else {
                Button::new("seal")
                    .outline()
                    .small()
                    .label("Seal")
                    .tooltip("Forget the key; the passphrase is needed again")
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_seal(window, cx)))
                    .into_any_element()
            })
            .child(
                Button::new("rename-vault")
                    .ghost()
                    .small()
                    .label("Rename…")
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_rename(window, cx))),
            )
            .child(
                Button::new("change-passphrase")
                    .ghost()
                    .small()
                    .label("Change passphrase…")
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_change_passphrase(window, cx))),
            );

        let secrets_header = h_flex()
            .flex_none()
            .px_4()
            .py_2()
            .gap_2()
            .child(div().flex_1().child(page::section_label(format!("Secrets ({})", vault.secrets.len()), cx)))
            .child(
                Button::new("add-secret")
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .label("Add secret")
                    .disabled(sealed)
                    .tooltip(if sealed { "Unseal the vault first" } else { "Add a secret to this vault" })
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_secret(None, window, cx))),
            );

        let rows = if vault.secrets.is_empty() {
            page::empty("No secrets", "Add one, then use it in a request as {{vault.NAME}}.")
        } else {
            v_flex()
                .id("secret-rows")
                .size_full()
                .overflow_y_scroll()
                .children(vault.secrets.iter().map(|name| self.render_secret(&vault, name, cx)))
                .into_any_element()
        };
        let usage = div().flex_none().px_4().py_2().text_xs().text_color(theme.muted_foreground).child(format!(
            "To run a program with these as environment variables:  bifurc unlock {} -- <command>",
            vault.name
        ));

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
            "Vaults",
            "Encrypted secrets that requests use as {{vault.NAME}}. Only the engine can read them; the files are safe to sync.",
            Button::new("new-vault")
                .small()
                .icon(IconName::Plus)
                .label("New vault")
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
