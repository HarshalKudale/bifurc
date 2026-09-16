/**
 * P2 work item 3 — idempotent engine shutdown.
 *
 * `src/main.ts` used to run the teardown sequence inline inside `app.on("before-quit")`. A
 * decoupled engine cannot own its own lifecycle that way: the Electron shell, a supervisor
 * restarting a crashed engine, and a `SIGTERM` handler in a container may **all** call shutdown,
 * in any order, and possibly concurrently. Running the sequence twice must be a no-op rather than
 * a second round of `taskkill`s and duplicate "stopped" logs.
 *
 * Each individual stop function is already null-guarded and safe to call when its subsystem is not
 * running (`stopServer`, `stopCompanionServer`, `stopAllAutoSync`); what was missing is a guard on
 * the *sequence* — hence the memoised promise below.
 *
 * This module is intentionally Electron-free. Keep it that way: it is on the `packages/engine`
 * side of the seam.
 */
import { processSpawner } from "@/applications/processSpawner";
import { stopAllAutoSync } from "@bifurc/engine/sync/autoSync";
import { stopCompanionServer } from "@/companion/companionServer";
import { stopServer } from "@bifurc/engine/proxy/server";

/**
 * The in-flight or completed shutdown. Memoised so the second and later callers await the same
 * work instead of starting a second teardown pass.
 */
let shutdownPromise: Promise<void> | null = null;

/** True once `shutdownEngine()` has been called at least once. */
export function isShuttingDown(): boolean {
  return shutdownPromise !== null;
}

/**
 * Tear the engine down. Safe to call any number of times, from any number of places.
 *
 * Order matters: child processes and pollers are stopped before the servers that would otherwise
 * keep accepting traffic and spawning more work.
 */
export function shutdownEngine(): Promise<void> {
  if (shutdownPromise) return shutdownPromise;

  shutdownPromise = Promise.resolve().then(() => {
    processSpawner.stopAll();
    stopAllAutoSync();
    stopCompanionServer();
    stopServer();
  });

  return shutdownPromise;
}
