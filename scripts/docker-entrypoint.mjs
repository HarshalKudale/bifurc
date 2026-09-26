/**
 * The image's entrypoint — `plan/17` work item 5.
 *
 * ## The one rule it exists to enforce
 *
 * The web GUI's port is **derived, never a setting** (`plan/17` §1: it is `companion + 1`). Deriving it
 * inside the engine would give the engine a document root, which `plan/15`'s *"a target, not a directory"*
 * model exists to prevent. So the derivation happens **here**, once, and its result is handed to both
 * sides: the static server binds `companion + 1`, and the host is told
 * `--gui-target 127.0.0.1:<companion + 1>` so the base domain routes to it.
 *
 * ## Why it pins `--ws-port`
 *
 * `host.ts` **scans** for a free companion/GUI pair when no port is given, and a scanned port is the one
 * thing this script cannot predict — leaving the GUI, the `--gui-target` and the healthcheck each guessing
 * at a different moment. So when the operator does not name a port, this script names one (the default, or
 * `BIFURC_COMPANION_PORT`), and the host binds exactly that. **That is what makes the three agree by
 * construction rather than by coincidence**, which is what the `Dockerfile`'s healthcheck comment asks for.
 *
 * ## Why the static server starts before the host, when `plan/17` §1 says the companion starts first
 *
 * §1's ordering is about the GUI being **told** the companion's port at start, because the desktop host
 * launches the GUI as a child and hands it the port. Nothing is launched for the GUI to be told here: it is
 * a static file server that neither knows nor cares where the companion is, and the handoff to the
 * *browser* happens through `--gui-target` and the connection form. So starting it first is safe, and
 * slightly better — a port it cannot bind is reported **before** a host has been started rather than after.
 *
 * ## Signals
 *
 * Node is PID 1 here, so it must forward rather than rely on the default action. The host stops its own
 * listeners on SIGTERM and exits `128 + signo` (`hostCli.ts`); this script forwards the signal, waits for
 * that exit, closes the static server and exits with the child's code — so `docker stop` reports the same
 * status the host chose, and `stop_grace_period` in `docker-compose.yml` has something to wait for.
 */
import { spawn } from "node:child_process";
import { access, mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createGuiStaticServer } from "./gui-static-server.mjs";

const require = createRequire(import.meta.url);

/**
 * The engine's default companion port, read **lazily**, from the **CommonJS** build.
 *
 * **Lazy** for a reason that is not stylistic: this module is imported by `docker-healthcheck.mjs` and by
 * the suite, and a module-scope `require` would make every one of those imports depend on
 * `packages/engine/dist` having been built. The image always has it; a checkout under test may not.
 *
 * **CJS rather than `import`** because the package's ESM output cannot be deep-imported:
 * `dist/store/appSettings.mjs` carries **extensionless** relative imports (`./paths`), which Node's ESM
 * resolver refuses with `ERR_MODULE_NOT_FOUND`. The CJS output resolves them the way CJS always has, and it
 * is what this image already runs — the `CMD`'s executable is `dist/runtime/hostCli.js`, not the `.mjs`.
 *
 * Importing it at all is `plan/17` §4's rule: `store/appSettings.ts` is *"the one home for the numbers"*, so
 * this must not become a second literal `4747`.
 *
 * Measured 2026-09-25; recorded in `TRAPS.md`.
 */
export function defaultCompanionPort() {
  return require("@bifurc/engine/store/appSettings").DEFAULT_COMPANION_PORT;
}

const HERE = path.dirname(fileURLToPath(import.meta.url));
const APP_ROOT = path.resolve(HERE, "..");

/** The executable, not `host.js` — that one is a library (`createWorkspaceHost`) and running it does nothing. */
const CLI = path.join(APP_ROOT, "packages", "engine", "dist", "runtime", "hostCli.js");

/** The browser host's bundle, built by `npm run build:web` (`web/vite.config.ts` sets this `outDir`). */
const GUI_ROOT = path.join(APP_ROOT, "dist", "web");

/** The GUI binds loopback like every other listener in this image — see `gui-static-server.mjs`. */
const GUI_HOST = "127.0.0.1";

/**
 * Where the resolved companion port is published for `docker-healthcheck.mjs`.
 *
 * A **file** because the healthcheck is a separate process: it cannot see this script's variables, and the
 * `CMD`'s arguments are not visible to it either. Without this, an operator who overrides `--ws-port` would
 * have to override `--health-cmd` too, which is the coincidence the Dockerfile's comment complains about.
 */
export const PORT_FILE = "/tmp/bifurc/companion-port";

/**
 * The companion port, from the flag, then the environment, then the default.
 *
 * Flag-wins-then-env is `hostArgs.ts`'s own precedence for `--gui-target` / `BIFURC_GUI_TARGET`, so an
 * operator has one rule to learn rather than two.
 *
 * **A malformed value throws rather than falling back**, which is `forwardProxyPortFromEnv()`'s rule and
 * for its reason: a typo in a compose file must fail on every path, not only on the path that happens to
 * read it. Falling back would start a host on a port nobody asked for while the GUI, the `--gui-target`
 * and the healthcheck all agreed with each other and disagreed with the operator.
 *
 * `fallback` is a parameter so a caller can state the default without reading the engine's build (see
 * `defaultCompanionPort`) — which is what lets the suite cover the three-way precedence below in a checkout
 * where `packages/engine/dist` has not been built.
 */
