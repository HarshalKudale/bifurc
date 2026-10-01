//! The short decisions: naming something, and confirming a deletion.
//!
//! Each is a dialog over the shell. They collect a value or a yes, then issue one engine command; what
//! changed arrives back as an engine event and the explorer rebuilds from it. Nothing is edited locally and
//! written back.

use std::rc::Rc;

use gpui_kit::component::{
    WindowExt as _,
    button::ButtonVariant,
    dialog::DialogButtonProps,
    form::{Field, Form},
    input::{Input, InputState},
    notification::Notification,
};
use gpui_kit::{AppContext as _, Context, Entity, ParentElement as _, Styled as _, Window};
use serde_json::{Value, json};

use super::Shell;
use crate::{
    engine::{call, describe},
    model::{NodeRef, RequestKind},
};

/// One labelled text field.
struct TextField {
    label: &'static str,
    input: Entity<InputState>,
}

impl Shell {
    /// Opens a dialog with the given fields. `submit` runs with their values, in order, when the user
    /// confirms (by the button or by Enter in a field).
    fn prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        title: &'static str,
        confirm: &'static str,
        fields: Vec<(&'static str, String, &'static str)>,
        submit: impl Fn(&mut Shell, Vec<String>, &mut Window, &mut Context<Shell>) + 'static,
    ) {
        let fields: Rc<Vec<TextField>> = Rc::new(
            fields
                .into_iter()
                .map(|(label, initial, placeholder)| TextField {
                    label,
                    input: cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(initial)),
                })
                .collect(),
        );
        let submit = Rc::new(submit);
        let shell = cx.weak_entity();

        // The dialog confirms on its button and on Enter in a field; both arrive here, and neither runs inside
        // an update of the shell, so the shell is reached through its handle.
        let confirm_now = {
            let fields = fields.clone();
            Rc::new(move |window: &mut Window, cx: &mut gpui_kit::App| {
                let values: Vec<String> = fields.iter().map(|f| f.input.read(cx).value().trim().to_owned()).collect();
                if values.iter().any(String::is_empty) {
                    window.push_notification(
                        Notification::warning("Fill in every field.").title("Can't continue yet"),
                        cx,
                    );
                    return false;
                }
                shell.update(cx, |shell, cx| submit(shell, values, window, cx)).ok();
                true
            })
        };

        // Deferred so a dialog opened by a command chosen from the palette opens after the palette closes.
        window.defer(cx, move |window, cx| {
            let first = fields.first().map(|f| f.input.clone());
            window.open_dialog(cx, {
                let fields = fields.clone();
                let confirm_now = confirm_now.clone();
                move |dialog, _, _| {
                    let confirm_now = confirm_now.clone();
                    dialog
                        .title(title)
                        .w_96()
                        .child(fields.iter().fold(Form::new(), |form, field| {
                            form.child(Field::new().label(field.label).child(Input::new(&field.input)))
                        }))
                        .button_props(
                            DialogButtonProps::default()
                                .ok_text(confirm)
                                .show_cancel(true)
                                .on_ok(move |_, window, cx| confirm_now(window, cx)),
                        )
                }
            });
            if let Some(input) = first {
                window.defer(cx, move |window, cx| input.update(cx, |state, cx| state.focus(window, cx)));
            }
        });
    }

    pub fn prompt_new_folder(
        &mut self,
        kind: RequestKind,
        parent: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt(
            window,
            cx,
            "New folder",
            "Create",
            vec![("Name", String::new(), "Folder name")],
            move |shell, values, _, cx| {
                let client = shell.model.read(cx).client().clone();
                let payload = json!({
                    "kind": kind.folder_kind(),
                    "name": values[0],
                    "parentId": parent,
                    "workspaceId": shell.model.read(cx).workspace_id(),
                });
                call(&client, cx, "folder.add", payload, |shell, outcome, cx| {
                    if let Err(error) = outcome {
                        shell.notify(format!("Couldn't create the folder. {}", describe(&error)), cx);
                    }
                });
            },
        );
    }

    pub fn prompt_rename(&mut self, node: NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.name_of(&node, cx);
        if let NodeRef::Request { kind, id } = &node
            && self.has_open_tab(*kind, id)
        {
            window.push_notification(
                Notification::info("This request is open. Change its name in the tab and save.")
                    .title("Rename in the tab"),
                cx,
            );
            return;
        }
        self.prompt(window, cx, "Rename", "Rename", vec![("Name", current, "Name")], move |shell, values, _, cx| {
            let client = shell.model.read(cx).client().clone();
            match node.clone() {
                NodeRef::Folder { kind, id } => {
                    let payload = json!({ "kind": kind.folder_kind(), "id": id, "name": values[0] });
                    call(&client, cx, "folder.rename", payload, |shell, outcome, cx| {
                        if let Err(error) = outcome {
                            shell.notify(format!("Couldn't rename the folder. {}", describe(&error)), cx);
                        }
                    });
                }
                NodeRef::Request { kind, id } => shell.rename_request(kind, id, values[0].clone(), cx),
            }
        });
    }

    /// Renames a saved request: read it, change the name, write it back. The change is one field of the
    /// entity the engine returned, so nothing else about the request is touched.
    fn rename_request(&mut self, kind: RequestKind, id: String, name: String, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace_id = model.workspace_id().to_owned();
        let load = json!({ "workspaceId": workspace_id, "kind": kind.entity_kind(), "id": id });
        call(&client.clone(), cx, "entity.load", load, move |shell, outcome, cx| {
            let entity = outcome.ok().and_then(|v| v.get("entity").cloned()).and_then(|e| match e {
                Value::Object(mut map) => {
                    map.insert("name".into(), json!(name));
                    Some(Value::Object(map))
                }
                _ => None,
            });
            let Some(entity) = entity else {
                shell.notify("Couldn't rename the request: it could not be read.".to_owned(), cx);
                return;
            };
            let update = json!({ "kind": kind.entity_kind(), "entity": entity });
            call(&client, cx, "entity.update", update, |shell, outcome, cx| {
                if let Err(error) = outcome {
                    shell.notify(format!("Couldn't rename the request. {}", describe(&error)), cx);
                }
            });
        });
    }

    pub fn confirm_delete(&mut self, node: NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_of(&node, cx);
        let (what, consequence) = match &node {
            NodeRef::Folder { .. } => ("folder", "The folder and everything in it will be deleted."),
            NodeRef::Request { .. } => ("request", "This can't be undone."),
        };
        let shell = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let shell = shell.clone();
                let node = node.clone();
                alert
                    .title(format!("Delete {what} “{name}”?"))
                    .description(consequence)
                    .confirm()
                    .ok_text("Delete")
                    .ok_variant(ButtonVariant::Danger)
                    .on_ok(move |_, _, cx| {
                        let node = node.clone();
                        shell.update(cx, |shell, cx| shell.delete_node(node, cx)).ok();
                        true
                    })
            });
        });
    }

    fn delete_node(&mut self, node: NodeRef, cx: &mut Context<Self>) {
        let client = self.model.read(cx).client().clone();
        let (action, payload) = match &node {
            NodeRef::Folder { kind, id } => ("folder.delete", json!({ "kind": kind.folder_kind(), "id": id })),
            NodeRef::Request { kind, id } => ("entity.delete", json!({ "kind": kind.entity_kind(), "id": id })),
        };
        call(&client, cx, action, payload, |shell, outcome, cx| {
            if let Err(error) = outcome {
                shell.notify(format!("Couldn't delete. {}", describe(&error)), cx);
            }
        });
    }

    pub fn prompt_new_mapping(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt(
            window,
            cx,
            "New mapping",
            "Create",
            vec![("Domain", String::new(), "api.localhost"), ("Target", String::new(), "localhost:3000")],
            |shell, values, _, cx| {
                let client = shell.model.read(cx).client().clone();
                let payload = json!({
                    "kind": "mappings",
                    "workspaceId": shell.model.read(cx).workspace_id(),
                    "entity": { "domain": values[0], "target": values[1], "enabled": true },
                });
                call(&client, cx, "entity.create", payload, |shell, outcome, cx| {
                    if let Err(error) = outcome {
                        shell.notify(format!("Couldn't create the mapping. {}", describe(&error)), cx);
                    }
                });
            },
        );
    }

    pub fn confirm_delete_mapping(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let domain = self
            .model
            .read(cx)
            .config()
            .mappings
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.domain.clone())
            .unwrap_or_else(|| "this mapping".to_owned());
        let shell = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let shell = shell.clone();
                let id = id.clone();
                alert
                    .title(format!("Delete mapping “{domain}”?"))
                    .description("Requests to this name will no longer be routed.")
                    .confirm()
                    .ok_text("Delete")
                    .ok_variant(ButtonVariant::Danger)
                    .on_ok(move |_, _, cx| {
                        let id = id.clone();
                        shell
                            .update(cx, |shell, cx| {
                                let client = shell.model.read(cx).client().clone();
                                let payload = json!({ "kind": "mappings", "id": id });
                                call(&client, cx, "entity.delete", payload, |shell, outcome, cx| {
                                    if let Err(error) = outcome {
                                        shell.notify(format!("Couldn't delete the mapping. {}", describe(&error)), cx);
                                    }
                                });
                            })
                            .ok();
                        true
                    })
            });
        });
    }

    /// The name shown for a tree node, for dialog titles and as the starting text of a rename.
    fn name_of(&self, node: &NodeRef, cx: &Context<Self>) -> String {
        let config = self.model.read(cx).config();
        match node {
            NodeRef::Folder { kind, id } => config.folder(*kind, id).map(|f| f.name.clone()),
            NodeRef::Request { kind, id } => config.request(*kind, id).map(|r| r.name.clone()),
        }
        .unwrap_or_default()
    }

    /// A message for the user, from somewhere that has no window in hand.
    pub(super) fn notify(&mut self, text: String, cx: &mut Context<Self>) {
        self.model.update(cx, |_, cx| cx.emit(crate::app_model::ModelEvent::Notice(text)));
    }
}
