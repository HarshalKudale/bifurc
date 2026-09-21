/**
 * P14 work item 5 — the port allocator, to D16's rule.
 *
 * ## The rule, from `plan/14` §3 / D16
 *
 * The environment supplies a **base port** and a workspace's proxy scans **contiguously upward** from it
 * until a bind succeeds. Resolution order:
 *
 * | # | Source | If the port is taken |
 * |---|---|---|
 * | 1 | **Per-workspace override** (the host-local registry, or the settings UI) | **Fail loudly**, naming the next free port. Never silently move a port the user chose. |
 * | 2 | **Environment base** — `80` on win32, `8080` elsewhere, overridable via `BIFURC_PROXY_BASE_PORT` | Scan upward: `base + 1`, `base + 2`, … |
 *
 * The webhook port follows the same shape with base `9101` on every platform (already above 1024, so no
 * platform split is needed).
 *
 * ## The one thing that must not be done differently: bind for real
 *
 * `applications/portUtils.ts#checkPortInUse` **binds a port and immediately closes it** to answer "is this
 * free?", then the caller's real `listen()` happens later. That is a TOCTOU race: between the probe and the
 * bind, another process — or another of this host's own runtimes — can take the port, and the allocator
 * would have promised a port it cannot deliver. D16 records the rule as **"must be driven by the real
 * `listen()`, never `checkPortInUse`"**, and this module honours it: `tryBind()` attempts an actual
 * `net.Server.listen()`, and the port is only considered free if that *succeeded*. `checkPortInUse` is still
 * the right tool for **reporting** a busy port to a user (it can name the owning PID, which a bind cannot),
 * and `killProcessOnPort` is unaffected.
 *
 * ## Scanning, and what "contiguous" means
 *
 * The scan is `base, base+1, base+2, …` up to `SCAN_LIMIT`. It deliberately does **not** consult the
 * registry first and jump to a remembered port: D16's whole point is that the increment is the designed path
 * for every workspace after the first, so a host that restarts should land its runtimes on the same ports as
 * long as the same ports are free — which the scan does naturally, in order, without needing to remember.
 *
 * ## What this module does not do
 *
 * It does not read the registry, write it, or know what a workspace is. It takes a *preference* and returns
 * a *port*. The host owns the registry (`plan/14` §4: the resolved port is recorded in the host-local
 * `runtime.json`, **never** in `workspace.json`, which is git-tracked and therefore syncs and collides).
 */

import * as net from "net";

/** Windows may bind privileged ports without elevation; everywhere else, 80 needs root or a capability. */
export const IS_WINDOWS = process.platform === "win32";

/**
 * The per-environment base port (D16). `BIFURC_PROXY_BASE_PORT` moves the **base**, not one workspace's
 * port, so the increment keeps working after it — which is what a container wants: pin the published port
 * without giving up the fallback.
 */
export function proxyBasePort(): number {
  const raw = process.env.BIFURC_PROXY_BASE_PORT;
  if (raw !== undefined && raw !== "") {
    const parsed = Number.parseInt(raw, 10);
    // Fail loudly on a malformed override rather than silently using the default: a container that set the
    // variable meant it, and a silent fallback would publish the wrong port.
    if (!Number.isInteger(parsed) || parsed < 0 || parsed > 65535) {
      throw new Error(
        `BIFURC_PROXY_BASE_PORT must be a port number (0-65535), got ${JSON.stringify(raw)}`,
      );
    }
    return parsed;
  }
  return IS_WINDOWS ? 80 : 8080;
}

/** The webhook base. `9101` on every platform — above 1024, so no environment split is needed. */
export const WEBHOOK_BASE_PORT = 9101;

/**
 * How far the scan will look. 1000 is far beyond any plausible workspace count and bounds the loop so a
 * pathological environment cannot hang startup.
 */
export const SCAN_LIMIT = 1000;

