/**
 * Run the same scenarios against the Node engine and the Rust engine and report where they differ.
 *
 * Each engine gets its own scratch data directory, seeded identically. A scenario is a sequence of commands;
 * every result is recorded after normalizing what legitimately differs between two runs (generated ids,
 * timestamps, durations, ports, the data directory). The two transcripts, the events each engine emitted and
 * the workspace files each one left on disk are then compared.
 *
 *   node scripts/engine-parity.mjs                 every scenario
 *   node scripts/engine-parity.mjs entities mocks  only scenarios whose name contains one of these
 *   node scripts/engine-parity.mjs --only rust     run one engine and print its transcript (no comparison)
 *   node scripts/engine-parity.mjs --verbose       print the transcripts of scenarios that differ in full
 *
 * Build first: `npm run build:packages` and `cargo build -p bifurc-engine`.
 */
import { spawn, execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as http from "node:http";
import * as net from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";
import { scenarios } from "./engine-parity-scenarios.mjs";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const EXE = process.platform === "win32" ? ".exe" : "";
const WS = "wsparity";

const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(name);
  if (i === -1) return undefined;
  const value = args[i + 1];
  args.splice(i, 2);
  return value;
};
const only = flag("--only");
const verbose = args.includes("--verbose") && args.splice(args.indexOf("--verbose"), 1).length > 0;
const filters = args;

const ENGINES = {
  node: {
    command: process.execPath,
    args: [path.join(root, "packages/engine/dist/runtime/hostCli.js")],
    ports: { edge: 18080, proxy: 18081, companion: 18082 },
  },
  rust: {
    command: process.env.BIFURC_ENGINE ?? path.join(root, "target/debug", `bifurc-engine${EXE}`),
    args: [],
    ports: { edge: 18180, proxy: 18181, companion: 18182 },
  },
};

const EVENTS = [
  "event.sync.status",
  "event.sync.entityStatus",
  "event.log.entry",
  "event.server.error",
  "event.entity.changed",
  "event.process.output",
  "event.settings.changed",
  "event.runner.progress",
  "event.runner.finished",
  "event.ws.status",
  "event.ws.message",
];

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** A session with an engine over its socket: 4-byte big-endian length, then one JSON object. */
class Client {
  constructor(socket) {
    this.socket = socket;
    this.pending = new Map();
    this.events = [];
    let buffer = Buffer.alloc(0);
    socket.on("data", (chunk) => {
      buffer = Buffer.concat([buffer, chunk]);
      while (buffer.length >= 4) {
        const length = buffer.readUInt32BE(0);
        if (buffer.length < 4 + length) break;
        const frame = JSON.parse(buffer.subarray(4, 4 + length).toString("utf8"));
        buffer = buffer.subarray(4 + length);
        if (typeof frame.event === "string") this.events.push(frame);
        else this.pending.get(frame.id)?.(frame);
      }
    });
    socket.on("error", () => {});
    socket.on("close", () => {
      for (const resolve of this.pending.values()) resolve({ ok: false, error: { code: "CLOSED", message: "closed" } });
    });
  }

  static connect(address) {
    return new Promise((resolve, reject) => {
      const socket = net.connect(address);
      socket.once("connect", () => resolve(new Client(socket)));
      socket.once("error", reject);
    });
  }

  /** The whole response frame, without its id: `{ ok: true, data }` or `{ ok: false, error }`. */
  request(action, payload = {}, timeoutMs = 30_000) {
    const id = randomUUID();
    const body = Buffer.from(JSON.stringify({ id, action, payload }), "utf8");
    const header = Buffer.alloc(4);
    header.writeUInt32BE(body.length, 0);
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        resolve({ ok: false, error: { code: "HARNESS_TIMEOUT", message: `${action} did not answer in ${timeoutMs}ms` } });
      }, timeoutMs);
      this.pending.set(id, (frame) => {
        clearTimeout(timer);
        this.pending.delete(id);
        const { id: _id, ...rest } = frame;
        resolve(rest);
      });
      this.socket.write(Buffer.concat([header, body]));
    });
  }

  close() {
    this.socket.destroy();
  }
}

