//! The Flows screen: the tree of saved flows, and the canvas the open one is drawn and run on.
//!
//! The tree is the model's snapshot. The canvas holds an *edit* of the open flow, written over the engine's
//! copy a moment after each change. The engine says what is wrong with the flow and what each node offers
//! (`flow.validate`), runs it (`flow.run`), and reports the run as it goes; this screen draws what it is
//! told. There is no panel beside the canvas: everything about a node is in the node.

mod canvas;
mod editor;
mod json_view;
mod load;
mod tree;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Map, Value, json};

use self::{
    canvas::{CanvasEvent, FlowCanvas},
    editor::{EditorEvent, NodeEditor},
    tree::{FlowTree, FlowTreeEvent, deleted_target, is_deleted, locked_to},
};
use crate::{
    app_model::{AppModel, ModelEvent},
    components::{
        Autosave, EntityAction, EntityBar,
        dialogs::{
            FOLDER_DELETE_CONSEQUENCE, PromptField, confirm_danger, delete_consequence, needs_update, prompt, resolve_deleted,
        },
        entity_bar, page,
    },
    engine::{call, call_in, failure, patch_entity},
    i18n::{t, t_with},
    model::{
        flow::{Described, FlowDoc, FlowNode, LoadView, NodeKind, RunView},
        tree::{TreeKey, folder_path},
    },
    shell::ActiveTarget,
};

pub struct FlowsScreen {
    model: Entity<AppModel>,
    /// The flow that is open, and its edit once it has been read.
    selected: Option<String>,
    doc: Option<FlowDoc>,
    /// The open flow as the engine has it: what was read, or last written.
    baseline: Value,
    saving: bool,
    /// Run the flow once the write in flight has landed: a run is of the saved flow.
    run_when_saved: bool,
    autosave: Autosave,
    /// The open flow was reverted on disk: it is read again whatever the edit holds.
    reload_due: bool,
    name: Entity<InputState>,
    described: Described,
    validating: bool,
    validate_again: bool,
    run: Option<RunView>,
    /// The flow's latest load test, and whether it is shown in place of the canvas.
    load: Option<LoadView>,
    show_load: bool,
    /// How many runs Run starts at once. More than one is a load test.
    instances: Entity<InputState>,
    selected_node: Option<String>,
    tree: Entity<FlowTree>,
    canvas: Entity<FlowCanvas>,
    editor: Entity<NodeEditor>,
    _subscriptions: Vec<Subscription>,
}

