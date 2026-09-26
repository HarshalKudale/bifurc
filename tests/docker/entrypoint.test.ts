/**
 * `plan/17` work item 5 — the image entrypoint's port resolution.
 *
 * These are the pure decisions, so they run in the **node** unit project rather than against sockets. The
 * cases that matter are the precedence (flag → environment → default) and the **rejections**, because a
 * malformed port that silently became the default would start a host on a port nobody asked for while the
 * GUI, the `--gui-target` and the healthcheck all agreed with each other and disagreed with the operator.
 */
import { describe, it, expect } from "vitest";
import {
  resolveCompanionPort,
  defaultCompanionPort,
  PORT_FILE,
} from "../../scripts/docker-entrypoint.mjs";

/** Passed explicitly so these cases do not depend on the engine's build output — see the last describe. */
const FALLBACK = 4747;

describe("resolveCompanionPort — precedence", () => {
  it("reads --ws-port", () => {
    expect(resolveCompanionPort(["--ws-port", "5000"], {}, FALLBACK)).toBe(5000);
  });

  it("reads --ws-port=<n>", () => {
    expect(resolveCompanionPort(["--ws-port=5001"], {}, FALLBACK)).toBe(5001);
  });

  it("finds the flag among other arguments", () => {
    // The other arguments are flags this CLI **does** still accept. They used to be `--ws-host` and
    // `--token-file`, which the parser now refuses by name (2026-09-25) — so a case built on them would be
    // testing this scanner against a command line that can no longer reach it.
    const argv = ["--data-dir", "/data", "--ws-port", "5002", "--gui-target", "5753"];
    expect(resolveCompanionPort(argv, {}, FALLBACK)).toBe(5002);
  });

  it("reads BIFURC_COMPANION_PORT when the flag is absent", () => {
    expect(resolveCompanionPort([], { BIFURC_COMPANION_PORT: "6000" }, FALLBACK)).toBe(6000);
  });

  it("lets the flag beat the environment — hostArgs.ts's own precedence", () => {
    expect(
      resolveCompanionPort(["--ws-port", "5000"], { BIFURC_COMPANION_PORT: "6000" }, FALLBACK)
    ).toBe(5000);
  });

  it("treats a blank environment value as unset", () => {
    // `BIFURC_COMPANION_PORT=` in a compose file is how an operator clears it; reading that as a port
    // would be reading it as 0.
    expect(resolveCompanionPort([], { BIFURC_COMPANION_PORT: "   " }, FALLBACK)).toBe(FALLBACK);
  });

  it("falls back when neither is present", () => {
    expect(resolveCompanionPort([], {}, 9999)).toBe(9999);
  });
});

describe("resolveCompanionPort — what it refuses", () => {
  it("rejects a port whose GUI half would not exist", () => {
    // 65534 is the last value with a `+ 1`; 65535 is not a bound that could be honoured.
    expect(resolveCompanionPort(["--ws-port", "65534"], {}, FALLBACK)).toBe(65534);
    expect(() => resolveCompanionPort(["--ws-port", "65535"], {}, FALLBACK)).toThrow(/65534/);
  });

  it("rejects a non-numeric, out-of-range or dangling value, naming its source", () => {
    expect(() => resolveCompanionPort(["--ws-port", "abc"], {}, FALLBACK)).toThrow(/--ws-port/);
    expect(() => resolveCompanionPort(["--ws-port", "0"], {}, FALLBACK)).toThrow(/--ws-port/);
    // A dangling flag returns `""`, which must not be read as "not given" and silently defaulted.
    expect(() => resolveCompanionPort(["--ws-port"], {}, FALLBACK)).toThrow(/--ws-port/);
    expect(() => resolveCompanionPort(["--ws-port", "1.5"], {}, FALLBACK)).toThrow(/--ws-port/);
    expect(() =>
      resolveCompanionPort([], { BIFURC_COMPANION_PORT: "nope" }, FALLBACK)
    ).toThrow(/BIFURC_COMPANION_PORT/);
  });
});

describe("the default is the engine's, not a second literal", () => {
  it("reads DEFAULT_COMPANION_PORT out of the engine, through its CJS build", () => {
    /**
     * `plan/17` §4 makes `store/appSettings.ts` *"the one home for the numbers"*, and this case is what keeps
     * the entrypoint honest about it. The **CJS** build is required rather than imported because the ESM one
     * cannot be deep-imported — extensionless relative imports (`TRAPS.md`).
     *
     * **A failure here reading `ERR_MODULE_NOT_FOUND` means `packages/engine/dist` is not built**, not that
     * the values diverged. `npm ci` builds it via `prepare`; `npm run build:packages` does it directly.
     */
    expect(defaultCompanionPort()).toBe(4747);
  });

  it("publishes the resolved port at the path the healthcheck reads", () => {
    // The literal is the contract between the two scripts, so it is pinned rather than derived: changing it
    // in one file and not the other is the failure this assertion exists to catch.
    expect(PORT_FILE).toBe("/tmp/bifurc/companion-port");
  });
});
