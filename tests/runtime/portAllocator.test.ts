/**
 * P14 WI 5 — the port allocator, tested against **real sockets**.
 *
 * The unit under test is defined by its interaction with the OS, so every case here binds real ports. A
 * mocked `net` could not fail in the way that matters: the whole reason D16 forbids `checkPortInUse` is that
 * a probe-then-close can be raced, and a mock has no race.
 *
 * The load-bearing cases are therefore:
 *   - `tryBind` reports a held port as unavailable AND **releases** a free one (a probe that leaks the port
 *     would make every allocation fail at the caller's real `listen()`);
 *   - the base path **scans upward** from a taken base (the designed path for the second workspace);
 *   - an override **throws** and names the next free port (D16's fail-loudly rule);
 *   - the proxy and webhook ranges are independent.
 */

import { describe, it, expect, afterEach, vi } from "vitest";
import * as net from "net";
import * as fs from "fs";
import * as path from "path";

import {
  allocatePorts,
  findFreePortFrom,
  nextFreePortAbove,
  proxyBasePort,
  tryBind,
  PortRangeExhaustedError,
  PortUnavailableError,
  WEBHOOK_BASE_PORT,
} from "@bifurc/engine/runtime/portAllocator";

/** Ports this file holds open, released in `afterEach` so a failure cannot leak into the next case. */
const held: net.Server[] = [];

/** Bind and HOLD a port, so it is genuinely unavailable — returns it. */
async function holdPort(port = 0): Promise<number> {
  const srv = new net.Server();
  await new Promise<void>((resolve, reject) => {
    srv.once("error", reject);
    srv.listen(port, "127.0.0.1", () => resolve());
  });
  held.push(srv);
  const addr = srv.address();
  return typeof addr === "object" && addr ? addr.port : port;
}

/** Ask the OS for a free port and release it — for cases that need a *specific* starting number. */
async function freePort(): Promise<number> {
  const srv = new net.Server();
  const port = await new Promise<number>((resolve, reject) => {
    srv.once("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      resolve(typeof addr === "object" && addr ? addr.port : 0);
    });
  });
  await new Promise<void>((resolve) => srv.close(() => resolve()));
  return port;
}

/** Whether a real `listen()` on `port` succeeds. Used only to tell a race from the regression — see below. */
function portIsFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = new net.Server();
    const done = (free: boolean): void => {
      srv.removeAllListeners();
      srv.on("error", () => {});
      srv.close(() => resolve(free));
    };
    srv.once("error", () => done(false));
    srv.once("listening", () => done(true));
    try {
      srv.listen(port, "127.0.0.1");
    } catch {
      done(false);
    }
  });
}

/**
 * Run an allocation from freshly-probed bases, retrying when a **probe's own port was taken** in the window
 * between `freePort()` closing it and the allocator's `tryBind` reaching it.
 *
 * ## Why this is needed, and why it does not weaken the regression it guards
 *
 * `freePort()` is bind-then-close — **the exact TOCTOU shape D16 forbids the allocator from using.** It is
 * acceptable in a *fixture* only while nothing races it, and under a full parallel run something can: the
 * OS may hand the same ephemeral port to another worker's `freePort()`, or to a server another suite is
 * starting. When that happens `allocatePorts` **correctly** scans past the base, and an assertion of the
 * form `expect(out.proxyPort).toBe(base)` fails for a reason that has nothing to do with the base-skip
 * regression it exists to catch.
 *
 * This was observed on 2026-09-21 once P14 WI 7 added `workspaceCommandScope.integration.test.ts`, which
 * binds two more ports per run: the case passed **21/21 isolated** and failed in the full suite.
 *
 * **The two causes are distinguishable, and that is what makes this safe rather than a retry that hides
 * bugs.** The regression leaves the base **free** (the allocator skipped it); a race leaves it **held**
 * (someone else has it). So a retry happens *only* while some missed base is genuinely occupied, and a base
 * that is free-but-skipped is returned unchanged so the caller's assertion still fails. A genuine
 * regression is deterministic, so it survives all five attempts and fails exactly as before.
 */
