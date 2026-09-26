/**
 * Verify reconnect-with-backoff in a real Chromium.
 *
 * `plan/08`'s criterion — *"disconnect/reconnect works; the UI never silently freezes"* — and the one thing
 * a unit suite cannot reach: the container's own loop, driven by a socket dying under a mounted UI.
 *
 * ## There used to be a second half, and the token took it with it
 *
 * This check ran **two** scenarios, because `ensureEngineToken()` generated a token on **every** host start
 * rather than reusing one — so a restarted host was *unreachable with the old token*, and that was the
 * expected case rather than an error. Both halves were worth driving:
 *
 *   1. host dies, restarts with the **same** token → the UI must recover on its own, shell still mounted;
 *   2. host dies, restarts with a **new** token → the loop must **stop** and say so, rather than spinning
 *      forever against a token that can never work again.
 *
 * **There is no token any more** (2026-09-25), so scenario 2 has no way to be produced: a restart cannot
 * invalidate anything, and the reconnect either succeeds or fails for an ordinary availability reason. The
 * loop still gives up on a *non-transient* failure — `unauthorized`, and a protocol major mismatch — and
 * that is pinned in `web/tests/connect.test.ts`, where it belongs: it needs a fake endpoint that answers
 * `UNAUTHORIZED`, not a dead host. What is left here is the scenario nothing else can reach, because it
 * needs a **real** socket dying under a **mounted** UI.
 *
 * ## Usage
 *
 * This one starts and kills the **host** itself — it has to, since killing it is the scenario — but it
 * expects the app to be served already:
 *
 * ```bash
 * npm run dev:web                     # in another shell, on 5199
 * npm run verify:web:reconnect
 * ```
 *
 * ## What it caught, and why the assertions are shaped this way
 *
 * The first run of this check found three defects in code that was already written and believed:
 *
 * - **`nav` was `0` during the outage *and* after recovery.** The container rendered a *replacement* tree
 *   while reconnecting, so the UI unmounted and every open tab and draft was discarded — while the comment
 *   beside it claimed the opposite. Hence `UI still mounted` below: an assertion that the shell survives.
 * - **Two `TypeError`s: `Cannot read properties of undefined (reading 'getConfig')`.** `window.api` was
 *   uninstalled on the drop, and a still-mounted UI then read `undefined.getConfig`. Leaving the dead client
 *   installed makes the same call *reject* — a state the UI renders — instead of throwing.
 * - **The error screen replaced the shell** when a refresh failed mid-outage, because `loadError` was
 *   treated as any-load rather than first-load. `shell still there` is what pins that.
 */
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 || process.argv[i + 1] === undefined ? fallback : process.argv[i + 1];
}

const APP_URL = arg("url", "http://localhost:5199/");
const PORT = arg("port", "8099");
const DATA_DIR = path.resolve(REPO, arg("data-dir", ".tmp-reconnect-check"));

const say = (label, value) => console.log("  " + String(label).padEnd(30) + value);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function findChromium(explicit) {
  if (explicit) return explicit;
  const root = process.env.LOCALAPPDATA || process.env.HOME;
  if (!root) return undefined;
  const cache = path.join(root, "ms-playwright");
  if (!fs.existsSync(cache)) return undefined;
  return fs
    .readdirSync(cache)
    .filter((n) => n.startsWith("chromium-"))
    .map((n) => path.join(cache, n, "chrome-win64", "chrome.exe"))
    .filter((p) => fs.existsSync(p))
    .sort()
    .pop();
}

