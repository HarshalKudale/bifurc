/**
 * Port finding, after `plan/17`: **every port is engine-level, none is per workspace**.
 *
 * ## What this module is now, and what it stopped being
 *
 * Until 2026-09-25 this file was the **per-workspace allocator** to D16's rule — a base port, a contiguous
 * scan upward, a per-workspace override that failed loudly, and a remembered resolved port so a workspace
 * that landed on 81 kept 81. `plan/17` §1 replaced that model: a workspace is a **project**, so switching
 * workspaces changes the **context** (which rules, mocks and webhook endpoints are in force) while the ports
 * and the infrastructure stay put.
 *
 * The consequence for this module is that it loses its subject. `allocatePorts()`,
 * `PortUnavailableError`, `PortRangeExhaustedError` and `AllocatedPorts` all existed to answer *"which port
 * does workspace N get?"* — a question the new model does not ask. They are **deleted, not deprecated**:
 * leaving them behind would keep a second, contradictory answer to a question the host now answers once.
 *
 * What survives is finding, and there is exactly **one** kind — the companion port, which a client has to be
 * told:
 *
 * | Finder | For | Why this shape |
 * |---|---|---|
 * | `findFreePortFrom` | the **companion** | one port, no neighbour to reserve |
 *
 * Three ports are deliberately **not** found here, because they are bound rather than discovered — a port
 * that has to be *addressed* cannot move without breaking whoever addresses it:
 *
 * - the **edge** (reverse proxy) — every `*.localhost` mapping points at it;
 * - the **forward proxy** — the browser's proxy setting names it, so a taken port **fails loudly** instead
 *   of the app silently incrementing;
 * - the **webhook** — a *feature*, off by default, whose port the user owns.
 *
 * ## The defaults do not live here
 *
 * The numbers are in `store/appSettings.ts`, next to the settings they are the defaults *of*, so a number
 * cannot drift from the field it fills. This module owns only the *finding*, which is a runtime concern and
 * takes its base as a parameter.
 *
 * ## The one thing that must not be done differently: bind for real
 *
 * `applications/portUtils.ts#checkPortInUse` **binds a port and immediately closes it** to answer "is this
 * free?", then the caller's real `listen()` happens later. That is a TOCTOU race: between the probe and the
 * bind, another process can take the port, and the finder would have promised a port it cannot deliver.
 * `plan/14` §4 records the rule as **"must be driven by the real `listen()`, never `checkPortInUse`"**, and
 * this module honours it: `tryBind()` attempts an actual `net.Server.listen()`, and a port is only free if
 * that *succeeded*. `checkPortInUse` is still the right tool for **reporting** a busy port to a user (it can
 * name the owning PID, which a bind cannot), and `killProcessOnPort` is unaffected.
 */

import * as net from "net";

/**
 * How far a scan will look. 1000 is far beyond any plausible need and bounds the loop so a pathological
 * environment cannot hang startup.
 */
export const SCAN_LIMIT = 1000;

/**
 * Attempt a **real** bind on `port`, then release it.
 *
 * This is the primitive the whole module is built on, and the thing `checkPortInUse` must not replace: the
 * answer comes from an actual `listen()`, not from a probe that closes before the caller binds. Port `0` is
 * special-cased by the OS to mean "any free port", so it always succeeds — callers that need a specific port
 * never pass it.
 */
export function tryBind(port: number, host = "127.0.0.1"): Promise<boolean> {
  return new Promise((resolve) => {
    const server = new net.Server();
    let settled = false;

    const release = (free: boolean): void => {
      if (settled) return;
      settled = true;
      // A late `error` after teardown would otherwise be an unhandled throw.
      server.removeAllListeners();
      server.on("error", () => {});
      // The handle must be released before the caller binds it for real, or the caller's `listen()` would
      // hit `EADDRINUSE` against *this* probe. `close()`'s callback is the point at which the port is free.
      server.close(() => resolve(free));
    };

    server.once("error", () => release(false));
    server.once("listening", () => release(true));
    try {
      server.listen(port, host);
    } catch {
      release(false);
    }
  });
}

/**
 * The first bindable port at or above `base`, or `null` if the whole range is taken.
 *
 * **The single-port finder** — for a listener with no neighbour it has to reserve. Its one caller is the
 * companion port (`plan/17` §1: *"Electron has no GUI port, so it checks only the companion's `n` — requiring
 * `n + 1` to be free there would refuse a port for a neighbour it is never going to use"*).
 *
 * `reserved` ports are skipped **without binding them**. Skipping rather than binding matters for a port the
 * caller is about to take itself: binding one to test it would be a probe of a port already spoken for, and
 * a `reserved` set that had to win a race against its own scan would be no guard at all.
 */
export async function findFreePortFrom(
  base: number,
  reserved: ReadonlySet<number> = new Set<number>(),
): Promise<number | null> {
  for (let offset = 0; offset < SCAN_LIMIT; offset += 1) {
    const candidate = base + offset;
    if (candidate > 65535) return null;
    if (reserved.has(candidate)) continue;
    // eslint-disable-next-line no-await-in-loop -- the scan is inherently sequential: each candidate must be
    // bound before the next is tried, or two starters scanning at once could both see the same port free.
    if (await tryBind(candidate)) return candidate;
  }
  return null;
}

/**
 * The next free port at or above `from`, for **error messages only**.
 *
 * Its one caller is the forward proxy's bind failure. `plan/17` §1 requires that port to fail loudly rather
 * than silently increment *"so the user enters a working one instead of the app silently incrementing"* —
 * and a message that names a working port is what makes that instruction followable. It is deliberately
 * advisory: nothing uses its answer to pick a port.
 */
export async function nextFreePortAbove(from: number): Promise<number | null> {
  for (let candidate = from; candidate <= 65535 && candidate < from + SCAN_LIMIT; candidate += 1) {
    // eslint-disable-next-line no-await-in-loop -- sequential for the same reason as `findFreePortFrom`.
    if (await tryBind(candidate)) return candidate;
  }
  return null;
}
