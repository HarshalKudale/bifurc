//! A run of a flow: what travels from node to node, the state a client draws the run from, and the one live
//! run each flow may have.
//!
//! What a node produced goes to every node its port leads to, each of which sees it as `input` and every
//! earlier node's as `nodes.<id>`. Branches that leave one port run side by side. A run ends when nothing is
//! left to do, when it is cancelled, when a node fails with nothing connected to its error port, or when its
//! time is up; whatever it connected is then closed.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use futures::future::{BoxFuture, join_all};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    condition,
    model::{
        Edge, Flow, Node, NodeKind, PORT_DONE, PORT_EACH, PORT_ELSE, PORT_ERROR, PORT_FALSE, PORT_OUT, PORT_PASS,
        PORT_TRUE,
    },
    nodes,
    reference::{Scope, lookup, pick},
    streams, validate,
};
use crate::{
    env::{VariableChanges, VariableScope, with_variable_scope},
    kernel::{CommandContext, bus},
    protocol::{EngineError, Result},
    utils::{
        id::{generate_id, now_ms},
        json::{array_of, is_truthy, opt_str, str_of},
    },
    workspace::config::load_entity,
};

/// How long a run may take when neither the flow nor the caller says.
const DEFAULT_TIMEOUT_MS: u64 = 5 * 60 * 1000;

/// How many times nodes may run in one run: what stops a stream that fans out without end.
const MAX_NODE_RUNS: u64 = 10_000;

/// How much of a node's input and output a snapshot keeps.
const PREVIEW_LIMIT: usize = 16 * 1024;

/// How often a run reports what changed. A fast stream changes a node many times between two reports.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// What a node is handed: what the node before it produced, and what every node before that did.
#[derive(Clone)]
pub struct Token {
    pub input: Arc<Value>,
    pub nodes: Arc<HashMap<String, Arc<Value>>>,
    /// Which turn of a loop, or which message of a stream, this is part of. An OR node lets the first of
    /// each through.
    pub wave: u64,
}

/// What is wired into a node's inputs for one of its runs: each input (`var:id`) and the value it gets.
pub type Wired = Vec<(String, Value)>;

/// Something a run connected, and the command that closes it.
pub struct Live {
    pub disconnect: &'static str,
    pub id_field: &'static str,
    pub id: String,
}

#[derive(Clone)]
struct NodeState {
    status: &'static str,
    runs: u64,
    port: Option<String>,
    error: Option<String>,
    duration_ms: Option<u64>,
    input: Value,
    /// What was wired into the node's inputs, by input.
    inputs: Value,
    output: Value,
    /// What last left by each of the node's ports.
    outputs: serde_json::Map<String, Value>,
    /// How many messages a stream node handed on, or how many turns a loop took.
    handed_on: u64,
    /// The tests of a request's answer, or the checks of a validate node: `{ name, passed, error? }` each.
    tests: Value,
}

impl Default for NodeState {
    fn default() -> Self {
        Self {
            status: "idle",
            runs: 0,
            port: None,
            error: None,
            duration_ms: None,
            input: Value::Null,
            inputs: json!({}),
            output: Value::Null,
            outputs: serde_json::Map::new(),
            handed_on: 0,
            tests: json!([]),
        }
    }
}

impl NodeState {
    fn produced(&mut self, port: &str, output: Value) {
        self.outputs.insert(port.to_string(), output.clone());
        self.output = output;
    }
}

struct State {
    status: &'static str,
    error: Option<String>,
    cancel_requested: bool,
    finished_at: Option<i64>,
    /// The node that started last: where the run is, or where it ended.
    at: Option<String>,
    nodes: HashMap<String, NodeState>,
    edges: HashMap<String, u64>,
    changed_nodes: HashSet<String>,
    changed_edges: HashSet<String>,
    total_tests: u64,
    passed_tests: u64,
}

