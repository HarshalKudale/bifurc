/**
 * P14 work item 6 — the host CLI's argument parser.
 *
 * ## Why the parser is tested and the executable is not
 *
 * `hostCli.ts` **executes on import** — that is what makes it a deployable artifact with no `main` guard
 * (see its header: this package emits both esm and cjs, whose entry checks differ, and `import.meta` is a
 * syntax error in the cjs leg). So a test cannot import it without starting a host. The parsing, the
 * validation and the usage text therefore live in `hostArgs.ts`, and this file drives them directly.
 *
 * The executable's remaining logic is argv → `WorkspaceHostOptions` plus one `readFileSync`, which is the
 * part worth *not* testing indirectly through a subprocess: the assertions below are the ones that would
 * otherwise have to be made by spawning `node dist/runtime/hostCli.js` and parsing its stderr.
 *
 * ## What these cases are actually protecting
 *
 * Two failures are worth more than the rest, and both are silent in production:
 *
 * 1. **An unrecognised flag must be fatal.** A typo'd `--data-dirr` that was skipped leaves the host on the
 *    *default* data root — which presents as "my workspaces vanished", long after the command line looked
 *    right. Same rule D16 applies to `BIFURC_PROXY_BASE_PORT`.
 * 2. **Absent `--workspace` must be `undefined`, never `[]`.** `WorkspaceHostOptions.workspaceIds` filters
 *    the settings list, so an empty array means "serve nothing" and an absent one means "serve everything".
 *    Those are opposite outcomes from inputs that look identical at the call site.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import { HOST_USAGE, parseHostArgs } from "../../src/runtime/hostArgs";

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("parseHostArgs", () => {
  it("reads --data-dir in the space-separated form", () => {
    const res = parseHostArgs(["--data-dir", "/data"]);
    expect(res).toEqual({ kind: "args", args: { dataDir: "/data", rpcSocket: undefined, tokenFile: undefined, workspaceIds: undefined } });
  });

  it("reads --data-dir in the = form, which is what a path with spaces invites", () => {
    // Worth supporting rather than rejecting: refusing it produces "unrecognised flag
    // `--data-dir=/my data`", which reads like the flag is wrong rather than the quoting.
    const res = parseHostArgs(["--data-dir=/my data"]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") expect(res.args.dataDir).toBe("/my data");
  });

  it("takes the data root from BIFURC_DATA_DIR when the flag is absent", () => {
    vi.stubEnv("BIFURC_DATA_DIR", "/from-env");
    const res = parseHostArgs([]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") expect(res.args.dataDir).toBe("/from-env");
  });

  it("prefers the flag over the environment", () => {
    vi.stubEnv("BIFURC_DATA_DIR", "/from-env");
    const res = parseHostArgs(["--data-dir", "/from-flag"]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") expect(res.args.dataDir).toBe("/from-flag");
  });

  it("fails when there is no data root at all, and says so", () => {
    vi.stubEnv("BIFURC_DATA_DIR", "");
    const res = parseHostArgs([]);
    expect(res.kind).toBe("error");
    // The message must name the missing thing AND carry the usage text — an operator reading a container
    // log has only this line to work from.
    if (res.kind === "error") {
      expect(res.message).toMatch(/--data-dir is required/);
      expect(res.message).toContain("Usage:");
    }
  });

  it("treats --help and -h as a request, not a failure", () => {
    // Distinct from `error` on purpose: the executable prints this to **stdout** and exits **0**. Folding
    // it into the error branch would make `--help` exit 1, which breaks every wrapper that checks the code.
    expect(parseHostArgs(["--help"]).kind).toBe("help");
    expect(parseHostArgs(["-h"]).kind).toBe("help");
  });

  it("fails on an unrecognised flag rather than skipping it", () => {
    // The typo case. Silently ignoring `--data-dirr` leaves the host on the default data root.
    const res = parseHostArgs(["--data-dir", "/data", "--data-dirr", "/other"]);
    expect(res.kind).toBe("error");
    if (res.kind === "error") expect(res.message).toMatch(/unrecognised flag "--data-dirr"/);
  });

  it("fails on a bare positional argument", () => {
    const res = parseHostArgs(["--data-dir", "/data", "stray"]);
    expect(res.kind).toBe("error");
    if (res.kind === "error") expect(res.message).toMatch(/unexpected argument "stray"/);
  });

  it("fails when a flag is missing its value", () => {
    const res = parseHostArgs(["--data-dir"]);
    expect(res.kind).toBe("error");
    if (res.kind === "error") expect(res.message).toMatch(/--data-dir needs a value/);
  });

  it("does not mistake a following flag for a value", () => {
    // `--data-dir --help` must not create a directory literally named "--help" and then serve the default
    // root — which is what "take argv[i+1] unconditionally" would do.
    const res = parseHostArgs(["--data-dir", "--help"]);
    expect(res.kind).toBe("error");
    if (res.kind === "error") expect(res.message).toMatch(/--data-dir needs a value/);
  });

  it("fails on an empty value", () => {
    const res = parseHostArgs(["--data-dir="]);
    expect(res.kind).toBe("error");
    if (res.kind === "error") expect(res.message).toMatch(/--data-dir needs a value/);
  });

  it("collects --workspace in the order given", () => {
    const res = parseHostArgs(["--data-dir", "/data", "--workspace", "b", "--workspace", "a"]);
    expect(res.kind).toBe("args");
    // Order matters: the host starts runtimes in this sequence, so the ports a workspace lands on depend
    // on it. Sorting or de-duplicating here would change which workspace gets the base port.
    if (res.kind === "args") expect(res.args.workspaceIds).toEqual(["b", "a"]);
  });

  it("leaves workspaceIds UNDEFINED when none was given — not an empty array", () => {
    // The load-bearing distinction: `workspaceIds` filters the settings list, so `[]` means "serve none"
    // and `undefined` means "serve every workspace in app.json".
    const res = parseHostArgs(["--data-dir", "/data"]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") expect(res.args.workspaceIds).toBeUndefined();
  });

  it("passes --rpc-socket and --token-file through", () => {
    const res = parseHostArgs([
      "--data-dir", "/data",
      "--rpc-socket", "/run/bifurc.sock",
      "--token-file", "/run/token",
    ]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") {
      expect(res.args.rpcSocket).toBe("/run/bifurc.sock");
      expect(res.args.tokenFile).toBe("/run/token");
    }
  });

  it("accepts the flags in any order", () => {
    // A container's CMD is hand-written and the order is arbitrary; nothing here is positional.
    const res = parseHostArgs(["--token-file", "/run/token", "--data-dir", "/data", "--rpc-socket", "/run/s.sock"]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") {
      expect(res.args.dataDir).toBe("/data");
      expect(res.args.rpcSocket).toBe("/run/s.sock");
      expect(res.args.tokenFile).toBe("/run/token");
    }
  });

  it("documents every flag it accepts, and every env var it reads", () => {
    // A usage text that has drifted from the parser is worse than none: it is the operator's only
    // documentation, and `--help` is what they reach for when the container will not start.
    for (const flag of ["--data-dir", "--rpc-socket", "--token-file", "--workspace", "--help"]) {
      expect(HOST_USAGE).toContain(flag);
    }
    for (const env of ["BIFURC_ENGINE_TOKEN", "BIFURC_PROXY_BASE_PORT", "BIFURC_DATA_DIR"]) {
      expect(HOST_USAGE).toContain(env);
    }
  });
});