impl FlowsScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("flows.name.placeholder")));
        let instances = cx.new(|cx| InputState::new(window, cx).default_value("1"));
        let tree = cx.new(|cx| FlowTree::new(model.clone(), window, cx));
        let editor = cx.new(|cx| NodeEditor::new(model.clone(), window, cx));
        let canvas = cx.new(|cx| FlowCanvas::new(editor.clone(), cx));
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| this.follow_model(window, cx)),
            cx.subscribe_in(&model, window, Self::on_model_event),
            cx.subscribe(&name, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let name = input.read(cx).value().trim().to_owned();
                    this.edit(cx, |doc| doc.name = name);
                }
            }),
            cx.subscribe_in(&tree, window, Self::on_tree_event),
            cx.subscribe_in(&canvas, window, Self::on_canvas_event),
            cx.subscribe(&editor, Self::on_editor_event),
        ];
        let mut this = Self {
            model,
            selected: None,
            doc: None,
            baseline: Value::Null,
            saving: false,
            run_when_saved: false,
            autosave: Autosave::new(window),
            reload_due: false,
            name,
            described: Described::default(),
            validating: false,
            validate_again: false,
            run: None,
            load: None,
            show_load: false,
            instances,
            selected_node: None,
            tree,
            canvas,
            editor,
            _subscriptions: subscriptions,
        };
        this.follow_model(window, cx);
        this
    }

    fn is_dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|doc| doc.to_value() != self.baseline)
    }

    /// The folder the flow with `id` is in, as the engine last listed it.
    fn folder_of(&self, id: &str, cx: &gpui_kit::App) -> Option<String> {
        self.model.read(cx).config().flows.iter().find(|flow| flow.id == id).and_then(|flow| flow.folder_id.clone())
    }

    /// The open flow, for the window's breadcrumb. Its name is typed in this screen's own bar.
    pub fn active_target(&self, cx: &gpui_kit::App) -> Option<ActiveTarget> {
        let id = self.doc.as_ref().map(|doc| doc.id.clone())?;
        let name = self.name.read(cx).value();
        let title = if name.is_empty() { SharedString::from(t("flows.untitled")) } else { name };
        let folders = self
            .folder_of(&id, cx)
            .map(|folder| folder_path(&self.model.read(cx).config().flow_folder_rows(), &folder))
            .unwrap_or_default();
        Some(ActiveTarget { folders, ..ActiveTarget::named(title, Some(id), self.name.clone()) })
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
    }

    /// Keeps a flow open that exists: the one that is, or else the first there is.
    fn follow_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let flows = &self.model.read(cx).config().flows;
        // Nor is one a newer Bifurc saved kept open: the engine does not read it.
        let usable = |flow: &&crate::model::config::FlowInfo| !flow.deleted && flow.locked.is_none();
        let still_there = self.selected.as_deref().is_some_and(|id| flows.iter().filter(usable).any(|flow| flow.id == id));
        if !still_there {
            match flows.iter().find(usable).map(|flow| flow.id.clone()) {
                Some(id) => self.open(id, window, cx),
                None => self.close(cx),
            }
        }
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.autosave.cancel();
        (self.selected, self.doc, self.run, self.selected_node) = (None, None, None, None);
        (self.load, self.show_load) = (None, false);
        self.baseline = Value::Null;
        self.described = Described::default();
        self.tree.update(cx, |tree, cx| tree.reveal(None, cx));
        self.sync_views(cx);
    }

    /// Read a flow from the engine and show it.
    fn open(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.close(cx);
        self.selected = Some(id.clone());
        self.tree.update(cx, |tree, cx| tree.reveal(Some(&id), cx));
        let model = self.model.read(cx);
        let (client, workspace_id) = (model.client().clone(), model.workspace_id().to_owned());
        let load = json!({ "workspaceId": workspace_id, "kind": "flows", "id": id });
        call_in(&client, window, cx, "entity.load", load, move |this, outcome, window, cx| {
            // Another flow was picked while this one was being read.
            if this.selected.as_deref() != Some(id.as_str()) {
                return;
            }
            let entity = outcome.as_ref().ok().and_then(|answer| answer.get("entity"));
            match entity.map(FlowDoc::from_value) {
                Some(Ok(doc)) => this.show(doc, window, cx),
                Some(Err(error)) => this.report(t("flows.error.read"), error, cx),
                None => this.report(
                    t("flows.error.open"),
                    failure(&outcome).unwrap_or_else(|| t("flows.open.not_there").to_owned()),
                    cx,
                ),
            }
        });
        let status = json!({ "workspaceId": workspace_id, "id": self.selected });
        call(&client, cx, "flow.status", status, |this, outcome, cx| {
            let report = outcome.unwrap_or(Value::Null);
            if report.get("flowId").and_then(Value::as_str) == this.selected.as_deref() && this.run.is_none() {
                this.run = RunView::from_report(&report);
                this.sync_views(cx);
            }
        });
        let status = json!({ "workspaceId": workspace_id, "id": self.selected });
        call(&client, cx, "flow.loadtest.status", status, |this, outcome, cx| {
            let report = outcome.unwrap_or(Value::Null);
            if report.get("flowId").and_then(Value::as_str) == this.selected.as_deref() && this.load.is_none() {
                this.take_load(&report, cx);
                // A load test that is going on is what there is to see.
                this.show_load = this.is_load_running();
            }
        });
    }

    fn show(&mut self, doc: FlowDoc, window: &mut Window, cx: &mut Context<Self>) {
        self.baseline = doc.to_value();
        self.name.update(cx, |state, cx| state.set_value(doc.name.clone(), window, cx));
        self.doc = Some(doc);
        self.selected_node = None;
        self.editor.update(cx, |editor, cx| editor.show(None, &[], window, cx));
        self.canvas.update(cx, |canvas, cx| {
            canvas.reset_view(cx);
            canvas.set_selected(None, cx);
        });
        self.validate(cx);
        self.sync_views(cx);
    }

    /// Selects the flow `by` places away from the open one, wrapping around.
    pub fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.model.read(cx).config().flows.iter().map(|flow| flow.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let at = ids.iter().position(|id| Some(id) == self.selected.as_ref());
        let next = at.map_or(0, |ix| (ix as isize + by).rem_euclid(ids.len() as isize) as usize);
        self.select(ids[next].clone(), window, cx);
    }

    pub fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(flow) = self.model.read(cx).config().flows.iter().find(|flow| flow.id == id).cloned() else { return };
        // A deleted flow is brought back or deleted for good, not edited.
        if flow.deleted {
            let deleted = deleted_target(&TreeKey::Item(flow.id.clone()));
            resolve_deleted(self.model.clone(), deleted, flow.name.clone(), window, cx);
            return;
        }
        // Neither is one a newer Bifurc saved: the engine does not read it until this one is updated.
        if let Some(version) = flow.locked.clone() {
            needs_update(flow.name.clone(), version, window, cx);
            return;
        }
        // An edit that is still waiting is written before the flow leaves the screen.
        self.autosave.cancel();
        self.save(cx);
        self.open(id, window, cx);
    }

    /// Change the flow and have the change checked, drawn and written.
    fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut FlowDoc)) {
        let Some(doc) = self.doc.as_mut() else { return };
        change(doc);
        self.validate(cx);
        self.sync_views(cx);
        self.autosave.schedule(cx, |this, _, cx| this.save(cx));
    }

    /// Writes the edit over the saved flow. A write asked for while another is in flight is not lost: the one
    /// in flight looks for a newer edit when it lands.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc.as_ref().filter(|_| !self.saving && self.is_dirty()) else { return };
        let (flow_id, saved) = (doc.id.clone(), doc.to_value());
        // The flow stays in the folder it is in now, wherever it was when it was read.
        let mut entity = saved.clone();
        entity["folderId"] = json!(self.folder_of(&flow_id, cx));
        self.saving = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let update = json!({ "kind": "flows", "entity": entity });
        call(&client, cx, "entity.update", update, move |this, outcome, cx| {
            this.saving = false;
            cx.notify();
            if let Some(message) = failure(&outcome) {
                this.run_when_saved = false;
                return this.report(t("flows.error.save"), message, cx);
            }
            if this.selected.as_deref() != Some(flow_id.as_str()) {
                return;
            }
            this.baseline = saved;
            if this.is_dirty() {
                this.save(cx);
            } else if std::mem::take(&mut this.run_when_saved) {
                this.start_run(cx);
            }
        });
    }

    /// Ask the engine what it makes of the flow as it is edited. A change made while it answers is asked
    /// about when the answer is in.
    fn validate(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc.as_ref() else { return };
        if self.validating {
            self.validate_again = true;
            return;
        }
        self.validating = true;
        let model = self.model.read(cx);
        let (client, flow_id) = (model.client().clone(), doc.id.clone());
        let payload = json!({ "workspaceId": model.workspace_id(), "flow": doc.to_value() });
        call(&client, cx, "flow.validate", payload, move |this, outcome, cx| {
            this.validating = false;
            if this.selected.as_deref() == Some(flow_id.as_str())
                && let Ok(answer) = &outcome
            {
                this.described = Described::from_value(answer);
                this.sync_views(cx);
            }
            if std::mem::take(&mut this.validate_again) {
                this.validate(cx);
            }
        });
    }

    /// What a node is handed, as far as is known before it runs again: what its last run was, or what the
    /// node that runs it last produced or holds. A part of it is chosen from this.
    fn handed_to(&self, node_id: &str) -> Value {
        let Some(doc) = &self.doc else { return Value::Null };
        let ran = |id: &str| self.run.as_ref().and_then(|run| run.nodes.get(id));
        if let Some(run) = ran(node_id).filter(|run| run.runs > 0 && !run.input.is_null()) {
            return run.input.clone();
        }
        let Some(edge) = doc.edges.iter().find(|edge| edge.to == node_id && !edge.is_wire()) else {
            return Value::Null;
        };
        let produced = ran(&edge.from).and_then(|run| run.output(&edge.port)).cloned();
        produced.unwrap_or_else(|| match doc.node(&edge.from) {
            Some(node) if node.kind == NodeKind::Sample => node.value.clone(),
            Some(node) if node.kind == NodeKind::Start => doc.default_inputs(),
            _ => Value::Null,
        })
    }

    /// Hand the canvas what it draws: the flow, what the engine says of it, and the run.
    fn sync_views(&mut self, cx: &mut Context<Self>) {
        let (doc, described, run) = (self.doc.clone(), self.described.clone(), self.run.clone());
        self.canvas.update(cx, |canvas, cx| canvas.set_graph(doc, described, run, cx));
        if let Some(id) = &self.selected_node {
            let handed = self.handed_to(id);
            self.editor.update(cx, |editor, cx| editor.set_handed(handed, cx));
        }
        cx.notify();
    }

    fn select_node(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_node = id;
        let node = self.selected_node.as_deref().zip(self.doc.as_ref()).and_then(|(id, doc)| doc.node(id).cloned());
        let inputs = self.doc.as_ref().map(|doc| doc.inputs.clone()).unwrap_or_default();
        self.editor.update(cx, |editor, cx| editor.show(node, &inputs, window, cx));
        self.canvas.update(cx, |canvas, cx| canvas.set_selected(self.selected_node.clone(), cx));
        self.sync_views(cx);
    }

    /// Put a node as it was edited in the flow. Where it is on the canvas is the canvas's to say.
    fn replace_node(&mut self, mut node: FlowNode, cx: &mut Context<Self>) {
        self.edit(cx, |doc| {
            // A branch that is gone takes the connections that left by it.
            if node.kind == NodeKind::Condition {
                let kept = |port: &str| port == "else" || node.branches.iter().any(|branch| branch.id == port);
                doc.edges.retain(|edge| edge.from != node.id || kept(&edge.port));
            }
            if let Some(slot) = doc.node_mut(&node.id) {
                (node.x, node.y, node.width, node.height) = (slot.x, slot.y, slot.width, slot.height);
                *slot = node;
            }
        });
    }

    fn on_canvas_event(
        &mut self,
        _: &Entity<FlowCanvas>,
        event: &CanvasEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.clone() {
            CanvasEvent::Selected(id) => self.select_node(id, window, cx),
            CanvasEvent::Moved { id, x, y } => self.edit(cx, |doc| {
                if let Some(node) = doc.node_mut(&id) {
                    (node.x, node.y) = (x, y);
                }
            }),
            CanvasEvent::Resized { id, width, height } => self.edit(cx, |doc| {
                if let Some(node) = doc.node_mut(&id) {
                    (node.width, node.height) = (width, height);
                }
            }),
            CanvasEvent::Connect { from, port, to } => self.edit(cx, |doc| {
                doc.connect(&from, &port, &to);
            }),
            CanvasEvent::Wire { from, port, to, input } => self.edit(cx, |doc| {
                doc.wire(&from, &port, &to, &input);
            }),
            CanvasEvent::WirePath { edge_id, path } => self.edit(cx, |doc| {
                if let Some(edge) = doc.edges.iter_mut().find(|edge| edge.id == edge_id) {
                    edge.path = path;
                }
            }),
            CanvasEvent::Disconnect(edge_id) => self.edit(cx, |doc| doc.disconnect(&edge_id)),
            CanvasEvent::Delete(id) => self.delete_node(&id, window, cx),
            CanvasEvent::Duplicate(id) => {
                let mut copy = None;
                self.edit(cx, |doc| copy = doc.duplicate_node(&id));
                if copy.is_some() {
                    self.select_node(copy, window, cx);
                }
            }
            CanvasEvent::Add { kind, x, y, link } => {
                let mut added = None;
                self.edit(cx, |doc| {
                    let id = doc.add_node(kind, x, y);
                    if let Some((from, port)) = &link {
                        doc.connect(from, port, &id);
                    }
                    added = Some(id);
                });
                self.select_node(added, window, cx);
                self.canvas.update(cx, |canvas, cx| canvas.focus(window, cx));
            }
            CanvasEvent::Edited(node) => {
                self.editor.update(cx, |editor, _| editor.follow((*node).clone()));
                self.replace_node(*node, cx);
            }
        }
    }

    /// Remove a node and its connections. The start node stays.
    fn delete_node(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let removable = |doc: &FlowDoc| doc.node(id).is_some_and(|node| node.kind != NodeKind::Start);
        if !self.doc.as_ref().is_some_and(removable) {
            return;
        }
        self.select_node(None, window, cx);
        self.edit(cx, |doc| doc.remove_node(id));
    }

    fn on_editor_event(&mut self, _: Entity<NodeEditor>, event: &EditorEvent, cx: &mut Context<Self>) {
        match event.clone() {
            EditorEvent::Changed(node) => self.replace_node(*node, cx),
            EditorEvent::Inputs(inputs) => self.edit(cx, |doc| doc.inputs = inputs),
        }
    }

    fn on_model_event(
        &mut self,
        _: &Entity<AppModel>,
        event: &ModelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let about_open_flow = |payload: &Value, this: &Self, cx: &Context<Self>| {
            payload.get("flowId").and_then(Value::as_str) == this.selected.as_deref()
                && payload.get("workspaceId").and_then(Value::as_str) == Some(this.model.read(cx).workspace_id())
        };
        match event {
            ModelEvent::FlowProgress(payload) if about_open_flow(payload, self, cx) => {
                let run_id = payload.get("runId").and_then(Value::as_str).unwrap_or_default();
                if self.run.as_ref().is_none_or(|run| run.run_id != run_id) {
                    self.run = Some(RunView::started(run_id.to_owned()));
                }
                let taken = self.run.as_mut().map(|run| run.apply_progress(payload)).unwrap_or_default();
                if !taken.is_empty() {
                    self.canvas.update(cx, |canvas, cx| canvas.pulse(&taken, cx));
                }
                self.sync_views(cx);
            }
            ModelEvent::FlowFinished(payload) if about_open_flow(payload, self, cx) => {
                self.run = RunView::from_report(payload.get("report").unwrap_or(&Value::Null));
                self.sync_views(cx);
            }
            ModelEvent::FlowLoadProgress(payload) if about_open_flow(payload, self, cx) => self.take_load(payload, cx),
            ModelEvent::FlowLoadFinished(payload) if about_open_flow(payload, self, cx) => {
                self.take_load(payload.get("report").unwrap_or(&Value::Null), cx);
            }
            ModelEvent::EntityReverted(id) if self.selected.as_deref() == Some(id.as_str()) => {
                // An edit waiting to be written belongs to the version that was just replaced.
                self.autosave.cancel();
                self.reload_due = true;
            }
            ModelEvent::ConfigChanged if self.reload_due => {
                self.reload_due = false;
                if let Some(id) = self.selected.clone() {
                    self.open(id, window, cx);
                }
            }
            // A request a node runs may have been renamed, changed or deleted, or the environment changed.
            ModelEvent::ConfigChanged => self.validate(cx),
            _ => {}
        }
    }

    fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(RunView::is_running)
    }

    /// A flow that calls an MCP server runs one instance at a time: the engine refuses it a load test.
    fn calls_mcp(&self) -> bool {
        self.doc.as_ref().is_some_and(|doc| doc.nodes.iter().any(|node| node.kind == NodeKind::Mcp))
    }

    fn is_load_running(&self) -> bool {
        self.load.as_ref().is_some_and(LoadView::is_running)
    }

    /// Take in what the engine says of the flow's load test: a progress report, or a whole one.
    fn take_load(&mut self, told: &Value, cx: &mut Context<Self>) {
        let load_id = told.get("loadId").and_then(Value::as_str).unwrap_or_default();
        if self.load.as_ref().is_none_or(|load| load.load_id() != load_id) {
            self.load = Some(LoadView::default());
        }
        if let Some(load) = self.load.as_mut() {
            load.apply(told);
        }
        cx.notify();
    }

    /// Start as many runs of the flow at once as the bar asks for, counted by the engine and shown as a list.
    fn start_load(&mut self, flow_id: String, instances: u64, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "id": flow_id, "instances": instances });
        call(&client, cx, "flow.loadtest.start", payload, |this, outcome, cx| match failure(&outcome) {
            Some(message) => this.report(t("flows.error.run"), message, cx),
            None => {
                this.show_load = true;
                cx.notify();
            }
        });
    }

    /// Run the flow as it is saved. An edit that is not written yet is written first.
    fn start_run(&mut self, cx: &mut Context<Self>) {
        let Some(flow_id) = self.selected.clone() else { return };
        if self.is_dirty() || self.saving {
            self.run_when_saved = true;
            self.autosave.cancel();
            return self.save(cx);
        }
        // The engine says what is too many; what is not a number is one run.
        let instances = self.instances.read(cx).value().trim().parse::<u64>().unwrap_or(1);
        if instances > 1 && !self.calls_mcp() {
            return self.start_load(flow_id, instances, cx);
        }
        self.show_load = false;
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "id": flow_id });
        call(&client, cx, "flow.run", payload, |this, outcome, cx| match failure(&outcome) {
            Some(message) => this.report(t("flows.error.run"), message, cx),
            None => {
                let run_id =
                    outcome.ok().and_then(|answer| answer.get("runId").and_then(Value::as_str).map(str::to_owned));
                // The first progress report may already have started the run's view.
                if let Some(run_id) = run_id.filter(|id| this.run.as_ref().is_none_or(|run| run.run_id != *id)) {
                    this.run = Some(RunView::started(run_id));
                    this.sync_views(cx);
                }
            }
        });
    }

    fn stop_run(&mut self, cx: &mut Context<Self>) {
        let Some(flow_id) = self.selected.clone() else { return };
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "id": flow_id });
        let command = if self.is_load_running() { "flow.loadtest.cancel" } else { "flow.cancel" };
        call(&client, cx, command, payload, |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(t("flows.error.stop"), message, cx);
            }
        });
    }

    pub fn prompt_new(&mut self, folder: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("flows.prompt.new_flow"),
            t("flows.common.create"),
            vec![PromptField::new(t("flows.common.name"), t("flows.field.name_example"))],
            move |this, values, window, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                let mut entity = FlowDoc::blank(&values[0]).to_value();
                entity["folderId"] = json!(folder);
                let payload = json!({ "kind": "flows", "workspaceId": model.workspace_id(), "entity": entity });
                call_in(&client, window, cx, "entity.create", payload, |this, outcome, window, cx| {
                    match failure(&outcome) {
                        Some(message) => this.report(t("flows.error.create"), message, cx),
                        None => {
                            let id = outcome
                                .ok()
                                .and_then(|answer| answer.get("id").and_then(Value::as_str).map(str::to_owned));
                            if let Some(id) = id {
                                this.autosave.cancel();
                                this.save(cx);
                                this.open(id, window, cx);
                            }
                        }
                    }
                });
            },
        );
    }

    fn prompt_new_folder(&mut self, parent: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("flows.prompt.new_folder"),
            t("flows.common.create"),
            vec![PromptField::new(t("flows.common.name"), t("flows.field.folder_name"))],
            move |this, values, _, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                let payload = json!({
                    "kind": "flow", "name": values[0], "parentId": parent, "workspaceId": model.workspace_id(),
                });
                call(&client, cx, "folder.add", payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report(t("flows.error.create_folder"), message, cx);
                    }
                });
            },
        );
    }

    fn name_of(&self, key: &TreeKey, cx: &gpui_kit::App) -> String {
        let config = self.model.read(cx).config();
        match key {
            TreeKey::Folder(id) => config.flow_folders.iter().find(|f| f.id == *id).map(|f| f.name.clone()),
            TreeKey::Item(id) => config.flows.iter().find(|f| f.id == *id).map(|f| f.name.clone()),
        }
        .unwrap_or_default()
    }

    /// Change one field of a saved flow that is not the one being edited here.
    fn patch_flow(&mut self, id: &str, field: &'static str, value: Value, what: &'static str, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let (client, workspace_id) = (model.client().clone(), model.workspace_id().to_owned());
        let mut patch = Map::new();
        patch.insert(field.into(), value);
        patch_entity(&client, cx, &workspace_id, "flows", id, patch, move |this, outcome, cx| {
            if let Err(message) = outcome {
                this.report(what, message, cx);
            }
        });
    }

    fn prompt_rename(&mut self, key: TreeKey, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.name_of(&key, cx);
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t_with("flows.prompt.rename_title", &[("name", &current)]),
            t("flows.common.rename"),
            vec![PromptField::new(t("flows.common.name"), t("flows.common.name")).initial(current.clone())],
            move |this, values, window, cx| match &key {
                TreeKey::Folder(id) => {
                    let client = this.model.read(cx).client().clone();
                    let payload = json!({ "kind": "flow", "id": id, "name": values[0] });
                    call(&client, cx, "folder.rename", payload, |this, outcome, cx| {
                        if let Some(message) = failure(&outcome) {
                            this.report(t("flows.error.rename_folder"), message, cx);
                        }
                    });
                }
                // The open flow is renamed where it is edited, so the edit and the file cannot disagree.
                TreeKey::Item(id) if this.selected.as_deref() == Some(id.as_str()) => {
                    let name = values[0].trim().to_owned();
                    this.name.update(cx, |state, cx| state.set_value(name.clone(), window, cx));
                    this.edit(cx, |doc| doc.name = name);
                }
                TreeKey::Item(id) => this.patch_flow(id, "name", json!(values[0]), t("flows.error.rename"), cx),
            },
        );
    }

    fn duplicate(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "kind": "flows", "id": id });
        call_in(&client, window, cx, "entity.duplicate", payload, |this, outcome, window, cx| {
            if let Some(message) = failure(&outcome) {
                return this.report(t("flows.error.duplicate"), message, cx);
            }
            let copy = outcome.ok().and_then(|answer| answer.pointer("/entity/id").and_then(Value::as_str).map(str::to_owned));
            if let Some(copy) = copy {
                this.autosave.cancel();
                this.save(cx);
                this.open(copy, window, cx);
            }
        });
    }

    fn move_node(&mut self, key: TreeKey, folder: Option<String>, cx: &mut Context<Self>) {
        match key {
            TreeKey::Folder(id) => {
                let model = self.model.read(cx);
                let client = model.client().clone();
                let payload = json!({ "kind": "flow", "id": id, "parentId": folder, "workspaceId": model.workspace_id() });
                call(&client, cx, "folder.move", payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report(t("flows.error.move_folder"), message, cx);
                    }
                });
            }
            TreeKey::Item(id) => self.patch_flow(&id, "folderId", json!(folder), t("flows.error.move"), cx),
        }
    }

    fn confirm_delete(&mut self, key: TreeKey, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_of(&key, cx);
        let (title, consequence) = match &key {
            TreeKey::Folder(_) => (t_with("flows.confirm.delete_folder_title", &[("name", &name)]), t(FOLDER_DELETE_CONSEQUENCE)),
            TreeKey::Item(id) => (
                t_with("flows.confirm.delete_flow_title", &[("name", &name)]),
                delete_consequence(self.model.read(cx).git().entity_state(id)),
            ),
        };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            title,
            consequence,
            t("flows.common.delete"),
            move |this, _, cx| {
                let client = this.model.read(cx).client().clone();
                let (action, payload) = match &key {
                    TreeKey::Folder(id) => ("folder.delete", json!({ "kind": "flow", "id": id })),
                    TreeKey::Item(id) => ("entity.delete", json!({ "kind": "flows", "id": id })),
                };
                // Nothing of the open flow is worth writing once it is gone, alone or with its folder.
                this.autosave.cancel();
                if matches!(&key, TreeKey::Item(id) if this.selected.as_deref() == Some(id.as_str())) {
                    this.doc = None;
                }
                call(&client, cx, action, payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report(t("flows.error.delete"), message, cx);
                    }
                });
            },
        );
    }

    fn on_tree_event(&mut self, _: &Entity<FlowTree>, event: &FlowTreeEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.clone() {
            FlowTreeEvent::Open(id) => self.select(id, window, cx),
            FlowTreeEvent::NewFlow { folder } => self.prompt_new(folder, window, cx),
            FlowTreeEvent::NewFolder { parent } => self.prompt_new_folder(parent, window, cx),
            FlowTreeEvent::Rename(key) => self.prompt_rename(key, window, cx),
            FlowTreeEvent::Duplicate(id) => self.duplicate(id, window, cx),
            FlowTreeEvent::Delete(key) => self.confirm_delete(key, window, cx),
            FlowTreeEvent::Move { key, folder } => self.move_node(key, folder, cx),
        }
    }

    /// What the footer under the canvas asks for. A flow has no draft, so Save has nothing to do.
    pub fn entity_action(&mut self, action: EntityAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else { return };
        match action {
            EntityAction::Save | EntityAction::Close | EntityAction::Move(_) => {}
            EntityAction::Sync => self.model.update(cx, |model, cx| model.sync_entity(&id, cx)),
            EntityAction::Revert => {
                let name = self.name.read(cx).value();
                confirm_danger(
                    cx.weak_entity(),
                    window,
                    cx,
                    t_with("flows.revert.title", &[("name", &name)]),
                    t("flows.revert.consequence"),
                    t("flows.common.revert"),
                    move |this, _, cx| this.model.update(cx, |model, cx| model.revert_entity(&id, cx)),
                );
            }
            EntityAction::Delete => {
                let key = TreeKey::Item(id);
                if !is_deleted(self.model.read(cx), &key) {
                    self.confirm_delete(key, window, cx);
                }
            }
        }
    }

    /// The selected flow, unless it is deleted or saved by a newer Bifurc.
    fn active_flow(&self, cx: &Context<Self>) -> Option<TreeKey> {
        let key = TreeKey::Item(self.selected.clone()?);
        let model = self.model.read(cx);
        (!is_deleted(model, &key) && locked_to(model, &key).is_none()).then_some(key)
    }

    /// Copies the selected flow, as the tree's menu does.
    pub fn duplicate_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(TreeKey::Item(id)) = self.active_flow(cx) {
            self.duplicate(id, window, cx);
        }
    }

    /// Asks for a new name for the selected flow, as the tree's menu does.
    pub fn rename_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.active_flow(cx) {
            self.prompt_rename(key, window, cx);
        }
    }

    /// What the last run came to, in a line.
    fn run_summary(&self) -> Option<(String, bool)> {
        let run = self.run.as_ref()?;
        let took = run
            .duration_ms
            .map(|ms| t_with("flows.run.took", &[("seconds", &format!("{:.1}", ms as f64 / 1000.0))]))
            .unwrap_or_default();
        let tests = match run.total_tests {
            0 => String::new(),
            total => t_with("flows.run.tests_passed", &[("passed", &(total - run.failed_tests)), ("total", &total)]),
        };
        Some(match run.status.as_str() {
            "running" => (t("flows.run.running").to_owned(), false),
            "succeeded" => (t_with("flows.run.succeeded", &[("took", &took), ("tests", &tests)]), run.failed_tests > 0),
            "cancelled" => (t_with("flows.run.stopped", &[("took", &took)]), false),
            _ => (
                t_with(
                    "flows.run.failed",
                    &[("took", &took), ("error", &run.error.as_deref().unwrap_or(t("flows.run.see_nodes")))],
                ),
                true,
            ),
        })
    }

    /// The bar over the canvas: the flow's name, how it is saved, its last run, and Run or Stop.
    fn render_bar(&self, doc: &FlowDoc, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let running = self.is_running() || self.is_load_running();
        let errors: Vec<String> =
            self.described.problems.iter().filter(|problem| problem.is_error).map(|problem| {
                let node = problem.node_id.as_deref().and_then(|id| doc.node(id)).map(FlowNode::title);
                match node {
                    Some(node) => format!("{node}: {}", problem.message),
                    None => problem.message.clone(),
                }
            }).collect();
        let first_problem = self.described.problems.iter().find(|problem| problem.is_error).and_then(|p| p.node_id.clone());
        let run = if running {
            Button::new("flow-stop")
                .small()
                .danger()
                .icon(IconName::Pause)
                .label(t("flows.common.stop"))
                .on_click(cx.listener(|this, _, _, cx| this.stop_run(cx)))
        } else {
            Button::new("flow-run")
                .small()
                .primary()
                .icon(IconName::Play)
                .label(t("flows.common.run"))
                .disabled(!self.described.ok || self.run_when_saved)
                .on_click(cx.listener(|this, _, _, cx| this.start_run(cx)))
        };
        let saved = match (self.saving, self.is_dirty()) {
            (true, _) => t("flows.save.saving"),
            (false, true) => t("flows.save.unsaved"),
            (false, false) => t("flows.save.saved"),
        };
        h_flex()
            .flex_none()
            .gap_3()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(theme.border)
            .child(div().flex_none().text_xs().text_color(theme.muted_foreground).child(saved))
            .child(div().flex_1())
            .when_some(self.run_summary().filter(|_| !self.show_load), |this, (text, bad)| {
                let color = if bad { theme.danger } else { theme.muted_foreground };
                this.child(div().min_w_0().text_sm().text_color(color).overflow_hidden().text_ellipsis().child(text))
            })
            .when(!errors.is_empty() && !running, |this| {
                let count = errors.len();
                let label = if count == 1 {
                    t_with("flows.problems.count_one", &[("count", &count)])
                } else {
                    t_with("flows.problems.count_other", &[("count", &count)])
                };
                this.child(
                    Button::new("flow-problems")
                        .small()
                        .ghost()
                        .icon(IconName::TriangleAlert)
                        .label(label)
                        .tooltip(errors.join("\n"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if first_problem.is_some() {
                                this.select_node(first_problem.clone(), window, cx);
                            }
                        })),
                )
            })
            .when(self.load.is_some(), |this| {
                let label = if self.show_load { t("flows.load.canvas") } else { t("flows.load.show") };
                this.child(Button::new("flow-load-view").small().ghost().label(label).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.show_load = !this.show_load;
                        cx.notify();
                    },
                )))
            })
            .child({
                // A flow that cannot have instances still shows the field, and says why it is off.
                let refused = self.calls_mcp();
                h_flex()
                    .id("flow-instances")
                    .flex_none()
                    .gap_3()
                    .child(div().text_xs().text_color(theme.muted_foreground).child(t("flows.load.instances")))
                    .child(div().w_20().child(Input::new(&self.instances).small().disabled(running || refused)))
                    .when(refused, |this| {
                        this.tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(t("flows.load.instances_mcp")).build(window, cx)
                        })
                    })
            })
            .child(run)
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(doc) = self.doc.as_ref() else {
            return match self.selected {
                Some(_) => page::loading(t("flows.loading.opening")),
                None => page::empty(t("flows.empty.title"), t("flows.empty.description")),
            };
        };
        let bar = EntityBar {
            saving: self.saving,
            unsaved: self.is_dirty(),
            problem: None,
            closable: false,
            deletable: true,
            ..EntityBar::of(self.model.read(cx), Some(doc.id.as_str()))
        };
        v_flex()
            .size_full()
            .child(self.render_bar(doc, cx))
            .child(match self.load.clone().filter(|_| self.show_load) {
                Some(shown) => div().flex_1().min_h_0().child(load::render(&shown, cx)),
                None => div().flex_1().min_h_0().child(self.canvas.clone()),
            })
            .child(entity_bar(bar, cx, |this, action, window, cx| this.entity_action(action, window, cx)))
            .into_any_element()
    }
}

impl Render for FlowsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        h_flex()
            .size_full()
            .items_stretch()
            .bg(theme.background)
            .child(div().w_64().flex_none().h_full().border_r_1().border_color(theme.border).child(self.tree.clone()))
            .child(div().flex_1().min_w_0().child(self.render_detail(cx)))
    }
}
