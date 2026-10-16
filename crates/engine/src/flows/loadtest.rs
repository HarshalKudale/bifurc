//! A load test of a flow: many runs of it started together, and what they came to.
//!
//! Each instance is a run like any other ([`super::run`]) that reports nothing itself. The load test keeps a
//! line for each (how it stands, where it is, what went wrong), counts them, and times every node across
//! them. It says how it goes with `flow.loadtest.progress` and ends with `flow.loadtest.finished`.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, LazyLock},
    time::Duration,
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::{runtime::Runtime, task::JoinSet};

use super::{
    model::NodeKind,
    put_error,
    run::{self, Plan, Run},
    said,
};
use crate::{
    cookies,
    kernel::{CommandContext, bus},
    protocol::Result,
    runtime::failed,
    utils::id::{generate_id, now_ms},
};

/// The most instances one load test may have.
const MAX_INSTANCES: u64 = 10_000;

/// How often a load test reports the instances that changed.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Where the instances run: a thread for each core, so a load test is not held to the one thread the engine
/// answers its clients on, and does not hold that thread up.
static WORKERS: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("bifurc-loadtest")
        .build()
        .expect("the load test runtime starts")
});

/// A node across the instances: how many reached it, how many it failed in, and how long it took.
#[derive(Default)]
struct NodeStat {
    runs: u64,
    failed: u64,
    timed: u64,
    total_ms: u64,
    max_ms: u64,
}

struct State {
    /// `running`, `finished` or `cancelled`.
    status: &'static str,
    finished_at: Option<i64>,
    /// A line for each instance, by its index.
    rows: Vec<Value>,
    /// The instances still running.
    live: HashMap<usize, Arc<Run>>,
    changed: BTreeSet<usize>,
    nodes: HashMap<String, NodeStat>,
}

struct LoadTest {
    ws_id: String,
    flow_id: String,
    flow_name: String,
    load_id: String,
    started_at: i64,
    /// The flow's nodes in its order: id and name.
    node_names: Vec<(String, String)>,
    state: Mutex<State>,
}

/// The line of instance `index`, from its run.
fn row_of(index: usize, run: &Run) -> Value {
    let mut row = run.brief();
    row["index"] = json!(index);
    row
}

/// An instance passed when its run succeeded, as a run by itself would: a failed test fails it only at a
/// node that does not ignore its tests.
fn passed(row: &Value) -> bool {
    row["status"] == "succeeded"
}

/// The value `percent` of the way up a sorted list.
fn percentile(sorted: &[u64], percent: usize) -> Option<u64> {
    sorted.get((sorted.len().checked_sub(1)?) * percent / 100).copied()
}

impl LoadTest {
    /// How the load test stands, without its lines.
    fn summary(&self, state: &State) -> Value {
        let running = state.rows.iter().filter(|row| row["status"] == "running").count();
        let passed = state.rows.iter().filter(|row| passed(row)).count();
        let mut took: Vec<u64> = state.rows.iter().filter_map(|row| row["durationMs"].as_u64()).collect();
        took.sort_unstable();
        let elapsed_ms = (state.finished_at.unwrap_or_else(now_ms) - self.started_at).max(1) as f64;
        json!({
            "workspaceId": self.ws_id, "flowId": self.flow_id, "flowName": self.flow_name, "loadId": self.load_id,
            "status": state.status, "startedAt": self.started_at, "finishedAt": state.finished_at,
            "instances": state.rows.len(), "running": running, "passed": passed,
            "failed": state.rows.len() - running - passed,
            "perSecond": took.len() as f64 * 1000.0 / elapsed_ms,
            "minMs": took.first(), "p50Ms": percentile(&took, 50), "p95Ms": percentile(&took, 95), "p99Ms": percentile(&took, 99),
            "maxMs": took.last(),
            "avgMs": (!took.is_empty()).then(|| took.iter().sum::<u64>() / took.len() as u64),
        })
    }