pub struct Run {
    pub ws_id: String,
    pub flow: Flow,
    pub run_id: String,
    pub ctx: CommandContext,
    /// The run's inputs by name: the flow's defaults under what the caller gave.
    pub inputs: Value,
    /// Which run of a load test this is. Such a run reports nothing itself: the load test counts it.
    pub instance: Option<usize>,
    /// How many stream messages the whole run takes before its streams are closed.
    pub max_events: Option<u64>,
    pub events: AtomicU64,
    /// The MCP connections the run has open, one per saved request.
    pub mcp: tokio::sync::Mutex<HashSet<String>>,
    /// The stream nodes that are connected.
    pub listening: Mutex<HashSet<String>>,
    started_at: i64,
    cancel: CancellationToken,
    node_runs: AtomicU64,
    waves: AtomicU64,
    /// The OR nodes that let something through, with the wave it was part of.
    merged: Mutex<HashSet<(String, u64)>>,
    /// The variables scripts set during the run, when they are not written to the environment.
    variables: Mutex<Vec<(String, String)>>,
    live: Mutex<Vec<Live>>,
    state: Mutex<State>,
}

/// A value as a snapshot keeps it: whole when small, the start of it when not.
fn preview(value: &Value) -> Value {
    let text = value.to_string();
    if text.len() <= PREVIEW_LIMIT {
        return value.clone();
    }
    let mut end = PREVIEW_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    json!({ "truncated": true, "bytes": text.len(), "preview": &text[..end] })
}

impl Run {
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// What the ids of the run's connections start with. Runs of one flow side by side each keep their own.
    pub fn live_prefix(&self) -> String {
        match self.instance {
            Some(instance) => format!("flow:{}#{instance}", self.flow.id),
            None => format!("flow:{}", self.flow.id),
        }
    }

    /// End the run as cancelled.
    pub(super) fn stop(&self) {
        self.state.lock().cancel_requested = true;
        self.cancel.cancel();
    }

    /// The run in a line, for a load test's list: how it stands, where it is or ended, and what went wrong.
    pub(super) fn brief(&self) -> Value {
        let state = self.state.lock();
        json!({
            "status": state.status, "at": state.at, "error": state.error,
            "failedTests": state.total_tests - state.passed_tests,
            "durationMs": state.finished_at.map(|end| (end - self.started_at).max(0)),
        })
    }

    /// Each node that ran, with whether it failed and how long its last run took.
    pub(super) fn timings(&self) -> Vec<(String, bool, Option<u64>)> {
        let state = self.state.lock();
        state.nodes.iter().map(|(id, node)| (id.clone(), node.status == "failed", node.duration_ms)).collect()
    }

    pub fn scope<'a>(&'a self, token: &'a Token) -> Scope<'a> {
        Scope { input: &token.input, nodes: &token.nodes, flow: &self.inputs }
    }

    /// Run a command for a node, with the variables of [`Self::scoped`].
    pub async fn invoke(&self, command: &str, payload: Value, bindings: Vec<(String, String)>) -> Result<Value> {
        self.scoped(bindings, self.ctx.registry.invoke(command, payload, self.ctx.clone())).await
    }

