//! The Flows screen: saved requests chained on a canvas, and the runs of those chains.
//!
//! The list is the model's snapshot. The canvas and the panel beside it hold an *edit* of the selected flow,
//! written over the engine's copy a moment after each change. The engine says what is wrong with the flow
//! and what each node offers (`flow.validate`), runs it (`flow.run`), and reports the run as it goes; this
//! screen draws what it is told.

mod canvas;
mod inspector;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use serde_json::{Value, json};

use self::{
    canvas::{CanvasEdge, CanvasEvent, CanvasNode, FlowCanvas},
    inspector::{Inspector, InspectorEvent},
};
use crate::{
    app_model::{AppModel, Deleted, ModelEvent},
    components::{
        Autosave, EntityAction, EntityBar,
        dialogs::{PromptField, confirm_danger, delete_consequence, prompt, resolve_deleted},
        entity_bar, page,
    },
    engine::{call, call_in, failure},
    model::flow::{Described, FlowDoc, FlowNode, NodeKind, RunView},
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
    timeout: Entity<InputState>,
    described: Described,
    validating: bool,
    validate_again: bool,
    run: Option<RunView>,
    selected_node: Option<String>,
    canvas: Entity<FlowCanvas>,
    inspector: Entity<Inspector>,
    _subscriptions: Vec<Subscription>,
}