/** Raised when a **per-workspace override** cannot be honoured. D16 requires this to fail loudly. */
export class PortUnavailableError extends Error {
  constructor(
    readonly requested: number,
    readonly nextFree: number | null,
    readonly kind: "proxy" | "webhook",
  ) {
    super(
      nextFree !== null
        ? `Port ${requested} (the configured ${kind} port for this workspace) is already in use. ` +
            `Port ${nextFree} is free — either free ${requested} or change this workspace's ${kind} port.`
        : `Port ${requested} (the configured ${kind} port for this workspace) is already in use, and no ` +
            `free port was found in the ${SCAN_LIMIT} ports above it. Free ${requested} or change this ` +
            `workspace's ${kind} port.`,
    );
    this.name = "PortUnavailableError";
  }
}

/** Raised when the scan exhausted its range with nothing free. */
export class PortRangeExhaustedError extends Error {
  constructor(
    readonly base: number,
    readonly kind: "proxy" | "webhook",
  ) {
    super(
      `No free ${kind} port in ${base}-${base + SCAN_LIMIT - 1}. ` +
        `Set BIFURC_PROXY_BASE_PORT to move the base, or free a port in that range.`,
    );
    this.name = "PortRangeExhaustedError";
  }
}

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
 * Scan contiguously upward from `base` for the first bindable port.
 *
 * Returns the port, or `null` if the whole range is taken. Used for the **environment base** path, where
 * landing on `base + 1` is the designed outcome rather than a degraded one — so callers must not warn about
 * it.
 *
 * `reserved` ports are skipped **without binding them**. Skipping rather than binding matters for the ports
 * this host is about to take itself: binding one to test it would be a probe of a port already spoken for,
 * and a `reserved` set that had to win a race against its own scan would be no guard at all.
 */
export async function findFreePortFrom(
  base: number,
  kind: "proxy" | "webhook" = "proxy",
  reserved: ReadonlySet<number> = new Set<number>(),
): Promise<number | null> {
  void kind;
  for (let offset = 0; offset < SCAN_LIMIT; offset += 1) {
    const candidate = base + offset;
    if (candidate > 65535) return null;
    if (reserved.has(candidate)) continue;
    // eslint-disable-next-line no-await-in-loop -- the scan is inherently sequential: each candidate must be
    // bound before the next is tried, or two runtimes scanning at once could both see the same port free.
    if (await tryBind(candidate)) return candidate;
  }
  return null;
}

/**
 * The next free port at or above `from`, for **error messages only**. D16 requires the "fail loudly" message
 * to *name* the next free port, so a user whose override was taken is told what to do; it is deliberately
 * advisory and not used to pick a port.
 */
export async function nextFreePortAbove(from: number): Promise<number | null> {
  for (let candidate = from; candidate <= 65535 && candidate < from + SCAN_LIMIT; candidate += 1) {
    // eslint-disable-next-line no-await-in-loop -- sequential for the same reason as `findFreePortFrom`.
    if (await tryBind(candidate)) return candidate;
  }
  return null;
}

/** The ports a runtime will use, with the provenance of each — so a host can log *why* it landed there. */
export interface AllocatedPorts {
  proxyPort: number;
  webhookPort: number;
  /** True when `preferredProxyPort` was honoured as-is (no increment, no override). */
  proxyPortWasPreferred: boolean;
  /** True when `preferredWebhookPort` was honoured as-is. */
  webhookPortWasPreferred: boolean;
}

export interface AllocateOptions {
  /** A per-workspace override. If given and taken, this **throws** — D16's fail-loudly rule. */
  preferredProxyPort?: number | null;
  /** A per-workspace webhook override, same rule. */
  preferredWebhookPort?: number | null;
  /** The environment base for the proxy. Defaults to `proxyBasePort()`. */
  proxyBase?: number;
  /** The environment base for the webhook. Defaults to `WEBHOOK_BASE_PORT`. */
  webhookBase?: number;
  /**
   * Ports the scan must not take even though they are free.
   *
   * **Not an optimisation — a correctness requirement.** The RPC endpoint binds a port of its own *after*
   * this allocator has run, and so does a workspace the operator has configured but not started. Without a
   * reserved set, the scan would happily hand workspace 2 the port the RPC server is about to want, and the
   * failure would surface as an RPC endpoint that cannot bind — long after the allocator reported success.
   * The host computes this set; the allocator only honours it, because only the host knows what it is about
   * to bind.
   */
  reserved?: Iterable<number>;
}

