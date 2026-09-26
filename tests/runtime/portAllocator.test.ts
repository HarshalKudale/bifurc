/**
 * `portAllocator.ts`, after `plan/17` §5 work item 2 minimized it.
 *
 * ## What this file used to test, and why most of it is gone
 *
 * Until 2026-09-25 this file was 869 lines and its subject was D16's **per-workspace allocator**:
 * `allocatePorts()`'s three-source resolution order (override → remembered → base scan), the fail-loudly
 * `PortUnavailableError`, `PortRangeExhaustedError`, the provenance flags that existed because *"the outcome
 * cannot distinguish the remembered path from the scan"*, and `proxyBasePort()`'s per-platform base.
 *
 * **Every one of those tested a function that no longer exists**, because the question it answered — *"which
 * port does workspace N get?"* — is not a question `plan/17` asks. They are deleted rather than adapted: a
 * test adapted to a function that is gone would be a test of nothing, and the project's own rule is that an
 * assertion has to be able to fail.
 *
 * ## What is left, and the two shapes it has to cover
 *
 * `plan/17` §1's port table has exactly two shapes of listener that need *finding*:
 *
 * | Finder | For |
 * |---|---|
 * | `findFreePortFrom` | a single port — Electron's companion, which has no GUI neighbour |
 * | `findPortPair` | the companion **and** the web GUI, which must be adjacent |
 *
 * The three ports that are **bound rather than found** — the edge, the forward proxy and the webhook — have
 * no finder to test. What they do have is a *failure* rule (fail loudly, do not move), and that is asserted
 * where it lives: `workspaceHost.integration.test.ts` and `portRestartStability.integration.test.ts`.
 *
 * ## The one hazard this file inherits, unchanged
 *
 * `holdBaseWithFreeSuccessors()` is kept verbatim from the previous version, and its header is the record of
 * why: a case that asserts *"the scan lands on `base + N`"* is asserting the finder **only if `base + N` was
 * free when the case started**. `holdPort()` takes a port from the OS's ephemeral range, so its successors
 * are whatever the OS happened to leave free — and under a full parallel run another worker takes them. Four
 * separate arrivals through three suites in two days are recorded in that header; the guard belongs on every
 * successor-asserting case, not only where a failure has already appeared.
 */
import { describe, it, expect, afterEach } from "vitest";
import * as net from "net";

import {
  findFreePortFrom,
  findPortPair,
  nextFreePortAbove,
  tryBind,
  webGuiPortFor,
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

/** Whether a real `listen()` on `port` succeeds. Used only to tell a race from a regression — see below. */
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
 * Hold an ephemeral port whose next `successors` ports are **all free**, so a case that asserts *"the scan
 * lands on `base + 2`"* is asserting the finder rather than the machine's ephemeral-port lottery.
 *
 * ## Why every successor-asserting case needs this
 *
 * `holdPort()` takes a port from the OS's ephemeral range. `base + 1`, `base + 2`, … are therefore whatever
 * the OS happens to have free at that instant — and under a full parallel run they are exactly the ports
 * another worker's `freePort()` or a server another suite is starting will take. When that happens the finder
 * **correctly** scans one port further, and the assertion fails for a reason that has nothing to do with the
 * property it exists to catch.
 *
 * **Measured, 2026-09-24:** a full run put `expected 49529 to be 49528`, `expected 50235 to be 50234` and a
 * `nextFreePortAbove` miss in three cases at once, all `35/35` isolated. The cause was
 * `tests/integration/edgeSplit.integration.test.ts` — `plan/15` WI 3's verification step, which binds a facade
 * edge and two harness listeners — landing ephemeral ports on those bases' successors.
 *
 * **And again the same day, from the same file's next work item:** adding one case to `edgeSplit` that starts a
 * GUI server plus a second proxy turned *"THROWS on a taken proxy override"* red with
 * `expected 57751 to be 57750`, and a sweep of this file found **two more cases** still asserting on an
 * unguarded successor. **Four separate arrivals through three suites in two days.** The generalisation is not
 * *"this file is flaky"* — it is that **the guard belongs on every successor-asserting case, and adding it
 * only where a failure has already appeared guarantees a fifth arrival.**
 *
 * ## The residual window, stated rather than implied
 *
 * This is a **check-then-use**, so it is not airtight: a successor can be taken in the microseconds between
 * the check and the call that relies on it. It shrinks that window from "whatever the OS handed out" to "a
 * scheduling accident", and the alternative — retrying the assertion itself — would be a retry that can hide
 * a real regression. **The distinction that matters is preserved:** a miss on a successor that is *still
 * free* is the finder skipping a port, and that still fails.
 */
async function holdBaseWithFreeSuccessors(successors: number): Promise<number> {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    const base = await holdPort();
    const wanted = Array.from({ length: successors }, (_, i) => base + 1 + i);
    const free = await Promise.all(wanted.map((p) => portIsFree(p)));
    if (free.every(Boolean)) return base;

    // Release the candidate and take another ephemeral port. `holdPort()` appends to `held`, so a retry that
    // did not pop would leak a listener per attempt.
    const srv = held.pop();
    if (srv) await new Promise<void>((resolve) => srv.close(() => resolve()));
  }
  throw new Error(`could not obtain a base with ${successors} free successors in 5 attempts`);
}

/**
 * The mirror of `holdBaseWithFreeSuccessors()` for the **pair** finder, which needs its `base` to be *free*
 * rather than held: a port that is free together with `successors` free ports above it.
 *
 * It cannot use `holdPort()` for the base, because holding it is exactly what `findPortPair` is supposed to
 * walk past — and that would turn *"the pair starts at the base"* into an untestable assertion. So it takes
 * the same check-then-use bargain, on the same terms and with the same residual window.
 */