    /// Do a node's `work` with the run's inputs, the variables its scripts set and the node's own `bindings`
    /// laid over the environment, in that order. What a script sets meanwhile is kept for the nodes after it.
    pub async fn scoped<T>(&self, bindings: Vec<(String, String)>, work: impl Future<Output = T>) -> T {
        let collected =
            (!self.flow.settings.persist_variables).then(|| Arc::new(Mutex::new(VariableChanges::default())));
        let mut variables: Vec<(String, String)> = self
            .inputs
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_string()))
            .collect();
        variables.extend(self.variables.lock().iter().cloned());
        variables.extend(bindings);
        let scope = VariableScope { variables, changes: collected.clone() };
        let result = with_variable_scope(scope, work).await;
        if let Some(collected) = collected {
            let changes = std::mem::take(&mut *collected.lock());
            let mut kept = self.variables.lock();
            // shortcut: a variable a script removes is only forgotten by the run, the environment's own value
            // shows through again; hide it too if scripts come to rely on unsetting one.
            kept.retain(|(name, _)| !changes.removed.contains(name) && !changes.set.iter().any(|(set, _)| set == name));
            kept.extend(changes.set);
        }
        result
    }

    /// `token` as a new turn of a loop or a new message of a stream.
    pub fn turn(&self, token: &Token) -> Token {
        Token { wave: self.waves.fetch_add(1, Ordering::Relaxed) + 1, ..token.clone() }
    }

    /// What is wired into the inputs of `node`: for each wire the part it takes of what its source produced on
    /// the way here, or of what a data node holds. A wire whose source has nothing to give is an error that
    /// names it: falling back to the environment would hide the mistake.
    fn wired(&self, node: &Node, token: &Token) -> std::result::Result<Wired, String> {
        let mut values = Wired::new();
        for wire in self.flow.wires_into(&node.id) {
            let source = self.flow.node(&wire.from);
            let held = match source {
                Some(source) if matches!(source.kind, NodeKind::Data | NodeKind::Sample) => Some(&source.value),
                _ => token.nodes.get(&wire.from).map(Arc::as_ref),
            };
            let value = held.and_then(|held| pick(held, &wire.path)).ok_or_else(|| {
                let name = wire.input.split_once(':').map_or(wire.input.as_str(), |(_, name)| name);
                let path: Vec<String> = wire.path.iter().map(super::reference::text_of).collect();
                format!(
                    "\"{name}\" is wired to {} of \"{}\", which has no value in this run.",
                    if path.is_empty() { "the output".to_string() } else { path.join(".") },
                    source.map_or(wire.from.as_str(), Node::label),
                )
            })?;
            values.push((wire.input.clone(), value));
        }
        let mut state = self.state.lock();
        for wire in self.flow.wires_into(&node.id) {
            *state.edges.entry(wire.id.clone()).or_default() += 1;
            state.changed_edges.insert(wire.id.clone());
        }
        let inputs: serde_json::Map<String, Value> = values.iter().cloned().collect();
        state.nodes.entry(node.id.clone()).or_default().inputs = preview(&Value::Object(inputs));
        Ok(values)
    }

    /// Remember something the run connected, to close it when the run ends.
    pub fn keep(&self, live: Live) {
        self.live.lock().push(live);
    }

    fn change(&self, node: &Node, change: impl FnOnce(&mut NodeState)) {
        let mut state = self.state.lock();
        change(state.nodes.entry(node.id.clone()).or_default());
        state.changed_nodes.insert(node.id.clone());
    }

    fn begin(&self, node: &Node, input: &Value) {
        let input = preview(input);
        self.state.lock().at = Some(node.label().to_string());
        self.change(node, |state| {
            state.status = "running";
            state.runs += 1;
            state.port = None;
            state.error = None;
            state.input = input;
        });
    }

    /// A stream node is connected and waits for messages.
    pub fn waiting(&self, node: &Node) {
        self.change(node, |state| state.status = "listening");
    }

    /// A stream node received `message`. Its `runs` stay the times the flow reached it: how many messages it
    /// handed on is what its connection counts.
    pub fn received(&self, node: &Node, port: &str, message: &Value) {
        let output = preview(message);
        self.change(node, |state| {
            state.produced(port, output);
            state.handed_on += 1;
        });
    }

    /// Something went wrong at a node that does not end it.
    pub fn note(&self, node: &Node, error: String) {
        self.change(node, |state| state.error = Some(error));
    }

    /// Keep and count the tests of a request's answer, or the checks of a validate node.
    fn tested(&self, node: &Node, tests: &[Value]) {
        let mut state = self.state.lock();
        state.total_tests += tests.len() as u64;
        state.passed_tests += tests.iter().filter(|test| test["passed"] == true).count() as u64;
        state.nodes.entry(node.id.clone()).or_default().tests = json!(tests);
        state.changed_nodes.insert(node.id.clone());
    }

    fn settle(&self, node: &Node, port: &str, output: &Value, took: Duration) {
        let shown = preview(output);
        let failed = port == PORT_ERROR;
        let error = failed.then(|| opt_str(output, "error").unwrap_or("Failed").to_string());
        self.change(node, |state| {
            state.status = if failed { "failed" } else { "succeeded" };
            state.port = Some(port.to_string());
            state.error = error.or(state.error.take());
            state.duration_ms = Some(took.as_millis() as u64);
            state.produced(port, shown);
        });
    }

    /// End the run as failed. The first failure is the one the run reports.
    pub fn fail(&self, message: String) {
        self.state.lock().error.get_or_insert(message);
        self.cancel.cancel();
    }

    fn node_value(node: &Node, state: &NodeState, whole: bool) -> Value {
        let mut out = json!({
            "id": node.id, "name": node.label(), "status": state.status, "runs": state.runs,
            "port": state.port, "error": state.error, "durationMs": state.duration_ms, "handedOn": state.handed_on,
        });
        if whole {
            out["input"] = state.input.clone();
            out["inputs"] = state.inputs.clone();
            out["output"] = state.output.clone();
            out["outputs"] = json!(state.outputs);
            out["tests"] = state.tests.clone();
        }
        out
    }

    /// The run as a client reads it back: every node in the flow's order, and how often each edge was taken.
    pub fn snapshot(&self) -> Value {
        let state = self.state.lock();
        let idle = NodeState::default();
        let nodes: Vec<Value> = self
            .flow
            .nodes
            .iter()
            .map(|node| Self::node_value(node, state.nodes.get(&node.id).unwrap_or(&idle), true))
            .collect();
        let edges: Vec<Value> = self
            .flow
            .edges
            .iter()
            .map(|edge| json!({ "id": edge.id, "count": state.edges.get(&edge.id).copied().unwrap_or(0) }))
            .collect();
        json!({
            "workspaceId": self.ws_id, "flowId": self.flow.id, "flowName": self.flow.name, "runId": self.run_id,
            "status": state.status, "error": state.error, "startedAt": self.started_at, "finishedAt": state.finished_at,
            "inputs": self.inputs, "nodeRuns": self.node_runs.load(Ordering::Relaxed),
            "totalTests": state.total_tests, "passedTests": state.passed_tests,
            "failedTests": state.total_tests - state.passed_tests, "nodes": nodes, "edges": edges,
        })
    }

    /// Report the nodes and edges that changed since the last report, if any did.
    fn report_progress(&self) {
        if self.instance.is_some() {
            return;
        }
        let payload = {
            let mut state = self.state.lock();
            if state.changed_nodes.is_empty() && state.changed_edges.is_empty() {
                return;
            }
            let (changed_nodes, changed_edges) =
                (std::mem::take(&mut state.changed_nodes), std::mem::take(&mut state.changed_edges));
            let nodes: Vec<Value> = self
                .flow
                .nodes
                .iter()
                .filter(|node| changed_nodes.contains(&node.id))
                .filter_map(|node| Some(Self::node_value(node, state.nodes.get(&node.id)?, false)))
                .collect();
            let edges: Vec<Value> = changed_edges
                .iter()
                .map(|id| json!({ "id": id, "count": state.edges.get(id).copied().unwrap_or(0) }))
                .collect();
            json!({
                "workspaceId": self.ws_id, "flowId": self.flow.id, "runId": self.run_id, "status": state.status,
                "nodes": nodes, "edges": edges,
            })
        };
        bus().emit("flow.progress", payload);
    }

    async fn close_live(&self) {
        let live = std::mem::take(&mut *self.live.lock());
        for one in live {
            let mut payload = json!({ "workspaceId": self.ws_id });
            payload[one.id_field] = json!(one.id);
            let _ = self.ctx.registry.invoke(one.disconnect, payload, self.ctx.clone()).await;
        }
    }

    fn finish(&self) {
        {
            let mut state = self.state.lock();
            state.status = match (&state.error, state.cancel_requested) {
                (Some(_), _) => "failed",
                (None, true) => "cancelled",
                (None, false) => "succeeded",
            };
            state.finished_at = Some(now_ms());
            // A node the end of the run caught in the middle of its work did not finish it.
            let caught: Vec<String> = state
                .nodes
                .iter()
                .filter(|(_, node)| matches!(node.status, "running" | "listening"))
                .map(|(id, _)| id.clone())
                .collect();
            for id in caught {
                state.nodes.get_mut(&id).expect("listed above").status = "stopped";
                state.changed_nodes.insert(id);
            }
        }
        if self.instance.is_some() {
            return;
        }
        self.report_progress();
        let report = self.snapshot();
        bus().emit(
            "flow.finished",
            json!({
                "workspaceId": self.ws_id, "flowId": self.flow.id, "runId": self.run_id,
                "status": report["status"], "error": report["error"], "report": report,
            }),
        );
    }
}