async function allocateFromUncontendedBases(
  run: (proxyBase: number, webhookBase: number) => Promise<{ proxyPort: number; webhookPort: number }>,
  opts: {
    /**
     * Set when the two ranges are deliberately given the **same** base, so the webhook is *expected* to land
     * elsewhere (`reserved.add(proxyPort)` excludes it). Without this the helper would read that expected
     * difference as a miss and retry until it gave up.
     */
    webhookSharesProxyBase?: boolean;
  } = {},
): Promise<{ out: { proxyPort: number; webhookPort: number }; proxyBase: number; webhookBase: number }> {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    const proxyBase = await freePort();
    const webhookBase = await freePort();
    const out = await run(proxyBase, webhookBase);

    const proxyMissed = out.proxyPort !== proxyBase;
    const webhookMissed = !opts.webhookSharesProxyBase && out.webhookPort !== webhookBase;
    if (!proxyMissed && !webhookMissed) return { out, proxyBase, webhookBase };

    // Retry only while a miss is explained by someone else holding the port. A miss on a base that is still
    // free is the allocator skipping its own base — return and let the assertion report it.
    const lostToRace =
      (proxyMissed && !(await portIsFree(proxyBase))) || (webhookMissed && !(await portIsFree(webhookBase)));
    if (lostToRace) continue;

    return { out, proxyBase, webhookBase };
  }
  throw new Error("could not obtain uncontended base ports in 5 attempts");
}

afterEach(async () => {
  while (held.length) {
    const srv = held.pop()!;
    await new Promise<void>((resolve) => srv.close(() => resolve()));
  }
  vi.unstubAllEnvs();
});

describe("tryBind() — the real bind, not a probe", () => {
  it("reports a free port as available AND actually releases it", async () => {
    const port = await freePort();
    expect(await tryBind(port)).toBe(true);

    // The point: the probe must not hold the port. A caller binds it for real immediately after, and
    // would hit EADDRINUSE against a leaked probe.
    const srv = new net.Server();
    await new Promise<void>((resolve, reject) => {
      srv.once("error", reject);
      srv.listen(port, "127.0.0.1", () => resolve());
    });
    held.push(srv);
    expect((srv.address() as net.AddressInfo).port).toBe(port);
  });

  it("reports a held port as unavailable", async () => {
    const port = await holdPort();
    expect(await tryBind(port)).toBe(false);
  });
});

describe("findFreePortFrom() — the environment-base scan (D16 order #2)", () => {
  it("returns the base itself when it is free", async () => {
    const base = await freePort();
    expect(await findFreePortFrom(base)).toBe(base);
  });

  it("scans contiguously upward when the base is taken", async () => {
    const base = await holdPort();
    const firstFree = await holdPort(base + 1);
    expect(firstFree).toBe(base + 1);

    // base and base+1 are both held, so the scan must land on base+2 — the designed path for the second
    // workspace, not a degraded one.
    expect(await findFreePortFrom(base)).toBe(base + 2);
  });

  it("returns null when the whole scanned range is exhausted", async () => {
    // Ask from the last legal base so the scan has exactly one candidate, and hold it.
    await holdPort(65535);
    // 65535 is the top of the port space, so the scan runs out rather than wrapping to 0.
    expect(await findFreePortFrom(65535)).toBeNull();
  });

  it("skips a reservation a sibling runtime is already using", async () => {
    // Simulates the real multi-workspace case: runtime A took base; runtime B scans from base.
    const base = await holdPort();
    expect(await findFreePortFrom(base)).toBe(base + 1);
  });
});

describe("nextFreePortAbove() — advisory, for the fail-loudly message", () => {
  it("returns the given port when it is free", async () => {
    const port = await freePort();
    expect(await nextFreePortAbove(port)).toBe(port);
  });

  it("walks past held ports", async () => {
    const port = await holdPort();
    await holdPort(port + 1);
    expect(await nextFreePortAbove(port)).toBe(port + 2);
  });
});