function seed(dataDir, ports) {
  fs.rmSync(dataDir, { recursive: true, force: true });
  fs.mkdirSync(dataDir, { recursive: true });
  fs.writeFileSync(
    path.join(dataDir, "app.json"),
    JSON.stringify(
      {
        port: ports.edge,
        forwardProxyPort: ports.proxy,
        companionPort: ports.companion,
        workspaces: [{ id: WS, name: "Parity", activeEnvironmentId: null }],
        activeWorkspaceId: WS,
      },
      null,
      2,
    ),
  );
}

async function startEngine(kind, dataDir) {
  const spec = ENGINES[kind];
  if (!fs.existsSync(spec.args[0] ?? spec.command)) {
    throw new Error(`the ${kind} engine is not built: ${spec.args[0] ?? spec.command}`);
  }
  seed(dataDir, spec.ports);
  const log = fs.openSync(path.join(dataDir, "engine.log"), "a");
  const child = spawn(spec.command, [...spec.args, "--data-dir", dataDir, "--no-tray"], {
    stdio: ["ignore", log, log],
    windowsHide: true,
    env: { ...process.env, BIFURC_DATA_DIR: "", BIFURC_PROXY_BASE_PORT: "" },
  });
  fs.closeSync(log);
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) break;
    try {
      const info = JSON.parse(fs.readFileSync(path.join(dataDir, "runtime.json"), "utf8"));
      if (info.pid === child.pid) {
        const client = await Client.connect(info.address);
        const hello = await client.request("hello", { protocolVersion: "1.0.0", clientName: "desktop-ui", clientVersion: "parity" });
        if (!hello.ok) throw new Error(`hello refused: ${JSON.stringify(hello.error)}`);
        await client.request("subscribe", { events: EVENTS });
        return { kind, child, client, dataDir, ports: spec.ports, hello: hello.data };
      }
    } catch (err) {
      if (err.code !== "ENOENT" && !(err instanceof SyntaxError)) throw err;
    }
    await sleep(100);
  }
  child.kill();
  throw new Error(`the ${kind} engine did not start:\n${fs.readFileSync(path.join(dataDir, "engine.log"), "utf8")}`);
}

async function stopEngine(engine) {
  await engine.client.request("engine.shutdown", {}, 5_000);
  engine.client.close();
  const deadline = Date.now() + 10_000;
  while (engine.child.exitCode === null && Date.now() < deadline) await sleep(50);
  if (engine.child.exitCode === null) engine.child.kill();
}

/** What legitimately differs between two runs, replaced by stable placeholders. */
function makeNormalizer(engine) {
  const ids = new Map();
  const dataDirs = [engine.dataDir, engine.dataDir.replace(/\\/g, "/"), engine.dataDir.replace(/\//g, "\\")];
  const volatileNumberKey = /(At|Ms|^ts|^timestamp|^pid|^seq|^size|^latency|^responseTime|^elapsed|^startTime|^endTime)$/;
  const volatileStringKey = /^(sessionId|commitHash|hash|sha|date|blobId|runId|startedAtIso|lastSyncedCommit)$/;
  const idPattern = /\bm[0-9a-z]{7}[0-9a-z]{4,5}\b/g;
  const text = (value) => {
    let out = value;
    for (const dir of dataDirs) out = out.split(dir).join("<dataDir>");
    out = out.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, "<uuid>");
    out = out.replace(idPattern, (id) => {
      if (!ids.has(id)) ids.set(id, `<id:${ids.size + 1}>`);
      return ids.get(id);
    });
    out = out.split(String(engine.ports.edge)).join("<edgePort>");
    out = out.split(String(engine.ports.proxy)).join("<proxyPort>");
    out = out.split(String(engine.ports.companion)).join("<companionPort>");
    return out;
  };
  const walk = (value, key) => {
    if (typeof value === "string") return volatileStringKey.test(key ?? "") ? "<volatile>" : text(value);
    if (typeof value === "number") {
      if (volatileNumberKey.test(key ?? "")) return "<n>";
      if (value === engine.ports.edge) return "<edgePort>";
      if (value === engine.ports.proxy) return "<proxyPort>";
      if (value === engine.ports.companion) return "<companionPort>";
      return value;
    }
    if (Array.isArray(value)) return value.map((item) => walk(item, key));
    if (value !== null && typeof value === "object") {
      const out = {};
      for (const [k, v] of Object.entries(value)) out[text(k)] = walk(v, k);
      return out;
    }
    return value;
  };
  return walk;
}

/** The workspace's files, as `relative path -> normalized content`. */
function snapshotDisk(engine, normalize) {
  const base = path.join(engine.dataDir, "data", WS);
  const files = {};
  const visit = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.name === ".git") continue;
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        visit(full);
        continue;
      }
      const rel = normalize(path.relative(base, full).replace(/\\/g, "/"));
      const raw = fs.readFileSync(full, "utf8");
      try {
        files[rel] = normalize(JSON.parse(raw));
      } catch {
        files[rel] = normalize(raw);
      }
    }
  };
  if (fs.existsSync(base)) visit(base);
  let commits = [];
  try {
    commits = execFileSync("git", ["log", "--format=%s"], { cwd: base, encoding: "utf8" }).trim().split("\n").map((l) => normalize(l));
  } catch {
    commits = ["<no git log>"];
  }
  return { files, commits };
}