/// Hand `output` to every node the port `port` of `node` leads to, and wait for all of them to be done with
/// it. A failure nothing is connected to ends the run.
pub async fn forward(run: &Arc<Run>, node: &Node, port: &str, output: Arc<Value>, token: &Token) {
    let edges: Vec<&Edge> = run.flow.edges_from(&node.id, port).collect();
    if edges.is_empty() {
        if port == PORT_ERROR {
            run.fail(format!("{}: {}", node.label(), opt_str(&output, "error").unwrap_or("failed")));
        }
        return;
    }
    {
        let mut state = run.state.lock();
        for edge in &edges {
            *state.edges.entry(edge.id.clone()).or_default() += 1;
            state.changed_edges.insert(edge.id.clone());
        }
    }
    let mut nodes = (*token.nodes).clone();
    nodes.insert(node.id.clone(), output.clone());
    let next = Token { input: output, nodes: Arc::new(nodes), wave: token.wave };
    join_all(edges.iter().map(|edge| deliver(run.clone(), edge.to.clone(), next.clone()))).await;
}

/// A node is done: record how, then pass what it produced on.
pub async fn fire(run: &Arc<Run>, node: &Node, port: &str, output: Arc<Value>, token: &Token, took: Duration) {
    run.settle(node, port, &output, took);
    forward(run, node, port, output, token).await;
}