export function resolveCompanionPort(argv, env = process.env, fallback = defaultCompanionPort()) {
  const fromFlag = flagValue(argv, "--ws-port");
  if (fromFlag !== undefined) return requirePort(fromFlag, "--ws-port");

  const raw = env.BIFURC_COMPANION_PORT;
  if (raw !== undefined && raw.trim() !== "") {
    return requirePort(raw.trim(), "BIFURC_COMPANION_PORT");
  }

  return fallback;
}

/**
 * The value of `--flag value` or `--flag=value`, or `undefined` when the flag is absent.
 *
 * A flag with no value returns `""` rather than `undefined`, so `requirePort` rejects it — the alternative
 * is treating a dangling `--ws-port` as "not given" and silently binding the default.
 */
function flagValue(argv, flag) {
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === flag) return argv[i + 1] ?? "";
    if (arg.startsWith(`${flag}=`)) return arg.slice(flag.length + 1);
  }
  return undefined;
}

/**
 * A port number, or a throw naming where the bad value came from.
 *
 * The upper bound is **65534, not 65535**, and that is the derivation showing through: the GUI's port is
 * `companion + 1`, so a companion on 65535 would leave the GUI nowhere to live. `findPortPair` carries the
 * same bound for the same reason.
 */
function requirePort(value, source) {
  const port = Number(value);
  if (!Number.isInteger(port) || port < 1 || port > 65534) {
    throw new Error(
      `${source} must be a port between 1 and 65534 (got ${JSON.stringify(value)}). ` +
        `The upper bound is 65534 because the web GUI's port is derived as companion + 1.`,
    );
  }
  return port;
}

async function main() {
  const argv = process.argv.slice(2);
  const companionPort = resolveCompanionPort(argv);
  const guiPort = companionPort + 1;

  /**
   * The GUI bundle is part of what this image promises, so its absence is a **broken image** rather than a
   * degraded mode. Serving nothing while the edge routes the base domain at `--gui-target` would be a base
   * domain that fails — a failure with no explanation at the point the user sees it. An operator who wants
   * the host alone can pass `--entrypoint node …/hostCli.js` and skip this script entirely.
   */
  await access(path.join(GUI_ROOT, "index.html")).catch(() => {
    throw new Error(
      `The web GUI bundle is missing (${path.join(GUI_ROOT, "index.html")}). ` +
        `It is built with \`npm run build:web\` and copied into the image; a tree without it cannot serve ` +
        `the base domain. To run the host alone, bypass this entrypoint.`,
    );
  });

  const gui = await createGuiStaticServer({ root: GUI_ROOT, port: guiPort, host: GUI_HOST });
  console.log(`[entrypoint] web GUI: ${gui.url} (derived, companion ${companionPort} + 1)`);

  await mkdir(path.dirname(PORT_FILE), { recursive: true });
  await writeFile(PORT_FILE, String(companionPort), "utf8");

  const args = [...argv];
  // Pin the port only when the operator did not: an explicit `--ws-port` is their choice and is honoured,
  // exactly as `host.ts` honours it over the scan.
  if (flagValue(argv, "--ws-port") === undefined) args.push("--ws-port", String(companionPort));
  // Same for the target — a second `--gui-target` would be a coin toss between two values.
  if (flagValue(argv, "--gui-target") === undefined && process.env.BIFURC_GUI_TARGET === undefined) {
    args.push("--gui-target", `${GUI_HOST}:${guiPort}`);
  }

  const child = spawn(process.execPath, [CLI, ...args], { stdio: "inherit" });

  for (const signal of ["SIGTERM", "SIGINT"]) {
    process.on(signal, () => child.kill(signal));
  }

  child.on("exit", (code, signal) => {
    void gui.close().finally(() => {
      // The host exits `128 + signo` for a signal it handled, so `code` is normally set. A signal that
      // reached the child before its handler was installed arrives here instead, and is translated the
      // same way rather than reported as a clean exit.
      process.exit(code ?? (signal ? 128 + signalNumber(signal) : 0));
    });
  });
}

/** `SIGTERM` → 15. Only the two signals this script forwards are mapped. */
function signalNumber(signal) {
  return signal === "SIGINT" ? 2 : 15;
}

/**
 * Run only when executed, never when imported.
 *
 * `docker-healthcheck.mjs` imports `PORT_FILE` and `resolveCompanionPort` from this file, and an unguarded
 * top-level `main()` would start a static server and a host inside the healthcheck process.
 */
const invokedDirectly =
  process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href;

if (invokedDirectly) {
  main().catch((err) => {
    console.error(`[entrypoint] ${err instanceof Error ? err.message : String(err)}`);
    process.exit(1);
  });
}