/** A stable upstream for request and proxy scenarios. */
function startUpstream() {
  const server = http.createServer((req, res) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const body = Buffer.concat(chunks).toString("utf8");
      const url = new URL(req.url, "http://upstream");
      if (url.pathname === "/status") {
        res.writeHead(Number(url.searchParams.get("code") ?? 200), { "content-type": "text/plain" });
        res.end("status");
        return;
      }
      if (url.pathname === "/html") {
        res.writeHead(200, { "content-type": "text/html; charset=utf-8", "x-upstream": "yes" });
        res.end("<h1>upstream</h1>");
        return;
      }
      if (url.pathname === "/graphql") {
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify({ data: { echo: JSON.parse(body || "{}") } }));
        return;
      }
      if (url.pathname === "/soap") {
        res.writeHead(200, { "content-type": "text/xml" });
        res.end(`<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"><soap:Body><Echo>${body.length}</Echo></soap:Body></soap:Envelope>`);
        return;
      }
      const headers = {};
      for (const [k, v] of Object.entries(req.headers)) {
        if (!["host", "connection", "accept-encoding", "user-agent", "accept"].includes(k)) headers[k] = v;
      }
      res.writeHead(200, { "content-type": "application/json", "x-upstream": "yes" });
      res.end(JSON.stringify({ method: req.method, path: req.url, headers, body }));
    });
  });
  return new Promise((resolve) => server.listen(18090, "127.0.0.1", () => resolve(server)));
}

/** An HTTP request made by the harness itself, e.g. through an engine's proxy. */
export function httpRequest({ port, method = "GET", requestPath, headers = {}, body }) {
  return new Promise((resolve) => {
    const req = http.request({ host: "127.0.0.1", port, method, path: requestPath, headers, agent: false }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => {
        const resHeaders = { ...res.headers };
        delete resHeaders.date;
        resolve({ status: res.statusCode, headers: resHeaders, body: Buffer.concat(chunks).toString("utf8") });
      });
    });
    req.on("error", (err) => resolve({ error: err.code ?? err.message }));
    req.setTimeout(10_000, () => req.destroy(new Error("timeout")));
    if (body !== undefined) req.write(body);
    req.end();
  });
}