/// Run the node `node_id` with `token`, then everything after it.
fn deliver(run: Arc<Run>, node_id: String, token: Token) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        let Some(node) = run.flow.node(&node_id) else { return };
        // A data node only holds a value, and an OR node lets through the first of what reaches it together.
        let merged = || !run.merged.lock().insert((node.id.clone(), token.wave));
        if run.is_cancelled() || node.kind == NodeKind::Data || (node.kind == NodeKind::Or && merged()) {
            return;
        }
        if run.node_runs.fetch_add(1, Ordering::Relaxed) >= MAX_NODE_RUNS {
            return run.fail(format!("The run was stopped after its nodes ran {MAX_NODE_RUNS} times."));
        }
        let started = Instant::now();
        run.begin(node, &token.input);
        let failed = |error: String| (PORT_ERROR.to_string(), Arc::new(json!({ "ok": false, "error": error })));
        let passed_on = |port: &str| (port.to_string(), token.input.clone());
        let (port, output) = match (run.wired(node, &token), node.kind) {
            (Err(error), _) => failed(error),
            (_, NodeKind::Start | NodeKind::Display | NodeKind::Or | NodeKind::Data) => passed_on(PORT_OUT),
            (_, NodeKind::Sample) => (PORT_OUT.to_string(), Arc::new(node.value.clone())),
            (_, NodeKind::Delay) => {
                tokio::time::sleep(Duration::from_millis(node.delay_ms)).await;
                passed_on(PORT_OUT)
            }
            (_, NodeKind::Condition) => {
                let scope = run.scope(&token);
                let taken = node.branches.iter().find(|branch| condition::matches(branch, &scope));
                passed_on(taken.map_or(PORT_ELSE, |branch| branch.id.as_str()))
            }
            (_, NodeKind::Validate) => {
                let scope = run.scope(&token);
                let checks: Vec<Value> = node
                    .rules
                    .iter()
                    .map(|rule| {
                        let name = format!("{} {:?} {}", rule.path, rule.op, rule.value);
                        json!({ "name": name.trim(), "passed": condition::holds(rule, &scope) })
                    })
                    .collect();
                run.tested(node, &checks);
                match checks.iter().filter(|check| check["passed"] != true).count() {
                    0 => passed_on(PORT_PASS),
                    failing => {
                        let error = format!("{failing} of {} checks failed", checks.len());
                        (PORT_ERROR.to_string(), Arc::new(json!({ "ok": false, "error": error, "input": *token.input })))
                    }
                }
            }
            (Ok(wired), NodeKind::Evaluate) => nodes::evaluate(&run, node, &token, &wired).await,
            // What it was handed goes on by the port its expression picks. An expression that throws leaves
            // by a port nothing can be connected to, which ends the run.
            (Ok(wired), NodeKind::If) => match nodes::evaluate(&run, node, &token, &wired).await {
                (port, output) if port == PORT_ERROR => (port, output),
                (_, output) => passed_on(if is_truthy(&output) { PORT_TRUE } else { PORT_FALSE }),
            },
            (_, NodeKind::Repeat | NodeKind::ForEach) => {
                let turns: Option<Vec<Value>> = match node.kind {
                    NodeKind::Repeat => Some((0..node.count).map(|index| json!({ "index": index })).collect()),
                    _ => {
                        let path = if node.path.trim().is_empty() { "input" } else { node.path.as_str() };
                        lookup(path, &run.scope(&token)).as_ref().and_then(Value::as_array).map(|items| {
                            items.iter().enumerate().map(|(index, item)| json!({ "index": index, "item": item })).collect()
                        })
                    }
                };
                match turns {
                    None => failed(format!("{} is not a list in this run.", node.path)),
                    Some(turns) => {
                        // One turn at a time: the next one starts when everything after this one is done.
                        for turn in turns {
                            if run.is_cancelled() {
                                return;
                            }
                            run.received(node, PORT_EACH, &turn);
                            forward(&run, node, PORT_EACH, Arc::new(turn), &run.turn(&token)).await;
                        }
                        passed_on(PORT_DONE)
                    }
                }
            }
            (Ok(wired), NodeKind::Request)
                if matches!(node.request_kind.as_str(), "sseRequests" | "wsConnections" | "socketioRequests") =>
            {
                return streams::listen(&run, node, &token, &wired, started, true).await;
            }
            (Ok(wired), NodeKind::Request) => {
                let answered = nodes::request(&run, node, &token, &wired).await;
                run.tested(node, array_of(&answered.1, "tests"));
                answered
            }
            (Ok(wired), NodeKind::Mcp) => nodes::mcp(&run, node, &token, &wired).await,
            (Ok(wired), NodeKind::Stream) => return streams::listen(&run, node, &token, &wired, started, false).await,
        };
        fire(&run, node, &port, output, &token, started.elapsed()).await;
    })
}