describe("allocatePorts() — D16's resolution order", () => {
  it("scans upward from the base for both ranges when there is no override", async () => {
    const base = await freePort();
    // Separated on purpose: two consecutive `freePort()`s come back adjacent, which would make the proxy
    // scan take the webhook's base (see the adjacency note on "keeps the two ranges independent" below).
    const webhookBase = base + 10;
    expect(await tryBind(webhookBase)).toBe(true);

    const out = await allocatePorts({ proxyBase: base, webhookBase });

    expect(out.proxyPort).toBe(base);
    expect(out.webhookPort).toBe(webhookBase);
    // Neither was a preferred port, so the host knows both were chosen by scanning.
    expect(out.proxyPortWasPreferred).toBe(false);
    expect(out.webhookPortWasPreferred).toBe(false);
  });

  it("honours a per-workspace override when it is free", async () => {
    const preferred = await freePort();
    const preferredWebhook = await freePort();

    const out = await allocatePorts({
      proxyBase: await freePort(),
      webhookBase: await freePort(),
      preferredProxyPort: preferred,
      preferredWebhookPort: preferredWebhook,
    });

    expect(out.proxyPort).toBe(preferred);
    expect(out.webhookPort).toBe(preferredWebhook);
    expect(out.proxyPortWasPreferred).toBe(true);
    expect(out.webhookPortWasPreferred).toBe(true);
  });

  /**
   * D16's **stickiness** — the third source, added 2026-09-22 after the acceptance audit found the rule was
   * specified and unimplemented (`plan/14` §8 criteria 6a/6b).
   *
   * Every case here asserts the **provenance flag**, not just the port. That is not belt-and-braces: on a
   * quiet machine the remembered port and the scan can produce the same number, so `expect(out.proxyPort)
   * .toBe(remembered)` can pass while the remembered input is ignored entirely. The flag cannot. (The same
   * reasoning as the `IS_WINDOWS` count in §8 criterion 3: when the claim is about *which mechanism ran*,
   * assert the mechanism.)
   */
  it("honours a remembered port when it is free — and says so via the flag", async () => {
    const remembered = await freePort();
    let base = await freePort();
    for (let i = 0; i < 5 && base === remembered; i += 1) base = await freePort();
    expect(base).not.toBe(remembered);
    expect(await portIsFree(remembered)).toBe(true);

    const out = await allocatePorts({
      proxyBase: base,
      webhookBase: await freePort(),
      rememberedProxyPort: remembered,
    });

    expect(out.proxyPort).toBe(remembered);
    expect(out.proxyPortWasRemembered).toBe(true);
    expect(out.proxyPortWasPreferred).toBe(false);
  });

  it("FALLS THROUGH to the base scan when the remembered port is taken — never throws", async () => {
    const base = await freePort();
    const remembered = await holdPort();
    expect(remembered).not.toBe(base);

    const out = await allocatePorts({
      proxyBase: base,
      webhookBase: await freePort(),
      rememberedProxyPort: remembered,
    });

    // This is the assertion that separates "soft" from "an override": a taken remembered port must land the
    // workspace on the scan's own first choice, not throw `PortUnavailableError` and not skip the workspace.
    expect(out.proxyPort).toBe(base);
    expect(out.proxyPortWasRemembered).toBe(false);
    expect(out.proxyPortWasPreferred).toBe(false);
  });

  it("the override beats the remembered port, and the remembered port beats the base", async () => {
    const base = await freePort();
    const remembered = await freePort();
    const preferred = await freePort();

    const out = await allocatePorts({
      proxyBase: base,
      webhookBase: await freePort(),
      rememberedProxyPort: remembered,
      preferredProxyPort: preferred,
    });

    // D16's source 1 outranks source 2 — a remembered outcome must never shadow a choice the user made.
    expect(out.proxyPort).toBe(preferred);
    expect(out.proxyPortWasPreferred).toBe(true);
    expect(out.proxyPortWasRemembered).toBe(false);
  });

  it("skips a remembered port the host has already reserved", async () => {
    const base = await freePort();
    const remembered = base + 5;
    expect(await portIsFree(remembered), "the fixture needs base + 5 free").toBe(true);

    /**
     * **The contrast is what makes this case falsifiable, and it was missing in the first version.**
     *
     * `expect(out.proxyPort).toBe(base)` alone cannot fail for its own reason: with the remembered path
     * removed altogether the scan takes `base` too, so the case passed for the wrong reason and guarded
     * nothing. Showing that the *same* fixture takes `remembered` when it is unreserved is what isolates the
     * reservation as the cause — and it is the half that breaks if the reserved check is dropped.
     */
    const unreserved = await allocatePorts({
      proxyBase: base,
      webhookBase: await freePort(),
      rememberedProxyPort: remembered,
    });
    expect(unreserved.proxyPort).toBe(remembered);
    expect(unreserved.proxyPortWasRemembered).toBe(true);

    const out = await allocatePorts({
      proxyBase: base,
      webhookBase: await freePort(),
      rememberedProxyPort: remembered,
      // Something the host is about to bind itself — the RPC endpoint, or a sibling already placed.
      reserved: [remembered],
    });

    expect(out.proxyPort).toBe(base);
    expect(out.proxyPortWasRemembered).toBe(false);
  });

  it("applies the same soft rule to the remembered webhook port", async () => {
    const remembered = await freePort();
    let webhookBase = await freePort();
    for (let i = 0; i < 5 && webhookBase === remembered; i += 1) webhookBase = await freePort();
    expect(webhookBase).not.toBe(remembered);

    const out = await allocatePorts({
      proxyBase: await freePort(),
      webhookBase,
      rememberedWebhookPort: remembered,
    });

    expect(out.webhookPort).toBe(remembered);
    expect(out.webhookPortWasRemembered).toBe(true);
    expect(out.webhookPortWasPreferred).toBe(false);
  });

  it("THROWS on a taken proxy override, naming the next free port — never silently increments", async () => {
    const taken = await holdPort();
    const nextFree = await holdPort(taken + 1);
    expect(nextFree).toBe(taken + 1);

    let thrown: unknown;
    try {
      await allocatePorts({ preferredProxyPort: taken, webhookBase: await freePort() });
    } catch (e) {
      thrown = e;
    }

    expect(thrown).toBeInstanceOf(PortUnavailableError);
    const err = thrown as PortUnavailableError;
    expect(err.requested).toBe(taken);
    // D16: the message must say what to do, so it names a concrete free port.
    expect(err.nextFree).toBe(taken + 2);
    expect(err.message).toContain(String(taken));
    expect(err.message).toContain(String(taken + 2));
    expect(err.message).toMatch(/in use/i);
  });

  it("THROWS on a taken webhook override too", async () => {
    const taken = await holdPort();
    await expect(
      allocatePorts({ webhookBase: await freePort(), preferredWebhookPort: taken }),
    ).rejects.toBeInstanceOf(PortUnavailableError);
  });

  /**
   * A trap worth stating, because it made this case fail for the wrong reason.
   *
   * `holdPort()` and `freePort()` both bind an **ephemeral** port, and the OS hands out adjacent numbers: a
   * `freePort()` taken right after a `holdPort()` is reliably `base + 1` — measured six times, `gap = 1`
   * every time. So the naive version of this case (`base = holdPort()`, `webhookBase = freePort()`) puts the
   * webhook base exactly where the proxy scan is about to land, and the webhook is then legitimately pushed
   * to `base + 2`. The assertion fails while the allocator is behaving correctly.
   *
   * The ranges must therefore be separated **deliberately**, by asking for the webhook base from a different
   * span rather than from the one the OS is currently handing out.
   */
  it("keeps the two ranges independent — a taken proxy port does not consume a webhook port", async () => {
    const base = await holdPort();
    // Deliberately far from the proxy range: an ephemeral port here would be `base + 1`.
    const webhookBase = base + 10;
    expect(await tryBind(webhookBase)).toBe(true);

    const out = await allocatePorts({ proxyBase: base, webhookBase });

    expect(out.proxyPort).toBe(base + 1);
    // The webhook is untouched by the proxy's scan.
    expect(out.webhookPort).toBe(webhookBase);
  });

  it("throws PortRangeExhaustedError when nothing is free near the top of the port space", async () => {
    // Base 65535 with 65535 held leaves no room for a single increment.
    await holdPort(65535);
    await expect(allocatePorts({ proxyBase: 65535, webhookBase: await freePort() })).rejects.toBeInstanceOf(
      PortRangeExhaustedError,
    );
  });

  /**
   * The regression a `reserved` set introduced, and the reason this case exists.
   *
   * The first version of the reserved set added **both bases** to it, so each range's scan skipped its own
   * base and every base-path allocation landed on `base + 1`. Nothing failed: the port was free, the runtime
   * came up, and the host's integration test *observed* `81, 82` and was written up as "D16's base plus
   * contiguous scan" when the truth was "base skipped, then contiguous". A wrong-but-working port is
   * invisible to any test that only asserts the runtime is serving — so the assertion here is on the
   * **identity of the first port**, in both ranges, with a reservation present.
   *
   * The second attempt reserved only the *other* range's base, which fixed the proxy half and broke the
   * webhook half for exactly the same reason. Reserving **neither** is the correct rule; this case is what
   * distinguishes them, so it must assert both ports and not merely the proxy's.
   */
  it("still takes each base itself when a reservation is supplied (regression)", async () => {
    const rpcPort = await freePort();

    // `allocateFromUncontendedBases` rather than raw `freePort()`: the bases are probed bind-then-close, so
    // under a parallel run one can be taken before the allocator reaches it, and the allocator would then
    // *correctly* scan — failing this assertion for the wrong reason. See the helper's header.
    const { out, proxyBase, webhookBase } = await allocateFromUncontendedBases((p, w) =>
      allocatePorts({ proxyBase: p, webhookBase: w, reserved: [rpcPort] }),
    );

    expect(out.proxyPort).toBe(proxyBase);
    expect(out.webhookPort).toBe(webhookBase);
  });

  it("skips a reserved port the scan would otherwise have taken", async () => {
    const base = await holdPort();
    const rpcPort = base + 1;

    // base is held and base+1 is reserved, so the scan must land on base+2 — the reservation is honoured
    // even though nothing is bound there.
    const out = await allocatePorts({ proxyBase: base, webhookBase: await freePort(), reserved: [rpcPort] });

    expect(out.proxyPort).toBe(base + 2);
  });

  it("keeps the two ranges apart even when the bases overlap (env var can make them)", async () => {
    // The hazard the original "bases reserve each other" line was reaching for. With one base used for both
    // ranges the webhook allocation must not be handed the proxy's port; the guard is `reserved.add(proxyPort)`
    // after the proxy is chosen, not the base.
    //
    // `webhookSharesProxyBase` because the webhook *should* differ here — the helper would otherwise read the
    // intended behaviour as a miss.
    const { out, proxyBase } = await allocateFromUncontendedBases(
      (shared) => allocatePorts({ proxyBase: shared, webhookBase: shared }),
      { webhookSharesProxyBase: true },
    );

    expect(out.proxyPort).toBe(proxyBase);
    expect(out.webhookPort).not.toBe(out.proxyPort);
    expect(out.webhookPort).toBe(proxyBase + 1);
  });
});