async function runScenario(engine, scenario) {
  const normalize = makeNormalizer(engine);
  const transcript = [];
  engine.client.events.length = 0;
  const t = {
    ws: WS,
    ports: engine.ports,
    upstream: "http://127.0.0.1:18090",
    /** Run a command and record its normalized result. Returns the raw result data (or undefined). */
    async call(action, payload = {}) {
      const result = await engine.client.request(action, payload);
      // A schema failure's wording is the validator's own; that it failed, and for which command, is what counts.
      const message = result.error?.message?.replace(/^(Invalid payload for command "[^"]+").*$/s, "$1");
      transcript.push({ call: action, result: normalize(result.ok ? { ok: true, data: result.data } : { ok: false, code: result.error?.code, message, details: result.error?.details }) });
      return result.ok ? result.data : undefined;
    },
    /** Run a command without recording it (setup whose result is checked elsewhere). */
    async raw(action, payload = {}) {
      return engine.client.request(action, payload);
    },
    /** Record an arbitrary value. */
    note(label, value) {
      transcript.push({ note: label, value: normalize(value) });
    },
    http: httpRequest,
    sleep,
    dataDir: engine.dataDir,
  };
  try {
    await scenario.run(t);
  } catch (err) {
    transcript.push({ harnessError: String(err?.stack ?? err) });
  }
  await sleep(scenario.settleMs ?? 400);
  // Event order across independent emitters is timing; compare what was emitted, not when.
  const events = engine.client.events
    .filter((e) => !(scenario.ignoreEvents ?? []).includes(e.event))
    .map((e) => JSON.stringify(normalize({ event: e.event, payload: e.payload })))
    .sort();
  return { transcript, events, disk: scenario.skipDisk ? null : snapshotDisk(engine, normalize) };
}

function diffValues(a, b, at, out) {
  if (out.length >= 40) return;
  if (JSON.stringify(a) === JSON.stringify(b)) return;
  const isObj = (v) => v !== null && typeof v === "object";
  if (isObj(a) && isObj(b) && Array.isArray(a) === Array.isArray(b)) {
    const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
    for (const key of keys) diffValues(a[key], b[key], `${at}${Array.isArray(a) ? `[${key}]` : `.${key}`}`, out);
    if (!Array.isArray(a) && out.length === 0 && JSON.stringify(Object.keys(a)) !== JSON.stringify(Object.keys(b))) {
      out.push(`${at}: key order\n      node: ${Object.keys(a).join(",")}\n      rust: ${Object.keys(b).join(",")}`);
    }
    return;
  }
  const show = (v) => (v === undefined ? "<absent>" : JSON.stringify(v)?.slice(0, 300));
  out.push(`${at}\n      node: ${show(a)}\n      rust: ${show(b)}`);
}

async function main() {
  const selected = scenarios.filter((s) => filters.length === 0 || filters.some((f) => s.name.includes(f)));
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-parity-"));
  const upstream = await startUpstream();
  const kinds = only ? [only] : ["node", "rust"];
  const results = {};
  let failed = 0;
  try {
    for (const kind of kinds) {
      results[kind] = [];
      for (const scenario of selected) {
        // A fresh engine and data directory per scenario: scenarios cannot affect each other.
        const engine = await startEngine(kind, path.join(scratch, `${kind}-${scenario.name}`));
        try {
          results[kind].push(await runScenario(engine, scenario));
        } finally {
          await stopEngine(engine);
        }
      }
    }
    if (only) {
      selected.forEach((scenario, i) => {
        console.log(`\n=== ${scenario.name}`);
        console.log(JSON.stringify(results[only][i], null, 1));
      });
      return;
    }
    selected.forEach((scenario, i) => {
      const [a, b] = [results.node[i], results.rust[i]];
      const diffs = [];
      diffValues(a.transcript, b.transcript, "transcript", diffs);
      const eventDiffs = [];
      diffValues(a.events, b.events, "events", eventDiffs);
      diffs.push(...eventDiffs.slice(0, 3));
      if (eventDiffs.length > 3) diffs.push(`… and ${eventDiffs.length - 3} more event differences`);
      if (a.disk || b.disk) diffValues(a.disk, b.disk, "disk", diffs);
      if (diffs.length === 0) {
        console.log(`  ok    ${scenario.name} (${a.transcript.length} steps, ${a.events.length} events)`);
        return;
      }
      failed += 1;
      console.log(`  DIFF  ${scenario.name}`);
      for (const diff of diffs) console.log(`    ${diff}`);
      if (verbose) {
        console.log("    --- node transcript\n" + JSON.stringify(a.transcript, null, 1));
        console.log("    --- rust transcript\n" + JSON.stringify(b.transcript, null, 1));
      }
    });
    console.log(`\n${selected.length - failed}/${selected.length} scenarios identical`);
  } finally {
    upstream.close();
    if (failed === 0 && !only) fs.rmSync(scratch, { recursive: true, force: true });
    else console.log(`scratch data kept in ${scratch}`);
  }
  process.exitCode = failed === 0 ? 0 : 1;
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 2;
});