    /// The whole load test: the summary, every instance's line and every node's times.
    fn snapshot(&self) -> Value {
        let state = self.state.lock();
        let mut out = self.summary(&state);
        out["rows"] = json!(state.rows);
        out["nodes"] = self
            .node_names
            .iter()
            .filter_map(|(id, name)| {
                let stat = state.nodes.get(id)?;
                Some(json!({
                    "id": id, "name": name, "runs": stat.runs, "failed": stat.failed, "maxMs": stat.max_ms,
                    "avgMs": stat.total_ms.checked_div(stat.timed),
                }))
            })
            .collect();
        out
    }

    /// Report the instances whose line changed since the last report, if any did.
    fn report_progress(&self) {
        let payload = {
            let mut guard = self.state.lock();
            let state = &mut *guard;
            for (index, run) in &state.live {
                let row = row_of(*index, run);
                if state.rows[*index] != row {
                    state.rows[*index] = row;
                    state.changed.insert(*index);
                }
            }
            if state.changed.is_empty() {
                return;
            }
            let rows: Vec<&Value> =
                std::mem::take(&mut state.changed).iter().map(|index| &state.rows[*index]).collect();
            let mut payload = self.summary(state);
            payload["rows"] = json!(rows);
            payload
        };
        bus().emit("flow.loadtest.progress", payload);
    }

    /// An instance is done: keep its last line and add its nodes to the times.
    fn settle(&self, index: usize, run: &Run) {
        let mut state = self.state.lock();
        state.rows[index] = row_of(index, run);
        state.changed.insert(index);
        state.live.remove(&index);
        for (id, failed, took) in run.timings() {
            let stat = state.nodes.entry(id).or_default();
            stat.runs += 1;
            stat.failed += u64::from(failed);
            if let Some(ms) = took {
                stat.timed += 1;
                stat.total_ms += ms;
                stat.max_ms = stat.max_ms.max(ms);
            }
        }
    }

    fn finish(&self) {
        {
            let mut state = self.state.lock();
            // An instance whose task died never settled.
            for index in std::mem::take(&mut state.live).into_keys() {
                state.rows[index]["status"] = json!("failed");
                let error = said("flow.instance_stopped", json!({}), "The instance stopped unexpectedly.");
                put_error(&mut state.rows[index], &error);
                state.changed.insert(index);
            }
            if state.status == "running" {
                state.status = "finished";
            }
            state.finished_at = Some(now_ms());
        }
        self.report_progress();
        let report = self.snapshot();
        bus().emit(
            "flow.loadtest.finished",
            json!({
                "workspaceId": self.ws_id, "flowId": self.flow_id, "loadId": self.load_id,
                "status": report["status"], "report": report,
            }),
        );
    }
}

async fn drive(load: Arc<LoadTest>, timeout: Option<Duration>) {
    let mut tasks = JoinSet::new();
    let live: Vec<(usize, Arc<Run>)> =
        load.state.lock().live.iter().map(|(index, run)| (*index, run.clone())).collect();
    for (index, run) in live {
        let load = load.clone();
        tasks.spawn(async move {
            // Each instance is a user of its own: what one signs in with is not another's.
            cookies::private(&run.run_id, run::drive(run.clone(), timeout)).await;
            load.settle(index, &run);
        });
    }
    let mut tick = tokio::time::interval(PROGRESS_INTERVAL);
    loop {
        tokio::select! {
            done = tasks.join_next() => if done.is_none() { break },
            _ = tick.tick() => load.report_progress(),
        }
    }
    load.finish();
}

static LOADS: LazyLock<Mutex<HashMap<String, Arc<LoadTest>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn find(ws_id: &str, flow_id: &str) -> Option<Arc<LoadTest>> {
    LOADS.lock().get(&format!("{ws_id}\0{flow_id}")).cloned()
}