describe("proxyBasePort() — the per-environment base (D16)", () => {
  it("defaults to 80 on win32 and 8080 elsewhere", () => {
    vi.stubEnv("BIFURC_PROXY_BASE_PORT", "");
    const expected = process.platform === "win32" ? 80 : 8080;
    expect(proxyBasePort()).toBe(expected);
  });

  it("is moved by BIFURC_PROXY_BASE_PORT, and the increment still works after it", async () => {
    const base = await freePort();
    vi.stubEnv("BIFURC_PROXY_BASE_PORT", String(base));
    expect(proxyBasePort()).toBe(base);

    // The variable moves the BASE, not one workspace's port — so a sibling still gets base+1.
    await holdPort(base);
    expect(await findFreePortFrom(proxyBasePort())).toBe(base + 1);
  });

  it("fails loudly on a malformed override rather than silently using the default", () => {
    vi.stubEnv("BIFURC_PROXY_BASE_PORT", "not-a-port");
    expect(() => proxyBasePort()).toThrow(/BIFURC_PROXY_BASE_PORT/);

    vi.stubEnv("BIFURC_PROXY_BASE_PORT", "70000");
    expect(() => proxyBasePort()).toThrow(/BIFURC_PROXY_BASE_PORT/);
  });

  it("uses the webhook base 9101 on every platform", () => {
    expect(WEBHOOK_BASE_PORT).toBe(9101);
  });
});

