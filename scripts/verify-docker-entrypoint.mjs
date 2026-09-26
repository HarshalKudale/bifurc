/**
 * End-to-end check of the Docker entrypoint — `plan/17` §5 work item 5.
 *
 * ## What it is for
 *
 * Work item 5's claim is a **three-way agreement** that no unit test can see: the static server binds
 * `companion + 1`, the host is told `--gui-target 127.0.0.1:<companion + 1>`, and the healthcheck probes the
 * companion — all three derived from **one** number resolved once. `tests/docker/entrypoint.test.ts` covers
 * the precedence of that number and `tests/integration/guiStaticServer.integration.test.ts` covers the static
 * server over real sockets, but neither starts the real entrypoint, and a test that renders a component
 * directly cannot see that its parent never mounts it. This starts the real thing.
 *
 * ## Usage
 *
 * ```bash
 * npm run build:web            # the entrypoint refuses to start without dist/web/index.html
 * npm run verify:docker-entrypoint
 * npm run verify:docker-entrypoint -- --companion-port 5000   # exercises BIFURC_COMPANION_PORT, GUI -> 5001
 * npm run verify:docker-entrypoint -- --keep-data
 * ```
 *
 * ## It is owner-run, not a suite, and it binds four ports
 *
 * It starts the real host, so it binds `80` (the edge), `8080` (the forward proxy) and the companion/GUI pair
 * (`4747`/`4748` by default). That is exactly why it is a script and not a vitest case: a suite that binds
 * ports degrades every other suite's "the scan lands on `base + N`" assertion. It also needs
 * `packages/engine/dist`, so `npm run build:packages` has to have run.
 *
 * ## The edge check is informational
 *
 * A busy `80` is a **reported** bind failure rather than a thrown one (`plan/17` §1), so the host keeps
 * running and something else may be answering on that port. That check is therefore reported and not counted
 * as a failure; everything else is asserted.
 *
 * ## What it deliberately does not check
 *
 * `SIGTERM` forwarding. Measured 2026-09-25: a Node child on Windows cannot catch `SIGTERM` at all —
 * `process.kill(pid, "SIGTERM")` terminates it unconditionally, the handler never runs, and the parent sees
 * `code=null signal=SIGTERM`. So that path is exercised by `docker stop` in the container, and is reported
 * here rather than asserted.
 */
import { spawn } from "node:child_process";
import { mkdir, readFile, rm } from "node:fs/promises";
import { createConnection } from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { PORT_FILE, resolveCompanionPort } from "./docker-entrypoint.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const APP_ROOT = path.resolve(HERE, "..");

// ── arguments ────────────────────────────────────────────────────────────────
const argv = process.argv.slice(2);
const flag = (name) => {
  const i = argv.indexOf(name);
  return i === -1 ? undefined : argv[i + 1];
};
const keepData = argv.includes("--keep-data");
const requestedPort = flag("--companion-port");
const dataDir = path.resolve(flag("--data-dir") ?? path.join(APP_ROOT, ".tmp-verify-entrypoint-data"));

/** The port the entrypoint should resolve: the environment's when asked for, otherwise the engine's default. */
const expectedCompanion = requestedPort === undefined ? resolveCompanionPort([], {}, 4747) : Number(requestedPort);
if (!Number.isInteger(expectedCompanion) || expectedCompanion < 1) {
  console.error(`--companion-port must be a port number (got ${JSON.stringify(requestedPort)})`);
  process.exit(2);
}
const expectedGui = expectedCompanion + 1;