/// The latest load test of a flow, running or done, or `None` if it never had one.
pub fn status(ws_id: &str, flow_id: &str) -> Option<Value> {
    find(ws_id, flow_id).map(|load| load.snapshot())
}

/// Stop a flow's load test: every instance still running ends as `cancelled`.
pub fn cancel(ws_id: &str, flow_id: &str) -> Value {
    let Some(load) = find(ws_id, flow_id) else {
        return failed(&said(
            "flow.no_loadtest",
            json!({ "id": flow_id, "workspace": ws_id }),
            format!("The flow \"{flow_id}\" has no load test in workspace \"{ws_id}\"."),
        ));
    };
    let mut state = load.state.lock();
    if state.status == "running" {
        state.status = "cancelled";
    }
    state.live.values().for_each(|run| run.stop());
    json!({ "ok": true })
}

/// Start `instances` runs of a saved flow together, in the background: `{ ok: true, loadId }`, or
/// `{ ok: false, error }` when the flow cannot run. Takes what `flow.run` takes, for every instance alike.
pub fn start(ctx: &CommandContext, params: &Value) -> Result<Value> {
    let instances = params.get("instances").and_then(Value::as_u64).filter(|n| (1..=MAX_INSTANCES).contains(n));
    let instances = instances.ok_or_else(|| {
        said(
            "flow.loadtest_instances",
            json!({ "max": MAX_INSTANCES }),
            format!("flow.loadtest.start takes `instances`: a whole number from 1 to {MAX_INSTANCES}."),
        )
    })? as usize;
    let mut plan: Plan = match run::plan(params)? {
        Ok(plan) => plan,
        Err(refusal) => return Ok(refusal),
    };
    // Instances are users going through an API side by side, which a tool call of an MCP server is not.
    if let Some(node) = plan.flow.nodes.iter().find(|node| node.kind == NodeKind::Mcp) {
        return Ok(failed(&said(
            "flow.loadtest_mcp",
            json!({ "node": node.label() }),
            format!("A flow with an MCP node runs one instance at a time: \"{}\" calls an MCP server.", node.label()),
        )));
    }
    // Side by side, the instances would write over each other's variables in the environment's file.
    plan.flow.settings.persist_variables = false;

    let load = {
        let mut loads = LOADS.lock();
        let key = format!("{}\0{}", plan.ws_id, plan.flow.id);
        if loads.get(&key).is_some_and(|load| load.state.lock().finished_at.is_none()) {
            return Ok(failed(&said(
                "flow.loadtest_running",
                json!({ "name": plan.flow.name }),
                format!(
                    "A load test of the flow \"{}\" is already running. Cancel it with flow.loadtest.cancel, or wait for it to finish.",
                    plan.flow.name
                ),
            )));
        }
        let live: HashMap<usize, Arc<Run>> = (0..instances).map(|index| (index, plan.run(ctx, Some(index)))).collect();
        let load = Arc::new(LoadTest {
            ws_id: plan.ws_id.clone(),
            flow_id: plan.flow.id.clone(),
            flow_name: plan.flow.name.clone(),
            load_id: generate_id(),
            started_at: now_ms(),
            node_names: plan.flow.nodes.iter().map(|node| (node.id.clone(), node.label().to_string())).collect(),
            state: Mutex::new(State {
                status: "running",
                finished_at: None,
                rows: (0..instances).map(|index| json!({ "index": index, "status": "running" })).collect(),
                live,
                changed: BTreeSet::new(),
                nodes: HashMap::new(),
            }),
        });
        loads.insert(key, load.clone());
        load
    };
    let load_id = load.load_id.clone();
    WORKERS.spawn(drive(load, plan.timeout));
    Ok(json!({ "ok": true, "loadId": load_id, "instances": instances }))
}

/// Forget every load test. Tests only.
#[cfg(test)]
pub(crate) fn reset() {
    LOADS.lock().clear();
}