/**
 * P14 §8 criterion 5 — "`EACCES` advances rather than killing the runtime".
 *
 * ## Which "override" the criterion means, and why that matters
 *
 * D16's table has **two** sources and the word "override" names both: the per-workspace override (a port the
 * user chose) and an override of the *environment base* (`BIFURC_PROXY_BASE_PORT`). The criterion's own
 * parenthetical — "the default path never attempts a privileged port, so this guards the override case
 * only" — is only true of the second: the defaults are `80` on win32 and `8080` elsewhere, and `8080` needs
 * no privilege, so the case that reaches a privileged port is an operator who set the base to one. That
 * reading is also the only one consistent with D16, because a *per-workspace* override that cannot be bound
 * must **throw**, not advance. Both halves are asserted below, and the pair is the content of the rule.
 */
describe("a bind failure that is not EADDRINUSE (P14 §8 criterion 5)", () => {
  afterEach(() => vi.restoreAllMocks());

  /**
   * Make a bind on `port` fail the way a privileged port fails on a platform that forbids it.
   *
   * `EACCES` cannot be produced for real on win32 — an unprivileged process may bind below 1024 there — so
   * the failure is injected. That is the honest instrument rather than a compromise: the claim under test is
   * the allocator's *response* to a failed bind, which is platform-independent, and the OS's reason for
   * refusing is not what is being asserted.
   *
   * The error is emitted **asynchronously, on `nextTick`**, not thrown from `listen()`. That is how a real
   * bind failure arrives, and it exercises `tryBind`'s `server.once("error", …)` path rather than its
   * `try/catch` — a synchronous throw would test the wrong half of the function.
   */
  function failBindWithEacces(port: number): void {
    const realListen = net.Server.prototype.listen;
    vi.spyOn(net.Server.prototype, "listen").mockImplementation(function (
      this: net.Server,
      ...args: unknown[]
    ) {
      if (args[0] === port) {
        process.nextTick(() => {
          const err = new Error(`listen EACCES: permission denied 127.0.0.1:${port}`) as NodeJS.ErrnoException;
          err.code = "EACCES";
          this.emit("error", err);
        });
        return this;
      }
      return (realListen as unknown as (...a: unknown[]) => net.Server).apply(this, args);
    } as unknown as typeof net.Server.prototype.listen);
  }

  it("advances past a BASE port whose bind fails, rather than killing the runtime", async () => {
    const base = await freePort();
    failBindWithEacces(base);

    const got = await findFreePortFrom(base);

    // The runtime comes up: an unbindable base is not fatal, it is what the scan exists for.
    expect(got).not.toBeNull();
    // It did not hand back the port whose bind failed...
    expect(got).not.toBe(base);
    // ...it moved upward, and what it settled on is genuinely bindable.
    expect(got!).toBeGreaterThan(base);
    expect(await portIsFree(got!)).toBe(true);
  });

  it("still fails LOUDLY when it is a per-workspace OVERRIDE that cannot be bound", async () => {
    const blocked = await freePort();
    failBindWithEacces(blocked);

    // The same failure, the other rule. An allocator that advanced here would be silently moving a port
    // the user explicitly chose, which is the one thing D16 says must never happen.
    await expect(allocatePorts({ preferredProxyPort: blocked })).rejects.toBeInstanceOf(PortUnavailableError);
  });
});

