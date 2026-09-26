/**
 * Smoke-check the browser host in a real Chromium.
 *
 * ## What this is for
 *
 * `e2e/` drives the **Electron** app and needs `npm run build` plus a desktop session. This drives the
 * **web** host, needs neither, and asks one question the unit suites structurally cannot: does the page
 * load, connect, render, and stay **quiet**? A rejected promise inside a React effect never reaches Vitest —
 * that is precisely how the `loadConfig` defect (an uncaught `EngineError` and a UI stuck on "Loading
 * workspace…") survived a green suite.
 *
 * It is a smoke check, not a suite: it does not replace work item 6, which still owes Chromium, Firefox and
 * WebKit plus the blob and reconnect flows. It exists so the common regressions are one command away.
 *
 * ## Usage
 *
 * Start a host first, then:
 *
 * ```bash
 * node packages/engine/dist/runtime/hostCli.js --data-dir ./data --ws-port 8080
 * npm run dev:web                     # in another shell, on 5199
 * node scripts/verify-web-ui.mjs --port 8080
 * ```
 *
 * **`--data-dir` is gone** (2026-09-25), and it existed for exactly one reason: the host's `engine.token`
 * lived there, and a browser cannot read a file, so this script read it on the user's behalf and typed it
 * into the form. The token is gone, so the flag is gone. A host that is not running is now reported by the
 * **form** rather than by a precondition check here — which is a strictly better place for it, because that
 * is where the user sees it too.
 *
 * ## Why it does not use `npx playwright install`
 *
 * The project already depends on Playwright, and a Chromium is usually already in the cache — just at a
 * different revision than the installed Playwright expects (`chromium-1243` against Playwright 1.60's
 * `1223` at the time of writing). Downloading ~500 MB to close a revision gap is not a trade worth making,
 * so the binary is **discovered** and passed as `executablePath`. Override with `--chrome <path>`.
 *
 * ## Two traps this script exists to avoid repeating
 *
 * - **A 404 arrives as a console error with no URL**, and `page.on("response")` does not see it when
 *   Chromium makes the request itself (a favicon, say). `consoleMessage.location()` is what carries the URL.
 * - **A fixed `waitForTimeout` reports the app's speed, not its correctness.** An early version of this
 *   check sampled mid-load and printed `nav 0 / main 0 / "Loading workspace…"` — a broken-looking shell that
 *   was merely slower than the sleep. Wait on a **selector**; it either appears or it does not.
 */
import fs from "node:fs";
import path from "node:path";
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
const HOST = arg("host", "127.0.0.1");
const PORT = arg("port", "8080");

/**
 * The form has exactly two fields — host and port — and this constant is the assertion, not a coincidence.
 *
 * The third used to be **Token**, and its absence is the thing this check now pins. It is worth pinning
 * because the failure it guards against is *silent*: `inputs.nth(2).fill(…)` on a two-field form would fill
 * nothing and throw, but a script that merely stopped filling the third field would pass while the token
 * input came back. Counting is what makes the regression loud.
 */
const FORM_FIELD_COUNT = 2;

/**
 * The newest Chromium in Playwright's cache, whatever revision it is.
 *
 * Revision-pinned discovery rather than a hardcoded path: the cache is shared across Playwright versions,
 * so pinning a revision would break the script on the next upgrade for no reason.
 */
function findChromium(explicit) {
  if (explicit) return explicit;
  const root = process.env.LOCALAPPDATA || process.env.HOME;
  if (!root) return undefined;
  const cache = path.join(root, "ms-playwright");
  if (!fs.existsSync(cache)) return undefined;

  const candidates = fs
    .readdirSync(cache)
    .filter((n) => n.startsWith("chromium-"))
    .map((n) => path.join(cache, n, "chrome-win64", "chrome.exe"))
    .filter((p) => fs.existsSync(p));
  return candidates.sort().pop();
}

const exe = findChromium(arg("chrome", undefined));
if (exe === undefined) {
  console.error(
    "No Chromium found in Playwright's cache. Run `npx playwright install chromium`, or pass --chrome <path>.",
  );
  process.exit(2);
}

const say = (label, value) => console.log("  " + String(label).padEnd(30) + value);

const browser = await chromium.launch({ headless: true, executablePath: exe });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });

const consoleErrors = [];
const consoleWarnings = [];
const pageErrors = [];
page.on("console", (m) => {
  const at = m.location()?.url ? `  @${m.location().url}` : "";
  if (m.type() === "error") consoleErrors.push(m.text() + at);
  else if (m.type() === "warning") consoleWarnings.push(m.text() + at);
});
page.on("pageerror", (e) => pageErrors.push(e.message));

try {
  console.log("=== the connection form ===");
  await page.goto(APP_URL, { waitUntil: "load" });
  await page.waitForSelector("form");
  say("heading", await page.locator("h1").first().textContent());

  const inputs = page.locator("input");
  const fieldCount = await inputs.count();
  say("form fields", `${fieldCount} (expected ${FORM_FIELD_COUNT})`);

  console.log("=== connect ===");
  await inputs.nth(0).fill(HOST);
  await inputs.nth(1).fill(PORT);
  await page.getByRole("button", { name: /connect/i }).click();

  // The form disappearing is the container's own signal that the handshake and the probe both succeeded.
  await page.waitForSelector("form", { state: "detached", timeout: 20000 });
  say("form gone", "yes");
  say("connected bar", await page.getByText(/Connected to/i).count());

  console.log("=== the shell ===");
  // A fresh profile lands on the first-launch gate, so the shell is only reachable after this.
  const accept = page.getByRole("button", { name: /accept/i });
  if ((await accept.count()) > 0) await accept.first().click();
  let reached = true;
  try {
    await page.waitForSelector("nav", { timeout: 30000 });
  } catch {
    reached = false;
  }
  say("reached the shell", reached);
  say("buttons rendered", await page.locator("button").count());

  const shot = path.join(REPO, "screenshots", "web-ui-verify.png");
  fs.mkdirSync(path.dirname(shot), { recursive: true });
  await page.screenshot({ path: shot });
  say("screenshot", shot);

  console.log("=== the numbers this exists for ===");
  say("console errors", consoleErrors.length);
  consoleErrors.slice(0, 10).forEach((m) => console.log("     ! " + m.slice(0, 200)));
  say("uncaught page errors", pageErrors.length);
  pageErrors.slice(0, 10).forEach((m) => console.log("     ! " + m.slice(0, 260)));
  say("console warnings", consoleWarnings.length);

  const clean =
    consoleErrors.length === 0 &&
    pageErrors.length === 0 &&
    reached &&
    fieldCount === FORM_FIELD_COUNT;
  console.log(clean ? "\nPASS" : "\nFAIL");
  process.exitCode = clean ? 0 : 1;
} finally {
  await browser.close();
}