/**
 * Allocate the proxy and webhook ports for one workspace.
 *
 * **The override path throws rather than moving**, which is the whole difference between the two sources in
 * D16's table: an override is a choice the user made, so an allocator that quietly gave them a different
 * port would be lying about what it did. The base path scans, because the increment is the designed
 * behaviour.
 *
 * The two allocations are independent — a taken proxy port must not consume the webhook range, and the
 * callers of this function are themselves serialised by the host (one call per runtime, in order), because
 * two *concurrent* calls could otherwise both see the same port free and race to bind it.
 */
export async function allocatePorts(opts: AllocateOptions = {}): Promise<AllocatedPorts> {
  const proxyBase = opts.proxyBase ?? proxyBasePort();
  const webhookBase = opts.webhookBase ?? WEBHOOK_BASE_PORT;
  // Materialised once, and the port this call allocates is added to it as it is chosen — so the webhook
  // allocation cannot be handed the port the proxy allocation just took when the two ranges overlap (they do
  // not today, but a `BIFURC_PROXY_BASE_PORT` of 9101 makes them overlap, and an allocator that relied on the
  // bases staying apart would be one env var away from a self-collision).
  const reserved = new Set<number>(opts.reserved ?? []);
  // **Neither base is a reservation, and that is subtle enough to have been got wrong twice.**
  //
  // A base is where *its own* range starts, so putting it in `reserved` makes `findFreePortFrom(base)` skip
  // `base` and land on `base + 1`: a silent off-by-one for the first workspace on a free machine — the
  // common case — and one no integration test catches, because a runtime on `base + 1` still comes up and
  // still serves. The first version of this file reserved *both* bases, which broke the base path in both
  // ranges; the fix is not to switch which one is reserved but to reserve **neither**.
  //
  // What the original comment was reaching for is real but belongs elsewhere: with a `BIFURC_PROXY_BASE_PORT`
  // of 9101 the two ranges overlap, and the proxy scan could take the port the webhook range is about to
  // want. That is handled by `reserved.add(proxyPort)` below, which excludes the *chosen* proxy port from the
  // webhook scan — a stronger guard than excluding the webhook's base, because it covers every port the
  // proxy actually took rather than one hypothetical collision point.

  // ── proxy ──────────────────────────────────────────────────────────────────
  let proxyPort: number;
  let proxyPortWasPreferred = false;

  if (opts.preferredProxyPort != null) {
    const preferred = opts.preferredProxyPort;
    // An override is checked against `reserved` as well as the bind. What `reserved` holds here is only the
    // **host's** reservations (the RPC endpoint, a not-yet-started workspace) — a user who typed the port the
    // host is about to bind has made a mistake the bind alone cannot detect, and silently honouring it would
    // trade their explicit choice for a broken host.
    if (reserved.has(preferred) || !(await tryBind(preferred))) {
      throw new PortUnavailableError(preferred, await nextFreePortAbove(preferred + 1), "proxy");
    }
    proxyPort = preferred;
    proxyPortWasPreferred = true;
  } else {
    // `proxyBase` is deliberately **not** in `reserved` — it is this range's own starting point, and the
    // first workspace on a free machine must be able to take it. Only the host's reservations are excluded.
    const found = await findFreePortFrom(proxyBase, "proxy", reserved);
    if (found === null) throw new PortRangeExhaustedError(proxyBase, "proxy");
    proxyPort = found;
  }
  reserved.add(proxyPort);

  // ── webhook ────────────────────────────────────────────────────────────────
  let webhookPort: number;
  let webhookPortWasPreferred = false;

  if (opts.preferredWebhookPort != null) {
    const preferred = opts.preferredWebhookPort;
    if (reserved.has(preferred) || !(await tryBind(preferred))) {
      throw new PortUnavailableError(preferred, await nextFreePortAbove(preferred + 1), "webhook");
    }
    webhookPort = preferred;
    webhookPortWasPreferred = true;
  } else {
    // Same rule as the proxy half: `webhookBase` is this range's own start, never a reservation.
    const found = await findFreePortFrom(webhookBase, "webhook", reserved);
    if (found === null) throw new PortRangeExhaustedError(webhookBase, "webhook");
    webhookPort = found;
  }

  return { proxyPort, webhookPort, proxyPortWasPreferred, webhookPortWasPreferred };
}