async function freeBaseWithFreeSuccessors(successors: number): Promise<number> {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    const base = await freePort();
    const wanted = Array.from({ length: successors + 1 }, (_, i) => base + i);
    const free = await Promise.all(wanted.map((p) => portIsFree(p)));
    if (free.every(Boolean)) return base;
  }
  throw new Error(`could not obtain a free base with ${successors} free successors in 5 attempts`);
}

afterEach(async () => {
  while (held.length) {
    const srv = held.pop()!;
    await new Promise<void>((resolve) => srv.close(() => resolve()));
  }
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

describe("findFreePortFrom() — the single-port finder (Electron's companion)", () => {
  it("returns the base itself when it is free", async () => {
    const base = await freeBaseWithFreeSuccessors(0);
    expect(await findFreePortFrom(base)).toBe(base);
  });

  it("scans contiguously upward when the base is taken", async () => {
    // `2` successors: `base + 1` is held explicitly below and `base + 2` is where the scan must land, so both
    // have to be free before the case starts. See `holdBaseWithFreeSuccessors` for why that is not automatic.
    const base = await holdBaseWithFreeSuccessors(2);
    const firstFree = await holdPort(base + 1);
    expect(firstFree).toBe(base + 1);

    // base and base+1 are both held, so the scan must land on base+2.
    expect(await findFreePortFrom(base)).toBe(base + 2);
  });

  it("returns null when the whole scanned range is exhausted", async () => {
    // Ask from the last legal base so the scan has exactly one candidate, and hold it.
    await holdPort(65535);
    // 65535 is the top of the port space, so the scan runs out rather than wrapping to 0.
    expect(await findFreePortFrom(65535)).toBeNull();
  });

  it("skips a reservation without binding it", async () => {
    // The host reserves the ports it has already bound (the edge, the forward proxy) so the pair search
    // cannot hand back one of them. Skipping rather than probing matters: probing a port the caller is
    // about to take would be a race against itself.
    const base = await holdBaseWithFreeSuccessors(2);
    expect(await findFreePortFrom(base, new Set<number>([base + 1]))).toBe(base + 2);
  });
});

describe("findPortPair() — the companion and the web GUI, which must be adjacent", () => {
  it("returns the base pair when both halves are free", async () => {
    const base = await freeBaseWithFreeSuccessors(1);
    expect(await findPortPair(base)).toEqual({ companionPort: base, webGuiPort: base + 1 });
  });

  /**
   * **The load-bearing case, and the reason this finder exists at all.**
   *
   * `plan/17` §1: *"`n` free with `n + 1` taken is a **failed** start, not a usable one."* A finder that
   * checked one port at a time would return `{ companionPort: base }` here and leave the GUI with nowhere to
   * live — and the GUI is what the edge's global mapping points at, so the failure would surface as a dead
   * base domain rather than as a port error.
   */
  it("skips a candidate whose successor is taken, rather than returning half a pair", async () => {
    const base = await freeBaseWithFreeSuccessors(3);
    await holdPort(base + 1);

    // `base` is free, but `base + 1` is not — so the pair is `base + 2` / `base + 3`, not `base` / taken.
    expect(await findPortPair(base)).toEqual({ companionPort: base + 2, webGuiPort: base + 3 });
  });

  it("skips a pair whose first half is reserved", async () => {
    const base = await freeBaseWithFreeSuccessors(2);
    expect(await findPortPair(base, new Set<number>([base]))).toEqual({
      companionPort: base + 1,
      webGuiPort: base + 2,
    });
  });

  it("skips a pair whose second half is reserved", async () => {
    // The `reserved` set is honoured for **both** halves, so a pair cannot straddle a port the caller is
    // about to bind. Reserving only the first half would be the narrower guard and the wrong one: the GUI's
    // port is as much the caller's as the companion's.
    const base = await freeBaseWithFreeSuccessors(3);
    expect(await findPortPair(base, new Set<number>([base + 1]))).toEqual({
      companionPort: base + 2,
      webGuiPort: base + 3,
    });
  });

  it("returns null when the range cannot hold a pair", async () => {
    // Hold the last two ports, so no adjacent pair exists at the top of the space.
    await holdPort(65534);
    await holdPort(65535);
    expect(await findPortPair(65534)).toBeNull();
  });
});

describe("webGuiPortFor() — derived from the companion, never a second setting", () => {
  it("is the companion's port plus one", () => {
    expect(webGuiPortFor(4747)).toBe(4748);
  });

  it("is a pure function of its argument, so the pair cannot be configured apart", () => {
    // The property that matters is not the arithmetic but the *absence of a second input*: the GUI port is
    // served **with** the companion's port handed to it, so an independently settable GUI port would be a
    // way to configure the pair into a state where the GUI looks for the companion somewhere it is not.
    // Asserted as a function of two different bases so it cannot pass by returning a constant.
    expect(webGuiPortFor(1000)).toBe(1001);
    expect(webGuiPortFor(65533)).toBe(65534);
  });
});

describe("nextFreePortAbove() — advisory, for the fail-loudly message", () => {
  it("returns the given port when it is free", async () => {
    const port = await freeBaseWithFreeSuccessors(0);
    expect(await nextFreePortAbove(port)).toBe(port);
  });

  it("walks past held ports", async () => {
    // Two successors: `port + 1` is held explicitly and `port + 2` is the expected landing.
    const port = await holdBaseWithFreeSuccessors(2);
    await holdPort(port + 1);
    expect(await nextFreePortAbove(port)).toBe(port + 2);
  });
});