/**
 * P14 §8 criterion 3 — "the platform branch is one constant".
 *
 * The audit found the first half met (`:358` asserts the two values) and the invariant half unguarded. Both
 * halves are here, and the second is a **structural** assertion rather than a behavioural one, deliberately:
 * "nothing else in the allocation path branches on platform" is a statement about how *many* platform reads
 * exist, and no behavioural test can express a count — it would have to enumerate every output of the module
 * and show each is platform-invariant, which is unbounded and would silently stop covering new outputs.
 * Asserting the count covers the whole file including code that has no test yet.
 */
describe("the platform branch is one constant (P14 §8 criterion 3)", () => {
  const realPlatform = process.platform;

  /**
   * Load a **fresh** module instance with `process.platform` faked.
   *
   * `IS_WINDOWS` is computed at module load (`export const IS_WINDOWS = process.platform === "win32"`), so
   * stubbing the platform after import would change nothing and both cases below would pass vacuously —
   * they would be asserting the *real* platform twice.
   */
  async function loadUnder(platform: NodeJS.Platform) {
    vi.resetModules();
    Object.defineProperty(process, "platform", { value: platform, configurable: true });
    return import("@bifurc/engine/runtime/portAllocator");
  }

  afterEach(() => {
    Object.defineProperty(process, "platform", { value: realPlatform, configurable: true });
    vi.unstubAllEnvs();
    vi.resetModules();
  });

  it("yields base 80 under a simulated win32", async () => {
    vi.stubEnv("BIFURC_PROXY_BASE_PORT", "");
    const mod = await loadUnder("win32");
    expect(mod.IS_WINDOWS).toBe(true);
    expect(mod.proxyBasePort()).toBe(80);
  });

  it("yields base 8080 under a simulated linux — the half this machine never exercises", async () => {
    vi.stubEnv("BIFURC_PROXY_BASE_PORT", "");
    const mod = await loadUnder("linux");
    expect(mod.IS_WINDOWS).toBe(false);
    expect(mod.proxyBasePort()).toBe(8080);
  });

  it("treats darwin as 'not win32' rather than as a third case", async () => {
    vi.stubEnv("BIFURC_PROXY_BASE_PORT", "");
    const mod = await loadUnder("darwin");
    expect(mod.proxyBasePort()).toBe(8080);
  });

  it("reads `process.platform` exactly once in the whole module, and uses the constant once", () => {
    const src = fs.readFileSync(
      path.resolve(process.cwd(), "packages/engine/src/runtime/portAllocator.ts"),
      "utf-8",
    );
    // The one read *is* the `IS_WINDOWS` definition — not merely a read that happens to be there.
    expect(src.match(/process\.platform/g) ?? []).toHaveLength(1);
    expect(src).toMatch(/export const IS_WINDOWS = process\.platform === "win32";/);

    /**
     * **Counted over the module's *code*, not over its text** — and that distinction is the difference
     * between this instrument working and this instrument lying.
     *
     * The first version counted `src` directly, and it broke on a **documentation** edit: a docblock added on
     * 2026-09-22 that merely *names* `IS_WINDOWS` (while explaining the soft remembered-port path) took the
     * count to 3 and failed the case. A comment is not a branch on platform, so counting raw text had made
     * the criterion sensitive to prose — and the tempting "fix" would have been to delete a useful
     * explanation to satisfy a test. **A count is only the right instrument if it counts the right thing.**
     *
     * Comment-only lines are dropped, which is sufficient here: every docblock line in this module begins with
     * `*`, `//` or `/*` once trimmed. A trailing comment on a *code* line is deliberately left in place, so a
     * real second use cannot hide behind one.
     */
    const code = src
      .split("\n")
      .filter((line) => {
        const t = line.trim();
        return !(t.startsWith("*") || t.startsWith("//") || t.startsWith("/*"));
      })
      .join("\n");

    // Declaration plus one use. A third occurrence **in code** is a second branch on platform, which is the
    // thing the criterion forbids — and it is the *use* count, not the read count, that catches a branch
    // written against the already-computed constant.
    expect(code.match(/\bIS_WINDOWS\b/g) ?? []).toHaveLength(2);
  });

  // The webhook base has no platform split at all (`WEBHOOK_BASE_PORT === 9101`, asserted above), so there
  // is nothing to simulate for it — the criterion's "assert it for both bases" is satisfied by the base
  // cases here plus that constant.
});

