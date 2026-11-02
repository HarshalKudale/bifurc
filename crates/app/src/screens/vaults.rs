//! The Vaults screen: named groups of secrets.
//!
//! This screen makes vaults (in a walk: [`vault_setup`]) and shows their secrets by *name*; no value is ever
//! shown, because the engine never sends one. A local vault keeps its values on this device, sealed with its
//! passphrase, and is the only kind whose secrets are added, changed or removed here. A linked vault pulls
//! every secret of one source in a secret manager: they are changed there, and only the source is synced.
//! Either must be unsealed before its secrets are used in requests (`{{vault.SECRET}}`), and it stays unsealed
//! until it is sealed or the engine stops. Starting a program with a vault's secrets is done from the command
//! line (`bifurc vault run <vault> -- <command>`), not here.
//!
//! [`vault_setup`]: crate::screens::vault_setup

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    list::ListItem,
    tag::Tag,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
};
use serde_json::{Value, json};

use crate::{
    app_model::AppModel,
    components::{
        EntityAction, EntityBar,
        dialogs::{PromptField, confirm_danger, prompt},
        entity_bar,
        master_detail::{list_rows, master_detail},
        page,
    },
    engine::{call, failure},
    i18n::{t, t_with},
    model::vault::{VaultInfo, VaultState},
    screens::vault_setup::{Purpose, open_vault_setup},
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
        self.model.update(cx, |model, cx| {
            model.notice(t_with("vaults.report", &[("what", &what), ("message", &message)]), cx)
        });
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

    /// The walk for a new vault, or for setting a linked one up on this device.
    fn open_setup(&mut self, purpose: Purpose, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.weak_entity();
        open_vault_setup(self.model.clone(), purpose, window, cx, move |id, _, cx| {
            let id = id.to_owned();
            view.update(cx, |this, cx| {
                this.selected = Some(id);
                this.model.update(cx, |model, cx| model.refresh_vaults(cx));
                cx.notify();
            })
            .ok();
        });
    }

    pub fn prompt_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_setup(Purpose::Create, window, cx);
    }

    /// A local vault asks for its passphrase; a linked one pulls its source at once.
    fn unseal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        if !vault.is_local() {
            let payload = json!({ "workspaceId": self.workspace(cx), "id": vault.id });
            self.run("vault.unseal", payload, t("vaults.error_unseal"), cx);
            return;
        }
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

    /// Pulls a linked vault's source once and says how many secrets it holds; no value comes back.
    fn check(&mut self, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx) else { return };
        let payload = json!({ "workspaceId": self.workspace(cx), "id": vault.id });
        let client = self.model.read(cx).client().clone();
        call(&client, cx, "vault.test", payload, move |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(t("vaults.error_check"), message, cx);
                return;
            }
            let count = outcome.ok().and_then(|v| v.get("names").and_then(Value::as_array).map(Vec::len)).unwrap_or(0);
            let text = t_with("vaults.check_ok", &[("name", &vault.name), ("count", &count)]);
            this.model.update(cx, |model, cx| model.notice(text, cx));
        });
    }

    /// Closes the "moved to this device" notice for good: every vault that was moved is acknowledged, not only the
    /// one on screen, so no other vault tells the user again in the next session.
    fn acknowledge(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace(cx);
        let moved: Vec<String> =
            self.model.read(cx).vaults().iter().filter(|v| v.moved).map(|v| v.id.clone()).collect();
        for id in moved {
            let payload = json!({ "workspaceId": workspace, "id": id });
            self.run("vault.acknowledge", payload, t("vaults.error_acknowledge"), cx);
        }
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

    /// A new secret, or (with `name`) a new value for an existing one, of a local vault.
    fn prompt_secret(&mut self, name: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault(cx).filter(VaultInfo::is_local) else { return };
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
        let Some(vault) = self.vault(cx).filter(VaultInfo::is_local) else { return };
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
        let body = if vault.is_local() { t("vaults.delete_body") } else { t("vaults.delete_linked_body") };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t_with("vaults.delete_title", &[("name", &vault.name)]),
            body,
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
            EntityAction::Sync if !vault.path.is_empty() => {
                self.model.update(cx, |model, cx| model.publish(vec![vault.path.clone()], cx))
            }
            EntityAction::Revert if !vault.path.is_empty() => {
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
            _ => {}
        }
    }

    fn state_tag(vault: &VaultInfo) -> Tag {
        match vault.state {
            _ if !vault.approved => Tag::warning().small().child(t("vaults.needs_setup")),
            VaultState::Sealed => Tag::secondary().small().child(t("vaults.sealed")),
            VaultState::Unsealed => Tag::success().small().child(t("vaults.unsealed")),
        }
    }

    /// Where the vault's values are kept, as one short phrase.
    fn kept_in(vault: &VaultInfo) -> String {
        if vault.is_local() { t("vaults.kept_here").to_owned() } else { vault.provider_label.clone() }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let vaults = self.model.read(cx).vaults().to_vec();
        list_rows("vault-list").children(vaults.into_iter().map(|vault| {
            let id = vault.id.clone();
            ListItem::new(gpui_kit::ElementId::Name(format!("vault-{}", vault.id).into()))
                .selected(self.selected.as_deref() == Some(vault.id.as_str()))
                .px_2()
                .py_1()
                .text_sm()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().overflow_hidden().text_ellipsis().child(vault.name.clone()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(Self::kept_in(&vault)),
                                ),
                        )
                        .child(Self::state_tag(&vault)),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = Some(id.clone());
                    cx.notify();
                }))
        }))
    }

    /// One secret by name, with how a request uses it. Only a local vault's can be changed or removed here.
    fn render_secret(&self, vault: &VaultInfo, name: &str, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let usage = vault.reference(name);
        let (replace_name, remove_name) = (name.to_owned(), name.to_owned());
        let copy = usage.clone();
        let editable = vault.is_local();
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
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(usage),
                    ),
            )
            .child(
                Button::new(gpui_kit::ElementId::Name(format!("copy-{name}").into()))
                    .ghost()
                    .small()
                    .icon(IconName::Copy)
                    .tooltip(t("vaults.copy_tooltip"))
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))),
            )
            .when(editable, |row| {
                row.child(
                    Button::new(gpui_kit::ElementId::Name(format!("replace-{name}").into()))
                        .ghost()
                        .small()
                        .label(t("vaults.new_value"))
                        .disabled(vault.state == VaultState::Sealed)
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
            })
            .into_any_element()
    }

    /// What the user has to know before using the vault: it was moved here, or it is not set up here.
    fn render_notice(&self, vault: &VaultInfo, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        if vault.moved {
            return Some(
                v_flex()
                    .flex_none()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .child({
                        let view = cx.weak_entity();
                        Alert::warning("vault-moved", t("vaults.moved_body")).title(t("vaults.moved_title")).on_close(
                            move |_, _, cx| {
                                view.update(cx, |this, cx| this.acknowledge(cx)).ok();
                            },
                        )
                    })
                    .child(
                        h_flex().justify_end().child(
                            Button::new("vault-moved-ok")
                                .outline()
                                .small()
                                .label(t("vaults.moved_ok"))
                                .on_click(cx.listener(|this, _, _, cx| this.acknowledge(cx))),
                        ),
                    )
                    .into_any_element(),
            );
        }
        (!vault.approved).then(|| {
            div()
                .flex_none()
                .px_4()
                .py_2()
                .child(
                    Alert::warning(
                        "vault-unapproved",
                        t_with("vaults.unapproved_body", &[("provider", &vault.provider_label)]),
                    )
                    .title(t("vaults.unapproved_title")),
                )
                .into_any_element()
        })
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
        let linked = !vault.is_local();
        let hint = match (linked, vault.approved, sealed) {
            (true, false, _) => t("vaults.state_unapproved_hint").to_owned(),
            (true, true, true) => t_with("vaults.state_linked_sealed_hint", &[("provider", &vault.provider_label)]),
            (false, _, true) => t("vaults.state_sealed_hint").to_owned(),
            (_, _, false) => t("vaults.state_unsealed_hint").to_owned(),
        };
        let main = if linked && !vault.approved {
            let setup = vault.clone();
            Button::new("setup-vault")
                .primary()
                .small()
                .label(t("vaults.setup_button"))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.open_setup(Purpose::Setup(setup.clone()), window, cx)),
                )
                .into_any_element()
        } else if sealed {
            Button::new("unseal")
                .primary()
                .small()
                .label(if linked { t("vaults.unseal_linked_button") } else { t("vaults.unseal_button") })
                .on_click(cx.listener(|this, _, window, cx| this.unseal(window, cx)))
                .into_any_element()
        } else {
            Button::new("seal")
                .outline()
                .small()
                .label(t("vaults.seal"))
                .tooltip(t("vaults.seal_tooltip"))
                .on_click(cx.listener(|this, _, _, cx| this.seal(cx)))
                .into_any_element()
        };
        let settings = vault.clone();
        let toolbar = h_flex()
            .flex_none()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(Self::state_tag(&vault))
            .child(div().flex_1().min_w_0().text_sm().text_color(theme.muted_foreground).child(hint))
            .child(main)
            .child(
                Button::new("rename-vault")
                    .ghost()
                    .small()
                    .label(t("vaults.rename_vault"))
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_rename(window, cx))),
            )
            .when(!linked, |this| {
                this.child(
                    Button::new("change-passphrase")
                        .ghost()
                        .small()
                        .label(t("vaults.change_passphrase"))
                        .on_click(cx.listener(|this, _, window, cx| this.prompt_change_passphrase(window, cx))),
                )
            })
            .when(linked && vault.approved, |this| {
                this.child(
                    Button::new("check-vault")
                        .ghost()
                        .small()
                        .label(t("vaults.check"))
                        .tooltip(t("vaults.check_tooltip"))
                        .on_click(cx.listener(|this, _, _, cx| this.check(cx))),
                )
                .child(
                    Button::new("vault-settings").ghost().small().label(t("vaults.settings")).on_click(cx.listener(
                        move |this, _, window, cx| this.open_setup(Purpose::Setup(settings.clone()), window, cx),
                    )),
                )
            });

        // A linked vault's secrets are changed in its provider; it says where, in place of an Add button.
        let add = if linked {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t_with("vaults.pulled_hint", &[("provider", &vault.provider_label)]))
                .into_any_element()
        } else {
            Button::new("add-secret")
                .outline()
                .small()
                .icon(IconName::Plus)
                .label(t("vaults.add_secret"))
                .disabled(sealed)
                .tooltip(if sealed { t("vaults.unseal_first_tooltip") } else { t("vaults.add_secret_tooltip") })
                .on_click(cx.listener(|this, _, window, cx| this.prompt_secret(None, window, cx)))
                .into_any_element()
        };
        let secrets_header = h_flex()
            .flex_none()
            .px_4()
            .py_2()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .child(page::section_label(t_with("vaults.secrets_count", &[("count", &vault.secrets.len())]), cx)),
            )
            .child(add);

        let rows = if vault.secrets.is_empty() {
            let body = if linked {
                t_with("vaults.no_pulled_body", &[("provider", &vault.provider_label)])
            } else {
                t("vaults.no_secrets_body").to_owned()
            };
            page::empty(t("vaults.no_secrets"), body)
        } else {
            v_flex()
                .id("secret-rows")
                .size_full()
                .overflow_y_scroll()
                .children(vault.secrets.iter().map(|name| self.render_secret(&vault, name, cx)))
                .into_any_element()
        };
        let usage = div()
            .flex_none()
            .px_4()
            .py_2()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(t_with("vaults.usage", &[("name", &vault.name)]));

        let footer = if linked {
            let model = self.model.read(cx);
            // A vault's file is not an entity in the engine's maps, so its state is looked up by path.
            let bar =
                EntityBar { git: model.git().files.get(&vault.path).copied(), ..EntityBar::of(model, Some(&vault.id)) };
            entity_bar(bar, cx, |this, action, window, cx| this.entity_action(action, window, cx)).into_any_element()
        } else {
            // Nothing of a local vault is synced: there is nothing to sync or revert, only to delete.
            h_flex()
                .flex_none()
                .gap_2()
                .px_4()
                .py_2()
                .border_t_1()
                .border_color(theme.border)
                .child(div().flex_1().text_xs().text_color(theme.muted_foreground).child(t("vaults.local_footer")))
                .child(
                    Button::new("delete-local-vault")
                        .ghost()
                        .small()
                        .label(t("vaults.delete"))
                        .on_click(cx.listener(|this, _, window, cx| this.confirm_delete(window, cx))),
                )
                .into_any_element()
        };

        v_flex()
            .size_full()
            .child(toolbar)
            .children(self.render_notice(&vault, cx))
            .child(secrets_header)
            .child(div().flex_1().min_h_0().child(rows))
            .child(usage)
            .child(footer)
            .into_any_element()
    }
}

impl Render for VaultsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header_tagged(
            t("vaults.title"),
            Some(Tag::warning().small().child(t("vaults.beta_tag"))),
            t("vaults.description"),
            Button::new("new-vault")
                .small()
                .icon(IconName::Plus)
                .label(t("vaults.new_vault"))
                .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
            cx,
        );
        let list = self.render_list(cx).into_any_element();
        let detail = self.render_detail(cx);
        let beta = div()
            .id("vaults-beta")
            .test_support()
            .flex_none()
            .px_4()
            .py_2()
            .child(Alert::warning("vaults-beta-alert", t("vaults.beta_body")).title(t("vaults.beta_title")));
        let body = v_flex().size_full().child(beta).child(div().flex_1().min_h_0().child(master_detail(
            Some(list),
            detail,
            cx,
        )));
        page::screen(header, body, cx)
    }
}
