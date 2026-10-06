//! The Mappings screen: which `.localhost` name goes to which local target.
//!
//! A mapping is a saved entity. The list comes from the model's snapshot; turning one on or off, creating,
//! editing and deleting are engine commands addressed by id.

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    switch::Switch,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Map, json};

use crate::{
    app_model::{AppModel, Deleted},
    components::{
        dialogs::{PromptField, confirm_danger, delete_consequence, prompt},
        page,
    },
    engine::{call, failure, patch_entity},
    model::{config::MappingInfo, git::FileState},
};

const DOMAIN_SUFFIX: &str = ".localhost";

/// The full name for what the user typed: `api` and `api.localhost` both mean `api.localhost`.
fn full_domain(typed: &str) -> String {
    let name = typed.trim().to_lowercase();
    if name.ends_with(DOMAIN_SUFFIX) { name } else { format!("{name}{DOMAIN_SUFFIX}") }
}

fn subdomain_of(domain: &str) -> &str {
    domain.strip_suffix(DOMAIN_SUFFIX).unwrap_or(domain)
}

pub struct MappingsScreen {
    model: Entity<AppModel>,
    filter: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl MappingsScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter mappings"));
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self { model, filter, _subscriptions: subscriptions }
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
    }

    pub fn prompt_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            "New mapping",
            "Create",
            vec![
                PromptField::new("Subdomain", "api  (becomes api.localhost)"),
                PromptField::new("Target", "localhost:3000"),
                PromptField::new("Label", "Optional name").optional(),
            ],
            |this, values, _, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                let mut entity = Map::new();
                entity.insert("domain".into(), json!(full_domain(&values[0])));
                entity.insert("target".into(), json!(values[1]));
                entity.insert("enabled".into(), json!(true));
                if !values[2].is_empty() {
                    entity.insert("label".into(), json!(values[2]));
                }
                let payload = json!({ "kind": "mappings", "workspaceId": model.workspace_id(), "entity": entity });
                call(&client, cx, "entity.create", payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report("Couldn't create the mapping.", message, cx);
                    }
                });
            },
        );
    }

    fn prompt_edit(&mut self, mapping: MappingInfo, window: &mut Window, cx: &mut Context<Self>) {
        let id = mapping.id.clone();
        prompt(
            cx.weak_entity(),
            window,
            cx,
            format!("Edit “{}”", mapping.domain),
            "Save",
            vec![
                PromptField::new("Subdomain", "api").initial(subdomain_of(&mapping.domain)),
                PromptField::new("Target", "localhost:3000").initial(mapping.target.clone()),
                PromptField::new("Label", "Optional name")
                    .initial(mapping.label.clone().unwrap_or_default())
                    .optional(),
            ],
            move |this, values, _, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                let workspace_id = model.workspace_id().to_owned();
                let mut patch = Map::new();
                patch.insert("domain".into(), json!(full_domain(&values[0])));
                patch.insert("target".into(), json!(values[1]));
                patch.insert("label".into(), json!(values[2]));
                patch_entity(&client, cx, &workspace_id, "mappings", &id, patch, |this, outcome, cx| {
                    if let Err(message) = outcome {
                        this.report("Couldn't save the mapping.", message, cx);
                    }
                });
            },
        );
    }

    fn confirm_delete(&mut self, mapping: MappingInfo, window: &mut Window, cx: &mut Context<Self>) {
        let id = mapping.id.clone();
        let consequence = delete_consequence(self.model.read(cx).git().entity_state(&id));
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Delete mapping “{}”?", mapping.domain),
            consequence,
            "Delete",
            move |this, _, cx| {
                let client = this.model.read(cx).client().clone();
                let payload = json!({ "kind": "mappings", "id": id });
                call(&client, cx, "entity.delete", payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report("Couldn't delete the mapping.", message, cx);
                    }
                });
            },
        );
    }

    /// Commits the changed mapping files (and pushes, when the workspace has a remote).
    fn sync_paths(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.publish(paths, cx));
    }

    fn confirm_revert(&mut self, paths: Vec<String>, what: String, window: &mut Window, cx: &mut Context<Self>) {
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Revert {what}?"),
            "Go back to the last synced version. Mappings that were never synced are removed.",
            "Revert",
            move |this, _, cx| this.model.update(cx, |model, cx| model.revert_paths(paths.clone(), cx)),
        );
    }

    fn render_row(&self, mapping: &MappingInfo, scheme: &str, port: u16, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let state = self.model.read(cx).git().entity_state(&mapping.id);
        let path = self.model.read(cx).git().entity_path(&mapping.id).map(str::to_owned);
        let busy = self.model.read(cx).is_publishing();
        let state_label = match state {
            Some(FileState::New) => "New",
            Some(FileState::Modified) => "Modified",
            Some(FileState::Deleted) => "Deleted",
            Some(FileState::Clean) | None => "",
        };
        let changed = !state_label.is_empty();
        let deleted = mapping.deleted;
        let model = self.model.clone();
        let deleted_model = self.model.clone();
        let toggle_id = mapping.id.clone();
        let this = cx.weak_entity();
        let (edit, delete) = (mapping.clone(), mapping.clone());
        // The port is part of the address unless it is the default one of the scheme.
        let default_port = if scheme == "https" { 443 } else { 80 };
        let url = if port == default_port || port == 0 {
            format!("{scheme}://{}", mapping.domain)
        } else {
            format!("{scheme}://{}:{port}", mapping.domain)
        };
        let key = |part: &str| gpui_kit::ElementId::Name(format!("mapping-{part}-{}", mapping.id).into());

        h_flex()
            .id(key("row"))
            .gap_3()
            .px_4()
            .py_2()
            .text_sm()
            .border_b_1()
            .border_color(theme.border)
            .child(div().w_12().flex_none().children((!deleted).then(|| {
                Switch::new(key("enabled")).checked(mapping.enabled).small().on_change(move |checked: &bool, _, cx| {
                    let checked = *checked;
                    model.update(cx, |m, cx| m.set_enabled("mappings", &toggle_id, checked, cx));
                })
            })))
            .child({
                let domain = div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .font_family(theme.mono_font_family.clone())
                    .child(mapping.domain.clone());
                // Deleted, and waiting to be confirmed or reverted: still listed, struck through.
                if deleted { domain.line_through().text_color(theme.muted_foreground) } else { domain }
            })
            .child(Icon::new(IconName::ArrowRight).small().text_color(theme.muted_foreground))
            .child(if deleted {
                // A deleted mapping has no target to show: what can be done with it goes here.
                let (revert_model, confirm_model) = (deleted_model.clone(), deleted_model);
                let (revert_id, confirm_id) = (mapping.id.clone(), mapping.id.clone());
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(Button::new(key("revert-delete")).small().label("Revert delete").on_click(
                        move |_, _, cx| {
                            let target = Deleted::Entity { kind: "mappings", id: revert_id.clone() };
                            revert_model.update(cx, |model, cx| model.revert_delete(target, cx));
                        },
                    ))
                    .child(Button::new(key("confirm-delete")).small().label("Confirm delete").on_click(
                        move |_, _, cx| {
                            let target = Deleted::Entity { kind: "mappings", id: confirm_id.clone() };
                            confirm_model.update(cx, |model, cx| model.confirm_delete(target, cx));
                        },
                    ))
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .font_family(theme.mono_font_family.clone())
                    .child(mapping.target.clone())
                    .into_any_element()
            })
            .child({
                let label = div()
                    .w_48()
                    .flex_none()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(theme.muted_foreground)
                    .child(mapping.label.clone().unwrap_or_default());
                if deleted { label.line_through() } else { label }
            })
            .child(div().w_24().flex_none().text_xs().text_color(theme.muted_foreground).child(state_label))
            .child(div().w_8().flex_none().children((!deleted).then(|| {
                Button::new(key("actions")).icon(IconName::Ellipsis).ghost().small().dropdown_menu(
                    move |menu, _, _| {
                        let (edit_view, delete_view) = (this.clone(), this.clone());
                        let (sync_view, revert_view) = (this.clone(), this.clone());
                        let (sync_path, revert_path) = (path.clone(), path.clone());
                        let revert_name = delete.domain.clone();
                        let (edit, delete, url) = (edit.clone(), delete.clone(), url.clone());
                        menu.item(PopupMenuItem::new("Edit…").on_click(move |_, window, cx| {
                            let mapping = edit.clone();
                            edit_view.update(cx, |this, cx| this.prompt_edit(mapping, window, cx)).ok();
                        }))
                        .item(PopupMenuItem::new("Copy address").on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Sync").disabled(!changed || busy).on_click(move |_, _, cx| {
                            if let Some(path) = sync_path.clone() {
                                sync_view.update(cx, |this, cx| this.sync_paths(vec![path], cx)).ok();
                            }
                        }))
                        .item(PopupMenuItem::new("Revert…").disabled(!changed || busy).on_click(
                            move |_, window, cx| {
                                if let Some(path) = revert_path.clone() {
                                    let what = format!("“{revert_name}”");
                                    revert_view
                                        .update(cx, |this, cx| this.confirm_revert(vec![path], what, window, cx))
                                        .ok();
                                }
                            },
                        ))
                        .separator()
                        .item(PopupMenuItem::new("Delete…").on_click(
                            move |_, window, cx| {
                                let mapping = delete.clone();
                                delete_view.update(cx, |this, cx| this.confirm_delete(mapping, window, cx)).ok();
                            },
                        ))
                    },
                )
            })))
            .into_any_element()
    }
}