/**
 * P14 §8 criterion 7 — the *named* reservations, and the webhook interaction.
 *
 * The audit's complaint was that reservations were covered only in general (`:184`, `:314`, `:328`) and that
 * the specific pair the criterion names was asserted nowhere. The numbers below are real constants
 * elsewhere in the tree — `store/appSettings.ts:85` (`companionPort: 9271`) and `store/config.ts:293`
 * (`grpcMockServerPort: 9102`) — and they are **written out rather than imported on purpose**: the claim is
 * that the allocator is *told* them by the host and never derives them, so importing them would let this
 * keep passing after a host stopped passing them.
 *
 * Each case holds the ports *below* the reserved one, so the scan genuinely wants it. Without that the
 * reservation would never be consulted and the case would pass whether or not it worked.
 */
describe("the named reservations — companion 9271 and the gRPC placeholder 9102 (P14 §8 criterion 7)", () => {
  const COMPANION = 9271;
  const GRPC_PLACEHOLDER = 9102;

  /**
   * These are fixed ports in Bifurc's own ranges, so a concurrent integration run could hold one. The
   * preconditions are asserted rather than assumed so that such a collision reports *which* port was
   * unavailable instead of surfacing as a raw `EADDRINUSE` from `holdPort` — a failure that reads like an
   * allocator defect when it is an environment collision.
   */
  it("skips the companion port when the scan reaches it", async () => {
    expect(await portIsFree(9270), "9270 must be free for this case to be decisive").toBe(true);
    expect(await portIsFree(9272), "9272 must be free for this case to be decisive").toBe(true);
    await holdPort(9270);
    expect(await findFreePortFrom(9270, "proxy", new Set([COMPANION, GRPC_PLACEHOLDER]))).toBe(9272);
  });

  it("skips the gRPC placeholder when the proxy scan reaches it", async () => {
    expect(await portIsFree(9100), "9100 must be free for this case to be decisive").toBe(true);
    expect(await portIsFree(9103), "9103 must be free for this case to be decisive").toBe(true);
    await holdPort(9100);
    await holdPort(9101);
    // 9100 and 9101 are held, so the next candidate is the reserved 9102 — the only port the scan can be
    // pushed onto, which is what makes this discriminating rather than incidental.
    expect(await findFreePortFrom(9100, "proxy", new Set([COMPANION, GRPC_PLACEHOLDER]))).toBe(9103);
  });

  it("the second workspace's webhook skips 9102 and lands on 9103 — the case the criterion names", async () => {
    expect(await portIsFree(9101), "9101 must be free for this case to be decisive").toBe(true);
    expect(await portIsFree(9103), "9103 must be free for this case to be decisive").toBe(true);
    await holdPort(9101);
    expect(await findFreePortFrom(9101, "webhook", new Set([COMPANION, GRPC_PLACEHOLDER]))).toBe(9103);
  });
});