/** Start a host and resolve once it reports its RPC endpoint. */
function startHost() {
  return new Promise((resolve, reject) => {
    const child = spawn(
      process.execPath,
      // **No `--domain`.** It was a required flag until 2026-09-24 and is now removed — the engine is not
      // deployable in the cloud, so it always answers as `localhost` (the host composes that itself). This
      // script's whole subject is a browser on *this* machine talking to an engine on it, which is exactly the
      // case `*.localhost` resolves for, so `localhost` was the honest value to pass and is now the only one.
      // **A stale `--domain` here is loud rather than silent** — the parser refuses an unrecognised flag — which
      // is what the host's own `Dockerfile` relies on too.
      //
      // **No environment variable either.** `BIFURC_ENGINE_TOKEN` used to be set here to *pin* the token, so
      // that a restart reused it and scenario 1 above was reachable at all. There is no token to pin.
      ["packages/engine/dist/runtime/hostCli.js", "--data-dir", DATA_DIR, "--ws-port", PORT],
      { cwd: REPO },
    );
    let out = "";
    const timer = setTimeout(() => reject(new Error("host never reported ready:\n" + out)), 30000);
    const onData = (d) => {
      out += d.toString();
      if (out.includes("rpc endpoint:")) {
        clearTimeout(timer);
        resolve(child);
      }
    };
    child.stdout.on("data", onData);
    child.stderr.on("data", onData);
  });
}

function stopHost(child) {
  return new Promise((resolve) => {
    if (!child || child.exitCode !== null) return resolve();
    child.once("exit", () => resolve());
    child.kill("SIGKILL");
  });
}

const exe = findChromium(arg("chrome", undefined));
if (exe === undefined) {
  console.error("No Chromium in Playwright's cache. Run `npx playwright install chromium`.");
  process.exit(2);
}

fs.rmSync(DATA_DIR, { recursive: true, force: true });
fs.mkdirSync(DATA_DIR, { recursive: true });

const browser = await chromium.launch({ headless: true, executablePath: exe });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
const pageErrors = [];
page.on("pageerror", (e) => pageErrors.push(e.message));

const failures = [];
const check = (label, ok, detail) => {
  say(label, detail ?? (ok ? "yes" : "NO"));
  if (!ok) failures.push(label);
};

let host;
try {
  host = await startHost();

  console.log("=== connect ===");
  await page.goto(APP_URL, { waitUntil: "load" });
  await page.waitForSelector("form");
  // Two fields: host and port. The third used to be the token, and it is gone — see the header. Filling by
  // index rather than by label is deliberate: it is what makes a *returned* token field fail this check.
  const inputs = page.locator("input");
  await inputs.nth(0).fill("127.0.0.1");
  await inputs.nth(1).fill(PORT);
  await page.getByRole("button", { name: /connect/i }).click();
  await page.waitForSelector("form", { state: "detached", timeout: 20000 });

  const accept = page.getByRole("button", { name: /accept/i });
  if ((await accept.count()) > 0) await accept.first().click();
  await page.waitForSelector("nav", { timeout: 30000 });
  check("shell reached", true);
  check("engine version shown", (await page.getByText(/engine 0\./).count()) > 0);

  console.log("=== the host dies — the UI must say so, not freeze ===");
  await stopHost(host);
  host = undefined;
  await page.waitForSelector("text=/reconnecting/i", { timeout: 20000 });
  check("reconnecting banner", true, "shown");
  // The assertion this check exists for: a teardown would discard every open tab for an outage the user
  // may not have noticed.
  check("UI still mounted", (await page.locator("nav").count()) === 1);

  console.log("=== restart it — the UI must recover on its own ===");
  host = await startHost();
  await page.waitForSelector("text=/reconnecting/i", { state: "detached", timeout: 45000 });
  check("banner gone", true);
  check("shell still there", (await page.locator("nav").count()) === 1);
  check("footer endpoint", (await page.getByText(`ws://127.0.0.1:${PORT}`).count()) > 0);

  console.log("=== console health ===");
  check("uncaught page errors", pageErrors.length === 0, pageErrors.length);
  pageErrors.slice(0, 8).forEach((m) => console.log("     ! " + m.slice(0, 220)));
} catch (e) {
  console.error("VERIFY FAILED:", e.message);
  failures.push("threw");
} finally {
  await stopHost(host);
  await browser.close();
  fs.rmSync(DATA_DIR, { recursive: true, force: true });
}

console.log(failures.length === 0 ? "\nPASS" : `\nFAIL — ${failures.join(", ")}`);
process.exitCode = failures.length === 0 ? 0 : 1;
