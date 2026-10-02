/**
 * The standalone engine's presence on a desktop: its tray icon, opening the interface, and when to stop.
 *
 * The engine outlives the interfaces that use it — the UI's window can close and a terminal running the CLI
 * can go away — so it needs a handle of its own that the user can reach. That is the tray icon, and the
 * engine owns it: it is shown while the engine runs, whichever client started it, and it is the way to open
 * the interface again, to restart the engine and to quit.
 *
 * Node cannot place a tray icon, so the icon is a small helper (`bifurc-tray`, built from `ui/crates/tray`)
 * that the engine runs as a child. The two talk in lines over the helper's stdio: the helper prints `ready`
 * once its icon is up, then `open`, `restart` or `quit` as the user picks them; the engine closes the
 * helper's stdin to take the icon down. A helper that is missing, or that cannot show an icon on this
 * platform, leaves the engine without one — and an engine nobody can reach must not linger, so it then stops
 * when its last interface closes. The same applies when the user turned "keep running in the tray" off.
 */
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import * as path from "node:path";
import { createInterface } from "node:readline";
import type { EngineEventBus } from "../kernel/eventBus";
import type { SessionIdentity } from "../kernel/registry";

const EXE = process.platform === "win32" ? ".exe" : "";
export const TRAY_BINARY = `bifurc-tray${EXE}`;
export const UI_BINARY = `bifurc-ui${EXE}`;

/** Clients that are an interface a person looks at, as they name themselves in `hello`. */
const INTERFACE_CLIENTS = new Set(["desktop-ui", "electron-shell", "web-ui"]);

/** How long a launched interface has to connect before another Open launches a second one. */
const LAUNCH_GRACE_MS = 10_000;

export interface DesktopCompanions {
  /** The tray helper's executable, or `null` when the engine runs without an icon. */
  tray: string | null;
  /** The interface the tray's Open starts, or `null` when there is none to start. */
  ui: string | null;
}

/**
 * Where the tray helper and the interface are.
 *
 * An explicit path wins, then `BIFURC_TRAY` / `BIFURC_UI` (how the interface that starts the engine names
 * itself), then a search upward from `searchFrom`: beside the engine in a packaged app, or under `ui/target`
 * in a checkout. The interface defaults to the one built next to the helper.
 */
export function findDesktopCompanions(opts: { tray?: string; ui?: string; searchFrom: string }): DesktopCompanions {
  const env = (name: string): string | undefined => {
    const value = process.env[name];
    return value !== undefined && value !== "" ? value : undefined;
  };

  let tray = opts.tray ?? env("BIFURC_TRAY") ?? null;
  if (tray === null) {
    const starts = [path.dirname(process.execPath), opts.searchFrom];
    search: for (const start of starts) {
      for (let dir = path.resolve(start); ; dir = path.dirname(dir)) {
        for (const candidate of [
          path.join(dir, TRAY_BINARY),
          path.join(dir, "ui", "target", "release", TRAY_BINARY),
          path.join(dir, "ui", "target", "debug", TRAY_BINARY),
        ]) {
          if (existsSync(candidate)) {
            tray = candidate;
            break search;
          }
        }
        if (path.dirname(dir) === dir) break;
      }
    }
  }

  let ui = opts.ui ?? env("BIFURC_UI") ?? null;
  if (ui === null && tray !== null) {
    const sibling = path.join(path.dirname(tray), UI_BINARY);
    if (existsSync(sibling)) ui = sibling;
  }
  return { tray, ui };
}

export interface DesktopOptions {
  companions: DesktopCompanions;
  /** The engine's data directory, handed to an interface the tray starts so it joins this engine. */
  dataDir: string;
  bus: EngineEventBus;
  /** The user's "keep running in the tray" setting, read when the last interface closes. */
  keepInTray: () => boolean;
  /** Stops the engine the way `engine.shutdown` does. */
  shutdown: (restart: boolean) => void;
  notice: (message: string) => void;
}

export interface Desktop {
  /** Shows the tray icon, if there is a helper to show it. */
  start(): void;
  onSessionStart(identity: SessionIdentity): void;
  onSessionEnd(identity: SessionIdentity): void;
}

export function createDesktop(opts: DesktopOptions): Desktop {
  const { companions, notice } = opts;
  /** Session ids of the connected interfaces. */
  const interfaces = new Set<string>();
  let tray: ChildProcess | null = null;
  let trayReady = false;
  let stopping = false;
  let launchedAt = 0;

  function openInterface(): void {
    if (interfaces.size > 0) {
      opts.bus.emitTyped("ui.activate", {});
      return;
    }
    if (companions.ui === null) {
      notice("the tray asked for the interface, but the engine was not told where it is (BIFURC_UI)");
      return;
    }
    // A double click arrives as two requests, and the interface takes a moment to connect.
    if (Date.now() - launchedAt < LAUNCH_GRACE_MS) return;
    launchedAt = Date.now();
    const child = spawn(companions.ui, [], {
      detached: true,
      stdio: "ignore",
      env: { ...process.env, BIFURC_DATA_DIR: opts.dataDir },
    });
    child.on("error", (err) => {
      launchedAt = 0;
      notice(`could not start the interface at ${companions.ui}: ${err.message}`);
    });
    child.unref();
  }

  function onTrayLine(line: string): void {
    switch (line.trim()) {
      case "ready":
        trayReady = true;
        break;
      case "open":
        openInterface();
        break;
      case "restart":
        opts.shutdown(true);
        break;
      case "quit":
        opts.shutdown(false);
        break;
    }
  }

  function closeTray(): void {
    trayReady = false;
    // The helper takes its icon down and exits when its stdin closes.
    tray?.stdin?.end();
    tray = null;
  }

  opts.bus.onTyped("engine.stopping", () => {
    stopping = true;
    closeTray();
  });

  return {
    start(): void {
      if (companions.tray === null || tray !== null) return;
      const child = spawn(companions.tray, [], { stdio: ["pipe", "pipe", "ignore"], windowsHide: true });
      tray = child;
      child.on("error", (err) => {
        if (tray === child) tray = null;
        trayReady = false;
        notice(`no tray icon: could not start ${companions.tray}: ${err.message}`);
      });
      child.on("exit", () => {
        if (tray !== child) return;
        tray = null;
        trayReady = false;
        if (!stopping) notice("the tray icon is gone — the engine will stop when its last interface closes");
      });
      // A helper that died takes its pipe with it; the write error is the `exit` above, told twice.
      child.stdin?.on("error", () => {});
      if (child.stdout !== null) createInterface({ input: child.stdout }).on("line", onTrayLine);
    },

    onSessionStart(identity: SessionIdentity): void {
      if (identity.clientName === undefined || !INTERFACE_CLIENTS.has(identity.clientName)) return;
      interfaces.add(identity.sessionId);
      launchedAt = 0;
    },

    onSessionEnd(identity: SessionIdentity): void {
      if (!interfaces.delete(identity.sessionId)) return;
      if (stopping || interfaces.size > 0) return;
      if (trayReady && opts.keepInTray()) return;
      notice(
        trayReady
          ? "the last interface closed and the engine is not kept in the tray — stopping"
          : "the last interface closed and there is no tray icon to reach the engine by — stopping",
      );
      opts.shutdown(false);
    },
  };
}