// ── small helpers ────────────────────────────────────────────────────────────
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const results = [];
function check(name, ok, detail) {
  results.push({ name, ok, detail });
  console.log(`  ${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  — ${detail}` : ""}`);
}
function note(name, detail) {
  console.log(`  ....  ${name}  — ${detail}`);
}

async function waitForPort(port, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const ok = await new Promise((resolve) => {
      const s = createConnection({ port, host: "127.0.0.1" });
      const done = (v) => {
        s.destroy();
        resolve(v);
      };
      s.setTimeout(400);
      s.once("connect", () => done(true));
      s.once("timeout", () => done(false));
      s.once("error", () => done(false));
    });
    if (ok) return true;
    await sleep(200);
  }
  return false;
}

async function waitForPortFile(timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const text = await readFile(PORT_FILE, "utf8").catch(() => undefined);
    if (text !== undefined && text.trim() !== "") return text.trim();
    await sleep(150);
  }
  return null;
}

/** A status line, or a thrown-error string. Never throws. */
async function probe(url, headers) {
  try {
    const res = await fetch(url, { headers, redirect: "manual" });
    const body = await res.text();
    return { status: res.status, type: res.headers.get("content-type") ?? "", bytes: body.length };
  } catch (err) {
    return { error: err instanceof Error ? err.message : String(err) };
  }
}

const DOCUMENT = { accept: "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8" };
const describe = (r) => (r.error ? `fetch failed: ${r.error}` : `${r.status} ${r.type} bytes=${r.bytes}`);

function runNode(script, extraEnv = {}) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [script], {
      cwd: APP_ROOT,
      env: { ...process.env, ...extraEnv },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let out = "";
    let err = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (err += d));
    child.on("exit", (code, signal) => resolve({ code, signal, out: out.trim(), err: err.trim() }));
  });
}

// ── run ──────────────────────────────────────────────────────────────────────
console.log(`companion ${expectedCompanion}, gui ${expectedGui} (derived), data ${dataDir}`);
console.log(`port file ${PORT_FILE}\n`);

await rm(PORT_FILE, { force: true }).catch(() => {});
await rm(dataDir, { recursive: true, force: true }).catch(() => {});
await mkdir(dataDir, { recursive: true });

const entryEnv = { ...process.env };
if (requestedPort !== undefined) entryEnv.BIFURC_COMPANION_PORT = String(expectedCompanion);

const entry = spawn(
  process.execPath,
  [path.join(HERE, "docker-entrypoint.mjs"), "--data-dir", dataDir],
  { cwd: APP_ROOT, env: entryEnv, stdio: ["ignore", "inherit", "inherit"] },
);
const exited = new Promise((resolve) => entry.on("exit", (code, signal) => resolve({ code, signal })));

try {
  const published = await waitForPortFile();
  console.log("");
  check("the entrypoint published a companion port", published !== null, `PORT_FILE = ${published}`);
  check(
    "the published port is the one it resolved",
    published !== null && Number(published) === expectedCompanion,
    `expected ${expectedCompanion}, got ${published}`,
  );

  check(`the companion is listening on ${expectedCompanion}`, await waitForPort(expectedCompanion));
  check(`the web GUI is listening on ${expectedGui} (companion + 1)`, await waitForPort(expectedGui));

  console.log("");
  const health = await runNode(path.join(HERE, "docker-healthcheck.mjs"));
  check(
    "the healthcheck exits 0 against the live entrypoint",
    health.code === 0,
    `exit=${health.code} ${JSON.stringify(health.out)}${health.err ? ` stderr=${JSON.stringify(health.err)}` : ""}`,
  );

  console.log("");
  const gui = `http://127.0.0.1:${expectedGui}`;
  const root = await probe(`${gui}/`, DOCUMENT);
  check("GET / serves the document", root.status === 200 && root.type.startsWith("text/html"), describe(root));

  const deep = await probe(`${gui}/deep/route`, DOCUMENT);
  check("a deep route falls back to index.html (document Accept)", deep.status === 200 && deep.type.startsWith("text/html"), describe(deep));

  const asset = await probe(`${gui}/assets/nope-zzz.js`, { accept: "*/*" });
  check("a missing asset is a 404, not HTML (wildcard Accept)", asset.status === 404, describe(asset));

  const bare = await probe(`${gui}/deep/route`, {});
  check("the same route without a document Accept is a 404", bare.status === 404, describe(bare));

  console.log("");
  const edge = await probe("http://127.0.0.1/", { ...DOCUMENT, host: "localhost" });
  if (edge.status === 200 && edge.type.startsWith("text/html")) {
    check("the edge routes the base domain to --gui-target", true, describe(edge));
  } else {
    note("the edge did not answer with the GUI", `${describe(edge)} — informational: a busy :80 is reported, not thrown`);
  }

  console.log("");
  entry.kill("SIGTERM");
  const code = await exited;
  if (process.platform === "win32") {
    note("SIGTERM forwarding", `not measurable on Windows (exit=${code.code} signal=${code.signal}); docker stop exercises it`);
  } else {
    check("the entrypoint exits 143 (128 + SIGTERM) after forwarding", code.code === 143, `exit=${code.code} signal=${code.signal}`);
  }
  check("the GUI is closed after the host exits", !(await waitForPort(expectedGui, 3000)));
} finally {
  entry.kill("SIGKILL");
  await rm(PORT_FILE, { force: true }).catch(() => {});
  if (!keepData) await rm(dataDir, { recursive: true, force: true }).catch(() => {});
}

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed.`);
if (failed.length > 0) {
  for (const f of failed) console.log(`  FAILED: ${f.name} — ${f.detail}`);
  process.exit(1);
}
console.log("The entrypoint, the static server, the derived GUI port and the healthcheck agree.");