pub(super) async fn drive(run: Arc<Run>, timeout: Option<Duration>) {
    {
        let start = run.flow.nodes.iter().find(|node| node.kind == NodeKind::Start).map(|node| node.id.clone());
        let token = Token { input: Arc::new(run.inputs.clone()), nodes: Arc::default(), wave: 0 };
        let work = async {
            if let Some(start) = start {
                deliver(run.clone(), start, token).await;
            }
        };
        let deadline = async {
            match timeout {
                Some(timeout) => tokio::time::sleep(timeout).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(work, deadline);
        let mut tick = tokio::time::interval(PROGRESS_INTERVAL);
        loop {
            tokio::select! {
                _ = &mut work => break,
                _ = run.cancel.cancelled() => break,
                _ = &mut deadline => {
                    let seconds = timeout.unwrap_or_default().as_secs_f64();
                    run.fail(format!("The run took longer than {seconds} s and was stopped."));
                    break;
                }
                _ = tick.tick() => run.report_progress(),
            }
        }
        // Leaving this block drops whatever the run was in the middle of, before what it connected is closed.
    }
    run.close_live().await;
    run.finish();
}

static RUNS: LazyLock<Mutex<HashMap<String, Arc<Run>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn key_of(ws_id: &str, flow_id: &str) -> String {
    format!("{ws_id}\0{flow_id}")
}

fn find(ws_id: &str, flow_id: &str) -> Option<Arc<Run>> {
    RUNS.lock().get(&key_of(ws_id, flow_id)).cloned()
}

/// The latest run of a flow, running or done, or `None` if it never ran.
pub fn status(ws_id: &str, flow_id: &str) -> Option<Value> {
    find(ws_id, flow_id).map(|run| run.snapshot())
}

/// Stop a flow's run. What it connected is closed and the run ends as `cancelled`.
pub fn cancel(ws_id: &str, flow_id: &str) -> Value {
    match find(ws_id, flow_id) {
        Some(run) => {
            run.stop();
            json!({ "ok": true })
        }
        None => json!({ "ok": false, "error": format!("The flow \"{flow_id}\" has no run in workspace \"{ws_id}\".") }),
    }
}

/// The run's inputs: the flow's defaults, under what the caller gave for the inputs the flow declares.
fn inputs_of(flow: &Flow, given: Option<&Value>) -> std::result::Result<Value, String> {
    let mut inputs: serde_json::Map<String, Value> =
        flow.inputs.iter().map(|input| (input.name.clone(), json!(input.value))).collect();
    for (name, value) in given.and_then(Value::as_object).into_iter().flatten() {
        match inputs.get_mut(name) {
            Some(slot) => *slot = value.clone(),
            None => return Err(format!("This flow has no input named \"{name}\".")),
        }
    }
    Ok(Value::Object(inputs))
}

/// Start a run of a saved flow in the background: `{ ok: true, runId }`, or `{ ok: false, error }` with the
/// flow's `problems` when it cannot run. Progress and the end go out as `flow.progress` / `flow.finished`.
pub fn start(ctx: &CommandContext, params: &Value) -> Result<Value> {
    let plan = match plan(params)? {
        Ok(plan) => plan,
        Err(refusal) => return Ok(refusal),
    };
    let run = {
        let mut runs = RUNS.lock();
        let key = key_of(&plan.ws_id, &plan.flow.id);
        if runs.get(&key).is_some_and(|run| run.state.lock().status == "running") {
            return Ok(json!({ "ok": false, "error": format!(
                "The flow \"{}\" is already running. Cancel it with flow.cancel, or wait for it to finish.",
                plan.flow.name
            ) }));
        }
        let run = plan.run(ctx, None);
        runs.insert(key, run.clone());
        run
    };
    let run_id = run.run_id.clone();
    tokio::spawn(drive(run, plan.timeout));
    Ok(json!({ "ok": true, "runId": run_id }))
}

/// What a run is made from: the saved flow, checked, and what the caller asked of the run.
pub(super) struct Plan {
    pub ws_id: String,
    pub flow: Flow,
    pub timeout: Option<Duration>,
    inputs: Value,
    max_events: Option<u64>,
}

/// Read what a run was asked for. The inner `Err` is the answer to a flow that cannot run as asked.
pub(super) fn plan(params: &Value) -> Result<std::result::Result<Plan, Value>> {
    let (ws_id, flow_id) = (str_of(params, "workspaceId"), str_of(params, "id"));
    let entity = load_entity(ws_id, "flows", flow_id)
        .ok_or_else(|| EngineError::not_found(format!("No saved flow \"{flow_id}\" in workspace \"{ws_id}\".")))?;
    let flow = Flow::parse(&entity)?;

    let described = validate::describe(ws_id, &flow);
    if described["ok"] != true {
        let count = array_of(&described, "problems").iter().filter(|problem| problem["severity"] == "error").count();
        return Ok(Err(json!({
            "ok": false, "problems": described["problems"],
            "error": format!("This flow cannot run: it has {count} problem(s) to fix first."),
        })));
    }
    let inputs = match inputs_of(&flow, params.get("inputs")) {
        Ok(inputs) => inputs,
        Err(error) => return Ok(Err(json!({ "ok": false, "error": error }))),
    };
    let timeout_ms =
        params.get("timeoutMs").and_then(Value::as_u64).or(flow.settings.timeout_ms).unwrap_or(DEFAULT_TIMEOUT_MS);
    Ok(Ok(Plan {
        ws_id: ws_id.to_string(),
        flow,
        timeout: (timeout_ms > 0).then(|| Duration::from_millis(timeout_ms)),
        inputs,
        max_events: params.get("maxEvents").and_then(Value::as_u64),
    }))
}

impl Plan {
    /// A run of the plan, not started yet: [`drive`] runs it.
    pub(super) fn run(&self, ctx: &CommandContext, instance: Option<usize>) -> Arc<Run> {
        Arc::new(Run {
            ws_id: self.ws_id.clone(),
            flow: self.flow.clone(),
            run_id: generate_id(),
            ctx: ctx.clone(),
            inputs: self.inputs.clone(),
            instance,
            max_events: self.max_events,
            events: AtomicU64::new(0),
            mcp: tokio::sync::Mutex::new(HashSet::new()),
            listening: Mutex::new(HashSet::new()),
            started_at: now_ms(),
            cancel: CancellationToken::new(),
            node_runs: AtomicU64::new(0),
            waves: AtomicU64::new(0),
            merged: Mutex::new(HashSet::new()),
            variables: Mutex::new(Vec::new()),
            live: Mutex::new(Vec::new()),
            state: Mutex::new(State {
                status: "running",
                error: None,
                cancel_requested: false,
                finished_at: None,
                at: None,
                nodes: HashMap::new(),
                edges: HashMap::new(),
                changed_nodes: HashSet::new(),
                changed_edges: HashSet::new(),
                total_tests: 0,
                passed_tests: 0,
            }),
        })
    }
}

/// Forget every run. Tests only.
#[cfg(test)]
pub(crate) fn reset_runs() {
    RUNS.lock().clear();
}