impl Render for MappingsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let config = self.model.read(cx).config();
        // With HTTPS on, the same port serves names over HTTPS.
        let scheme = if config.settings.tls_enabled { "https" } else { "http" };
        let port = config.settings.proxy_port;
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        let all = config.mappings.len();
        let mappings: Vec<MappingInfo> = config
            .mappings
            .iter()
            .filter(|m| {
                needle.is_empty()
                    || m.domain.to_lowercase().contains(&needle)
                    || m.target.to_lowercase().contains(&needle)
                    || m.label.as_deref().is_some_and(|label| label.to_lowercase().contains(&needle))
            })
            .cloned()
            .collect();

        let model = self.model.read(cx);
        let changed = model.git().rollup(&["mappings/"]).changed;
        let busy = model.is_publishing();
        let (sync_paths, revert_paths) = (changed.clone(), changed.clone());
        let header = page::header(
            "Mappings",
            "Route a .localhost name to a local server through the proxy.",
            h_flex()
                .gap_2()
                .child(div().w_56().child(
                    Input::new(&self.filter).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ))
                .child(
                    Button::new("revert-mappings")
                        .small()
                        .label("Revert")
                        .tooltip("Put every changed mapping back to its last synced version")
                        .disabled(changed.is_empty() || busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let paths = revert_paths.clone();
                            let what = match paths.len() {
                                1 => "1 changed mapping".to_owned(),
                                count => format!("{count} changed mappings"),
                            };
                            this.confirm_revert(paths, what, window, cx);
                        })),
                )
                .child(
                    Button::new("sync-mappings")
                        .small()
                        .label("Sync")
                        .tooltip("Commit the changed mappings (and push them, when there is a remote)")
                        .loading(busy)
                        .disabled(changed.is_empty() || busy)
                        .on_click(cx.listener(move |this, _, _, cx| this.sync_paths(sync_paths.clone(), cx))),
                )
                .child(
                    Button::new("new-mapping")
                        .small()
                        .icon(IconName::Plus)
                        .label("New mapping")
                        .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
                ),
            cx,
        );

        let body = if all == 0 {
            page::empty_with_action(
                "No mappings",
                "A mapping sends a name such as api.localhost to a server running on your machine.",
                Button::new("empty-new-mapping")
                    .label("New mapping")
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_new(window, cx))),
            )
        } else if mappings.is_empty() {
            page::empty("No matches", "No mapping matches the filter.")
        } else {
            let rows: Vec<gpui_kit::AnyElement> =
                mappings.iter().map(|mapping| self.render_row(mapping, scheme, port, cx)).collect();
            v_flex()
                .size_full()
                .child(
                    h_flex()
                        .flex_none()
                        .gap_3()
                        .px_4()
                        .py_2()
                        .text_xs()
                        .font_medium()
                        .text_color(theme.muted_foreground)
                        .border_b_1()
                        .border_color(theme.border)
                        .child(div().w_12().flex_none().child("On"))
                        .child(div().flex_1().child("Name"))
                        // The lane the arrow occupies in the rows.
                        .child(div().size_4().flex_none())
                        .child(div().flex_1().child("Target"))
                        .child(div().w_48().flex_none().child("Label"))
                        .child(div().w_24().flex_none().child("Status"))
                        .child(div().w_8().flex_none()),
                )
                .child(v_flex().id("mapping-rows").flex_1().min_h_0().overflow_y_scroll().children(rows))
                .into_any_element()
        };
        page::screen(header, body, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subdomain_and_a_full_name_mean_the_same_mapping() {
        assert_eq!(full_domain(" API "), "api.localhost");
        assert_eq!(full_domain("api.localhost"), "api.localhost");
        assert_eq!(subdomain_of("api.localhost"), "api");
        assert_eq!(subdomain_of("odd.name"), "odd.name");
    }
}
