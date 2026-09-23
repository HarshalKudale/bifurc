/**
 * Verify reconnect-with-backoff in a real Chromium.
 *
 * `plan/08`'s criterion — *"disconnect/reconnect works; the UI never silently freezes"* — and the one thing
 * a unit suite cannot reach: the container's own loop, driven by a socket dying under a mounted UI.
 *
 * ## The token rotation is why this has two halves
 *
 * `ensureEngineToken()` generates a token on **every** host start rather than reusing one — a token that
 * survives a restart is a credential that survives a restart. So a restarted host is *unreachable with the
 * old token*, and that is the expected case rather than an error. Both halves are worth checking:
 *
 *   1. host dies, restarts with the **same** token (the `BIFURC_ENGINE_TOKEN` / mounted-secret path) →
 *      the UI must recover on its own, with the shell still mounted;
 *   2. host dies, restarts with a **new** token → the loop must **stop** and say so, rather than spinning
 *      forever against a token that can never work again.
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

/** Fixed on purpose: half of this check is a restart with the *same* token. */
const FIXED_TOKEN = "verify-web-reconnect-token";

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
function startHost(token) {
  return new Promise((resolve, reject) => {
    const child = spawn(
      process.execPath,
      ["packages/engine/dist/runtime/hostCli.js", "--data-dir", DATA_DIR, "--ws-port", PORT],
      { cwd: REPO, env: { ...process.env, BIFURC_ENGINE_TOKEN: token } },
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
  host = await startHost(FIXED_TOKEN);

  console.log("=== connect ===");
  await page.goto(APP_URL, { waitUntil: "load" });
  await page.waitForSelector("form");
  const inputs = page.locator("input");
  await inputs.nth(0).fill("127.0.0.1");
  await inputs.nth(1).fill(PORT);
  await inputs.nth(2).fill(FIXED_TOKEN);
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

  console.log("=== restart with the SAME token — it must recover on its own ===");
  host = await startHost(FIXED_TOKEN);
  await page.waitForSelector("text=/reconnecting/i", { state: "detached", timeout: 45000 });
  check("banner gone", true);
  check("shell still there", (await page.locator("nav").count()) === 1);
  check("footer endpoint", (await page.getByText(`ws://127.0.0.1:${PORT}`).count()) > 0);

  console.log("=== restart with a NEW token — it must STOP and say so ===");
  await stopHost(host);
  host = await startHost(FIXED_TOKEN + "-rotated");
  await page.waitForSelector("form", { timeout: 45000 });
  check("returned to the form", true);
  const alert = (await page.locator('[role="alert"]').first().textContent()) ?? "";
  check("token message is actionable", /regenerated/.test(alert));

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