impl FlowsScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Flow name"));
        let timeout = cx.new(|cx| InputState::new(window, cx).placeholder("300"));
        let canvas = cx.new(FlowCanvas::new);
        let inspector = cx.new(|cx| Inspector::new(model.clone(), window, cx));
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| this.follow_model(window, cx)),
            cx.subscribe_in(&model, window, Self::on_model_event),
            cx.subscribe(&name, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let name = input.read(cx).value().trim().to_owned();
                    this.edit(cx, |doc| doc.name = name);
                }
            }),
            cx.subscribe(&timeout, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let seconds: Option<u64> = input.read(cx).value().trim().parse().ok();
                    this.edit(cx, |doc| doc.settings.timeout_ms = seconds.map(|seconds| seconds * 1000));
                }
            }),
            cx.subscribe_in(&canvas, window, Self::on_canvas_event),
            cx.subscribe(&inspector, Self::on_inspector_event),
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
            timeout,
            described: Described::default(),
            validating: false,
            validate_again: false,
            run: None,
            selected_node: None,
            canvas,
            inspector,
            _subscriptions: subscriptions,
        };
        this.follow_model(window, cx);
        this
    }

    fn is_dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|doc| doc.to_value() != self.baseline)
    }

    /// The open flow, for the window's breadcrumb. The shell shows this screen's name field there.
    pub fn active_target(&self, cx: &gpui_kit::App) -> Option<ActiveTarget> {
        let id = self.doc.as_ref().map(|doc| doc.id.clone())?;
        let name = self.name.read(cx).value();
        let title = if name.is_empty() { SharedString::from("Flow") } else { name };
        Some(ActiveTarget::named(title, Some(id), self.name.clone()))
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
    }

    /// Keeps a flow open that exists: the one that is, or else the first there is.
    fn follow_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let flows = &self.model.read(cx).config().flows;
        let still_there =
            self.selected.as_deref().is_some_and(|id| flows.iter().any(|flow| flow.id == id && !flow.deleted));
        if !still_there {
            match flows.iter().find(|flow| !flow.deleted).map(|flow| flow.id.clone()) {
                Some(id) => self.open(id, window, cx),
                None => self.close(cx),
            }
        }
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.autosave.cancel();
        (self.selected, self.doc, self.run, self.selected_node) = (None, None, None, None);
        self.baseline = Value::Null;
        self.described = Described::default();
        self.sync_views(cx);
    }

    /// Read a flow from the engine and show it.
    fn open(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.close(cx);
        self.selected = Some(id.clone());
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
                Some(Err(error)) => this.report("Couldn't read the flow.", error, cx),
                None => this.report(
                    "Couldn't open the flow.",
                    failure(&outcome).unwrap_or_else(|| "It is not there.".to_owned()),
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
    }

    fn show(&mut self, doc: FlowDoc, window: &mut Window, cx: &mut Context<Self>) {
        self.baseline = doc.to_value();
        self.name.update(cx, |state, cx| state.set_value(doc.name.clone(), window, cx));
        let seconds = doc.settings.timeout_ms.map(|ms| (ms / 1000).to_string()).unwrap_or_default();
        self.timeout.update(cx, |state, cx| state.set_value(seconds, window, cx));
        self.doc = Some(doc);
        self.selected_node = None;
        self.inspector.update(cx, |inspector, cx| inspector.show(None, &[], window, cx));
        self.canvas.update(cx, |canvas, cx| {
            canvas.reset_view(cx);
            canvas.set_selected(None, cx);
        });
        self.validate(cx);
        self.sync_views(cx);
    }

    pub fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(flow) = self.model.read(cx).config().flows.iter().find(|flow| flow.id == id).cloned() else { return };
        // A deleted flow is brought back or deleted for good, not edited.
        if flow.deleted {
            let deleted = Deleted::Entity { kind: "flows", id: flow.id.clone() };
            resolve_deleted(self.model.clone(), deleted, flow.name.clone(), window, cx);
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
        self.saving = true;
        cx.notify();
        let client = self.model.read(cx).client().clone();
        let update = json!({ "kind": "flows", "entity": saved });
        call(&client, cx, "entity.update", update, move |this, outcome, cx| {
            this.saving = false;
            cx.notify();
            if let Some(message) = failure(&outcome) {
                this.run_when_saved = false;
                return this.report("Couldn't save the flow.", message, cx);
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

    fn node_title(node: &FlowNode) -> SharedString {
        if node.name.is_empty() { node.kind.label().into() } else { node.name.clone().into() }
    }

    /// Hand the canvas and the panel what they draw: the flow, what the engine says of it, and the run.
    fn sync_views(&mut self, cx: &mut Context<Self>) {
        let (mut nodes, mut edges) = (Vec::new(), Vec::new());
        if let Some(doc) = &self.doc {
            for node in &doc.nodes {
                let info = self.described.nodes.get(&node.id);
                let run = self.run.as_ref().and_then(|run| run.nodes.get(&node.id));
                let request = info.and_then(|info| info.request_name.clone());
                let detail = match node.kind {
                    NodeKind::Start => match doc.inputs.len() {
                        0 => "Where a run begins".to_owned(),
                        1 => "1 input".to_owned(),
                        count => format!("{count} inputs"),
                    },
                    NodeKind::Request | NodeKind::Stream => request.unwrap_or_else(|| "No request chosen".to_owned()),
                    NodeKind::Mcp => match (request, node.target.is_empty()) {
                        (Some(request), false) => format!("{} · {}", request, node.target),
                        (Some(request), true) => request,
                        (None, _) => "No request chosen".to_owned(),
                    },
                    NodeKind::Condition => format!("{} branch(es), then Else", node.branches.len()),
                    NodeKind::Delay => format!("Waits {} ms", node.delay_ms),
                };
                nodes.push(CanvasNode {
                    id: node.id.clone(),
                    kind: node.kind,
                    title: Self::node_title(node),
                    detail: detail.into(),
                    x: node.x,
                    y: node.y,
                    ports: info
                        .map(|info| info.ports.iter().map(|(id, name)| (id.clone(), name.clone().into())).collect())
                        .unwrap_or_default(),
                    problem: self.described.has_error(&node.id),
                    status: run.map_or("idle", |run| run.status.as_str()).to_owned().into(),
                    runs: run.map_or(0, |run| run.runs),
                });
            }
            let taken = |id: &str| self.run.as_ref().and_then(|run| run.edges.get(id)).copied().unwrap_or(0);
            edges = doc
                .edges
                .iter()
                .map(|edge| CanvasEdge {
                    id: edge.id.clone(),
                    from: edge.from.clone(),
                    port: edge.port.clone(),
                    to: edge.to.clone(),
                    count: taken(&edge.id),
                })
                .collect();
        }
        self.canvas.update(cx, |canvas, cx| canvas.set_graph(nodes, edges, cx));

        let context = self.selected_node.as_deref().zip(self.doc.as_ref()).map(|(id, doc)| {
            let info = self.described.nodes.get(id).cloned().unwrap_or_default();
            let connections: Vec<(String, SharedString)> = doc
                .edges
                .iter()
                .filter(|edge| edge.from == id)
                .map(|edge| {
                    let port = info
                        .ports
                        .iter()
                        .find(|(port, _)| *port == edge.port)
                        .map_or(edge.port.as_str(), |(_, name)| name);
                    let to = doc.node(&edge.to).map(Self::node_title).unwrap_or_default();
                    (edge.id.clone(), format!("{port} → {to}").into())
                })
                .collect();
            let run = self.run.as_ref().and_then(|run| run.nodes.get(id)).cloned();
            (info, connections, run)
        });
        if let Some((info, connections, run)) = context {
            self.inspector.update(cx, |inspector, cx| inspector.set_context(info, connections, run, cx));
        }
        cx.notify();
    }

    fn select_node(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_node = id;
        let node = self.selected_node.as_deref().zip(self.doc.as_ref()).and_then(|(id, doc)| doc.node(id).cloned());
        let inputs = self.doc.as_ref().map(|doc| doc.inputs.clone()).unwrap_or_default();
        self.inspector.update(cx, |inspector, cx| inspector.show(node, &inputs, window, cx));
        self.canvas.update(cx, |canvas, cx| canvas.set_selected(self.selected_node.clone(), cx));
        self.sync_views(cx);
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
            CanvasEvent::Connected { from, port, to } => self.edit(cx, |doc| {
                doc.connect(&from, &port, &to);
            }),
            CanvasEvent::Delete(id) => self.delete_node(&id, cx),
        }
    }

    /// Remove a node and its connections. The start node stays.
    fn delete_node(&mut self, id: &str, cx: &mut Context<Self>) {
        let removable = |doc: &FlowDoc| doc.node(id).is_some_and(|node| node.kind != NodeKind::Start);
        if !self.doc.as_ref().is_some_and(removable) {
            return;
        }
        self.selected_node = None;
        self.canvas.update(cx, |canvas, cx| canvas.set_selected(None, cx));
        self.edit(cx, |doc| doc.remove_node(id));
    }

    fn on_inspector_event(&mut self, _: Entity<Inspector>, event: &InspectorEvent, cx: &mut Context<Self>) {
        match event.clone() {
            InspectorEvent::Changed(node) => self.edit(cx, |doc| {
                let mut node = *node;
                // A branch that is gone takes the connections that left by it.
                if node.kind == NodeKind::Condition {
                    let kept = |port: &str| port == "else" || node.branches.iter().any(|branch| branch.id == port);
                    doc.edges.retain(|edge| edge.from != node.id || kept(&edge.port));
                }
                if let Some(slot) = doc.node_mut(&node.id) {
                    // The panel does not know where the node was dragged to since it was picked.
                    (node.x, node.y) = (slot.x, slot.y);
                    *slot = node;
                }
            }),
            InspectorEvent::Inputs(inputs) => self.edit(cx, |doc| doc.inputs = inputs),
            InspectorEvent::Disconnect(edge_id) => self.edit(cx, |doc| doc.disconnect(&edge_id)),
            InspectorEvent::Delete(id) => self.delete_node(&id, cx),
        }
    }

    fn add_node(&mut self, kind: NodeKind, window: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = self.canvas.read(cx).place_for_new();
        let mut added = None;
        self.edit(cx, |doc| added = Some(doc.add_node(kind, x, y)));
        self.select_node(added, window, cx);
        self.canvas.update(cx, |canvas, cx| canvas.focus(window, cx));
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
            // A request a node runs may have been renamed, changed or deleted.
            ModelEvent::ConfigChanged => self.validate(cx),
            _ => {}
        }
    }

    fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(RunView::is_running)
    }

    /// Run the flow as it is saved. An edit that is not written yet is written first.
    fn start_run(&mut self, cx: &mut Context<Self>) {
        let Some(flow_id) = self.selected.clone() else { return };
        if self.is_dirty() || self.saving {
            self.run_when_saved = true;
            self.autosave.cancel();
            return self.save(cx);
        }
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "id": flow_id });
        call(&client, cx, "flow.run", payload, |this, outcome, cx| match failure(&outcome) {
            Some(message) => this.report("The flow did not run.", message, cx),
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
        call(&client, cx, "flow.cancel", payload, |this, outcome, cx| {
            if let Some(message) = failure(&outcome) {
                this.report("Couldn't stop the flow.", message, cx);
            }
        });
    }

    pub fn prompt_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            "New flow",
            "Create",
            vec![PromptField::new("Name", "Sign in and place an order")],
            |this, values, window, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                let payload = json!({
                    "kind": "flows",
                    "workspaceId": model.workspace_id(),
                    "entity": FlowDoc::blank(&values[0]).to_value(),
                });
                call_in(&client, window, cx, "entity.create", payload, |this, outcome, window, cx| {
                    match failure(&outcome) {
                        Some(message) => this.report("Couldn't create the flow.", message, cx),
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
                    format!("Revert “{name}” to the last synced version?"),
                    "The changes saved since then are discarded.",
                    "Revert",
                    move |this, _, cx| this.model.update(cx, |model, cx| model.revert_entity(&id, cx)),
                );
            }
            EntityAction::Delete => {
                let name = self.name.read(cx).value();
                let consequence = delete_consequence(self.model.read(cx).git().entity_state(&id));
                confirm_danger(
                    cx.weak_entity(),
                    window,
                    cx,
                    format!("Delete flow “{name}”?"),
                    consequence,
                    "Delete",
                    move |this, _, cx| {
                        let client = this.model.read(cx).client().clone();
                        // Nothing here is worth writing once the flow is gone.
                        this.autosave.cancel();
                        this.doc = None;
                        call(
                            &client,
                            cx,
                            "entity.delete",
                            json!({ "kind": "flows", "id": id }),
                            |this, outcome, cx| {
                                if let Some(message) = failure(&outcome) {
                                    this.report("Couldn't delete the flow.", message, cx);
                                }
                            },
                        );
                    },
                );
            }
        }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut flows: Vec<_> = self.model.read(cx).config().flows.clone();
        flows.sort_by_key(|flow| flow.name.to_lowercase());
        v_flex()
            .id("flow-list")
            .w_56()
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .p_2()
            .gap_1()
            .children(flows.into_iter().map(|flow| {
                let id = flow.id.clone();
                ListItem::new(gpui_kit::ElementId::Name(format!("flow-{}", flow.id).into()))
                    .selected(self.selected.as_deref() == Some(flow.id.as_str()))
                    .px_2()
                    .py_1()
                    .text_sm()
                    .child(
                        div()
                            .overflow_hidden()
                            .text_ellipsis()
                            .when(flow.deleted, |this| this.line_through().text_color(theme.muted_foreground))
                            .child(flow.name.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.select(id.clone(), window, cx)))
            }))
    }

    /// What the last run came to, in a line.
    fn run_summary(&self) -> Option<(String, bool)> {
        let run = self.run.as_ref()?;
        let took = run.duration_ms.map(|ms| format!(" in {:.1} s", ms as f64 / 1000.0)).unwrap_or_default();
        let tests = match run.total_tests {
            0 => String::new(),
            total => format!(", {} of {total} tests passed", total - run.failed_tests),
        };
        Some(match run.status.as_str() {
            "running" => ("Running…".to_owned(), false),
            "succeeded" => (format!("Succeeded{took}{tests}"), run.failed_tests > 0),
            "cancelled" => (format!("Stopped{took}"), false),
            _ => (format!("Failed{took}: {}", run.error.as_deref().unwrap_or("see the nodes")), true),
        })
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let running = self.is_running();
        let add = |kind: NodeKind, icon: IconName, cx: &mut Context<Self>| {
            Button::new(kind.label())
                .small()
                .ghost()
                .icon(icon)
                .label(kind.label())
                .disabled(running)
                .on_click(cx.listener(move |this, _, window, cx| this.add_node(kind, window, cx)))
        };
        let run = if running {
            Button::new("flow-stop")
                .small()
                .danger()
                .icon(IconName::Pause)
                .label("Stop")
                .on_click(cx.listener(|this, _, _, cx| this.stop_run(cx)))
        } else {
            Button::new("flow-run")
                .small()
                .primary()
                .icon(IconName::Play)
                .label("Run")
                .disabled(!self.described.ok || self.run_when_saved)
                .tooltip(if self.described.ok { "Run this flow" } else { "Fix the flow's problems to run it" })
                .on_click(cx.listener(|this, _, _, cx| this.start_run(cx)))
        };
        h_flex()
            .flex_none()
            .gap_1()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(theme.border)
            .child(div().text_xs().text_color(theme.muted_foreground).child("Add"))
            .child(add(NodeKind::Request, IconName::Globe, cx))
            .child(add(NodeKind::Stream, IconName::Inbox, cx))
            .child(add(NodeKind::Mcp, IconName::Bot, cx))
            .child(add(NodeKind::Condition, IconName::Replace, cx))
            .child(add(NodeKind::Delay, IconName::Pause, cx))
            .child(div().flex_1())
            .when_some(self.run_summary(), |this, (text, bad)| {
                let color = if bad { theme.danger } else { theme.muted_foreground };
                this.child(div().text_sm().text_color(color).overflow_hidden().text_ellipsis().child(text))
            })
            .child(run)
    }

    /// The panel when no node is picked: what keeps the flow from running, and how it runs.
    fn render_flow_panel(&self, doc: &FlowDoc, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let problems: Vec<AnyElement> = self
            .described
            .problems
            .iter()
            .enumerate()
            .map(|(index, problem)| {
                let node_id = problem.node_id.clone();
                let node = node_id.as_deref().and_then(|id| doc.node(id)).map(Self::node_title);
                let (icon, color) = match problem.is_error {
                    true => (IconName::CircleX, theme.danger),
                    false => (IconName::TriangleAlert, theme.warning),
                };
                h_flex()
                    .id(("flow-problem", index))
                    .items_start()
                    .gap_2()
                    .text_sm()
                    .child(Icon::new(icon).small().text_color(color).flex_none())
                    .child(
                        v_flex()
                            .min_w_0()
                            .when_some(node, |this, node| this.child(div().font_medium().child(node)))
                            .child(problem.message.clone()),
                    )
                    .when(node_id.is_some(), |this| {
                        this.cursor_pointer().on_click(
                            cx.listener(move |this, _, window, cx| this.select_node(node_id.clone(), window, cx)),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        v_flex()
            .id("flow-panel")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_4()
            .child(
                v_flex()
                    .gap_2()
                    .child(page::section_label("Before it can run", cx))
                    .when(problems.is_empty(), |this| {
                        this.child(div().text_sm().text_color(theme.muted_foreground).child("Nothing is in the way."))
                    })
                    .children(problems),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(page::section_label("Stop a run after (seconds)", cx))
                    .child(Input::new(&self.timeout).small())
                    .child(div().text_xs().text_color(theme.muted_foreground).child("Empty is 300. 0 is no limit.")),
            )
            .child(
                Switch::new("flow-persist")
                    .small()
                    .checked(doc.settings.persist_variables)
                    .label("Keep the variables scripts set in the environment after a run")
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        let checked = *checked;
                        this.edit(cx, |doc| doc.settings.persist_variables = checked);
                    })),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(
                "Pick a node to set it up. Drag a node by its card, the canvas by its background, and a dot on a node's right edge onto the node that runs next.",
            ))
            .into_any_element()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(doc) = self.doc.as_ref() else {
            return match self.selected {
                Some(_) => page::loading("Opening the flow…"),
                None => page::empty(
                    "No flow",
                    "Create a flow to chain saved requests: what one answers is what the next one is given.",
                ),
            };
        };
        let theme = cx.theme().clone();
        let bar = EntityBar {
            saving: self.saving,
            unsaved: self.is_dirty(),
            problem: None,
            closable: false,
            deletable: true,
            ..EntityBar::of(self.model.read(cx), Some(doc.id.as_str()))
        };
        let panel = match self.selected_node.is_some() && self.inspector.read(cx).node_id().is_some() {
            true => self.inspector.clone().into_any_element(),
            false => self.render_flow_panel(doc, cx),
        };
        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(div().flex_1().min_w_0().child(self.canvas.clone()))
                    .child(div().w_80().flex_none().h_full().border_l_1().border_color(theme.border).child(panel)),
            )
            .child(entity_bar(bar, cx, |this, action, window, cx| this.entity_action(action, window, cx)))
            .into_any_element()
    }
}

impl Render for FlowsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = page::header(
            "Flows",
            "Saved requests chained together: each node is handed what the one before it answered.",
            Button::new("new-flow")
                .small()
                .icon(IconName::Plus)
                .label("New flow")
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
