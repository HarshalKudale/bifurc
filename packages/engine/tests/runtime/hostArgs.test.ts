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
 * Three failures are worth more than the rest, and each is silent in production:
 *
 * 1. **An unrecognised flag must be fatal.** A typo'd `--data-dirr` that was skipped leaves the host on the
 *    *default* data root — which presents as "my workspaces vanished", long after the command line looked
 *    right. Same rule D16 applies to `BIFURC_PROXY_BASE_PORT`.
 * 2. **Absent `--workspace` must be `undefined`, never `[]`.** `WorkspaceHostOptions.workspaceIds` filters
 *    the settings list, so an empty array means "serve nothing" and an absent one means "serve everything".
 *    Those are opposite outcomes from inputs that look identical at the call site.
 * 3. **A flag removed on 2026-09-25 must be refused *by name and with its reason*, not as a typo.** The nine
 *    bind and token flags were deleted when the engine stopped being remotely deployable. A compose file
 *    written before that date is the *expected* input, and `unrecognised flag` would send its author hunting
 *    for a spelling mistake that is not there. See the dedicated block below.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import { HOST_USAGE, parseHostArgs } from "../../src/runtime/hostArgs";
import { platformDefaultDataDir } from "../../src/workspace/paths";

afterEach(() => {
  vi.unstubAllEnvs();
});

/**
 * The flags `HOST_USAGE` documents, and therefore the flags the parser must accept.
 *
 * Hoisted so the two cases that use it cannot disagree: one asserts the usage text **contains** each of
 * these, the other extracts the flags out of the usage text and asserts the set is **exactly** this.
 * Together they pin the prose in both directions — a flag added to the parser and forgotten in the text
 * fails the first, and a flag left in the text after being removed fails the second.
 *
 * It mirrors `hostArgs.ts`'s `VALUE_FLAGS` (not exported, so it cannot be imported here) plus `--help`.
 * `-h` is the same flag spelled short and is not listed separately.
 *
 * **It used to be sixteen entries.** Ten of them named bind hosts, TLS paths and the token file, and all
 * ten were removed on 2026-09-25 — see the block further down, which asserts each is refused by name.
 */
const DOCUMENTED_FLAGS = [
  "--data-dir",
  "--rpc-socket",
  "--ws-port",
  "--workspace",
  "--edge-port",
  "--transport",
  "--help",
];

describe("parseHostArgs", () => {
  it("reads --data-dir in the space-separated form", () => {
    const res = parseHostArgs(["--data-dir", "/data"]);
    // The whole shape, so a field that is parsed but never surfaced (or surfaced but never parsed) shows up
    // here. It used to be longer: `wsHost` and `tokenFile` went with the bind opt-ins and the engine token
    // on 2026-09-25, and the web-GUI target went when the web UI host was decommissioned — leaving `dataDir`
    // plus the four optional ones below.
    expect(res).toEqual({
      kind: "args",
      args: {
        dataDir: "/data",
        transport: "socket",
        rpcSocket: undefined,
        wsPort: undefined,
        workspaceIds: undefined,
        edgePort: undefined,
      },
    });
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

  it("defaults the data root to the desktop app's, so a bare `bifurc-engine` serves what the app shows", () => {
    vi.stubEnv("BIFURC_DATA_DIR", "");
    const res = parseHostArgs([]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") expect(res.args.dataDir).toBe(platformDefaultDataDir());
  });

  it("selects the transport with --transport, defaulting to socket", () => {
    const pick = (argv: string[]) => {
      const res = parseHostArgs(["--data-dir", "/d", ...argv]);
      return res.kind === "args" ? res.args.transport : res.kind === "error" ? res.message : res.kind;
    };
    expect(pick([])).toBe("socket");
    expect(pick(["--transport", "stdio"])).toBe("stdio");
    expect(pick(["--transport", "ws"])).toBe("ws");
    expect(pick(["--ws-port", "0"])).toBe("ws");
    expect(pick(["--transport", "pipe"])).toMatch(/--transport must be socket, ws or stdio/);
    expect(pick(["--transport", "stdio", "--ws-port", "0"])).toMatch(/takes no --rpc-socket or --ws-port/);
    expect(pick(["--transport", "ws", "--rpc-socket", "/s"])).toMatch(/cannot be combined with --rpc-socket/);
  });

  it("REFUSES --domain, which was removed rather than left inert", () => {
    // It used to be the second required flag (`plan/15` work item 9, added 2026-09-23). It is gone because the
    // engine is no longer deployable in the cloud, so every deployment is on the machine the client is on and
    // the name it answers as is `localhost` — and a flag that can only hold one value is the *accepted and
    // ignored* shape this parser refuses everywhere else.
    //
    // **Refusing it by name is the point, not a side effect.** A stale `docker run … --domain localhost` (the
    // image's own CMD, before this change) would otherwise start a host that silently ignored the argument, and
    // an operator who believes they named their engine is an operator who will not look here when their
    // `*.localhost` mappings behave unexpectedly. Fatal, with the usage text, on the file's own rule for
    // unrecognised flags.
    //
    // It is **not** in `REMOVED_FLAGS`, unlike the nine below — it was removed a day earlier and its message
    // is the generic one. `hostArgs.ts`'s header argues why.
    const res = parseHostArgs(["--data-dir", "/data", "--domain", "localhost"]);
    expect(res.kind).toBe("error");
    if (res.kind === "error") {
      expect(res.message).toMatch(/unrecognised flag "--domain"/);
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

  it("passes --rpc-socket through", () => {
    // It used to be tested alongside `--token-file`, which was the second half of "serve RPC over a socket
    // and guard it". The token is gone; the socket path is not, and it is still the default transport.
    const res = parseHostArgs(["--data-dir", "/data", "--rpc-socket", "/run/bifurc.sock"]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") expect(res.args.rpcSocket).toBe("/run/bifurc.sock");
  });

  it("passes --ws-port through — this flag alone now selects WebSocket mode", () => {
    // `--ws-host` used to sit beside it and named a *bind address*, which no longer exists. With one flag
    // left there is no longer a command line that names a WebSocket port without naming a host, or the
    // reverse — which is why the coherence check that used to guard that pair is gone rather than kept.
    const res = parseHostArgs(["--data-dir", "/data", "--ws-port", "9272"]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") expect(res.args.wsPort).toBe(9272);
  });

  it("accepts --ws-port 0 — an ephemeral port, which is what a test harness asks for", () => {
    const res = parseHostArgs(["--data-dir", "/data", "--ws-port", "0"]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") expect(res.args.wsPort).toBe(0);
  });

  it("fails on an invalid --ws-port", () => {
    const res = parseHostArgs(["--data-dir", "/data", "--ws-port", "not-a-port"]);
    expect(res.kind).toBe("error");
    if (res.kind === "error") expect(res.message).toMatch(/--ws-port must be an integer from 0 to 65535/);
  });

  it("fails when socket and ws flags are mixed", () => {
    const res = parseHostArgs(["--data-dir", "/data", "--rpc-socket", "/run/b.sock", "--ws-port", "9272"]);
    expect(res.kind).toBe("error");
    // The message names the two flags that are actually in conflict. It used to say `--ws-host/--ws-port`,
    // because `--ws-host` was part of "WebSocket mode"; it is one flag now, and the message says so.
    if (res.kind === "error") expect(res.message).toMatch(/--rpc-socket cannot be combined with --ws-port/);
  });

  it("accepts the flags in any order", () => {
    // A container's CMD is hand-written and the order is arbitrary; nothing here is positional.
    const res = parseHostArgs(["--edge-port", "8080", "--data-dir", "/data", "--rpc-socket", "/run/s.sock"]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") {
      expect(res.args.dataDir).toBe("/data");
      expect(res.args.rpcSocket).toBe("/run/s.sock");
      expect(res.args.edgePort).toBe(8080);
    }
  });

  it("documents every flag it accepts, and every env var it reads", () => {
    // A usage text that has drifted from the parser is worse than none: it is the operator's only
    // documentation, and `--help` is what they reach for when the container will not start.
    //
    // **This list is hand-maintained, and it drifted once.** Work item 3 added the three edge flags and left
    // them out of it, so `--edge-port`, `--edge-bind-host` and `--edge-allow-non-loopback` went unasserted
    // *by this test* for as long as they existed — found 2026-09-23 by grepping the suite for the flags
    // rather than for the layer that changed. Two of those three have since been removed outright, which is
    // what the "removed on 2026-09-25" block below asserts; the surviving list is `DOCUMENTED_FLAGS`.
    for (const flag of DOCUMENTED_FLAGS) {
      expect(HOST_USAGE).toContain(flag);
    }
    for (const env of ["BIFURC_PROXY_BASE_PORT", "BIFURC_DATA_DIR"]) {
      expect(HOST_USAGE).toContain(env);
    }
  });

  it("recognises every flag its own usage text names — the half that cannot drift", () => {
    // The reverse direction of the case above, and the one that needs no list: take the flags out of `HOST_USAGE`
    // itself and check the parser knows each. A documented flag the parser rejects is a `--help` that lies, which
    // an operator meets at the worst possible moment — mid-deployment, with no way to tell a typo from a version.
    //
    // `"x"` is passed as every value, which is why the assertion is "not an error that means the flag was
    // *unknown*" rather than "parses". `--ws-port x` and `--edge-port x` fail their integer check, each
    // naming itself — which proves the flag was **recognised**, the entire question being asked here.
    const documented = [...new Set(HOST_USAGE.match(/--[a-z][a-z-]*/g) ?? [])].filter((f) => f !== "--help");
    // Without this, a regex that silently matched nothing would make the loop below assert nothing at all — a
    // green test that cannot fail, which is worse than no test. Pinning the *set* rather than a count is what
    // also makes this case catch a flag left behind in the prose after being removed: `DOCUMENTED_FLAGS` is the
    // list the case above just checked against the parser, so a disagreement between the two is a real one.
    expect(documented.slice().sort()).toEqual(DOCUMENTED_FLAGS.filter((f) => f !== "--help").slice().sort());
    for (const flag of documented) {
      const res = parseHostArgs(["--data-dir", "/data", flag, "x"]);
      const unrecognised = res.kind === "error" && /unrecognised flag/.test(res.message);
      expect(unrecognised, `${flag} is documented in HOST_USAGE but the parser rejects it`).toBe(false);
      // A removed flag re-added to the prose is refused *by name* rather than as unrecognised, so the check
      // above would wave it through. It needs its own assertion.
      const removed = res.kind === "error" && /was removed on 2026-09-25/.test(res.message);
      expect(removed, `${flag} is documented in HOST_USAGE but the parser refuses it as removed`).toBe(false);
    }
  });
});

/**
 * The nine flags that were removed on **2026-09-25**, and the diagnosis each one now produces.
 *
 * ## Why the parser names them instead of saying "unrecognised"
 *
 * These are not typos. They are flags an operator **copied from a compose file that was correct when it was
 * written** — this repository's own `DOCKER.md` and `docker-compose.yml`, until the container image was
 * decommissioned on 2026-09-26 — and the generic message sends them hunting for a spelling mistake that is
 * not there. `hostArgs.ts`'s `REMOVED_FLAGS` is the only place left that can still recover what these meant,
 * so it does, with the reason.
 *
 * ## What replaced them
 *
 * Nothing, and that is the design. Each one existed either to publish a listener or to guard a boundary, and
 * the engine is not deployable remotely: every listener binds loopback unconditionally, so there is no
 * address to name and no boundary for a token to guard. The *policy* is deliberately not restated here — it
 * belongs to the modules that bind, and `transport/ws.test.ts`, `proxy/server.test.ts` and
 * `tests/proxy/edge.test.ts` are where it is pinned. A copy of the rule here would be a copy that can drift.
 *
 * ## Why the assertion is on the *message* and not on `kind`
 *
 * `kind === "error"` is already guaranteed by the unrecognised-flag branch, so asserting it alone would pass
 * even if `REMOVED_FLAGS` were emptied — a green test that cannot fail. The *reason* is the half that would
 * silently regress, so each case names its own flag **and** a fragment of the reason specific to it.
 *
 * ## The nine-versus-ten, since the count looks off by one
 *
 * `--domain` was removed a day earlier (2026-09-24) and is **not** in this table: it falls through to
 * `unrecognised flag`, which is what the case in the `parseHostArgs` block above asserts. The difference is
 * not arbitrary — `hostArgs.ts`'s header argues it as a deployment decision rather than a consequence of the
 * loopback bind, and it is the last case where the old message is still the honest one.
 */
describe("parseHostArgs — the flags removed on 2026-09-25", () => {
  /** Each removed flag paired with a fragment of *its own* reason, so a copy-pasted table cannot pass. */
  const removed = [
    ["--ws-host", /bind host is not a choice/],
    ["--ws-allow-non-loopback", /not deployable remotely/],
    ["--ws-tls-cert", /TLS was only required/],
    ["--ws-tls-key", /TLS was only required/],
    ["--token-file", /engine token is gone/],
    ["--proxy-bind-host", /forward proxy binds loopback/],
    ["--proxy-allow-non-loopback", /not deployable remotely/],
    ["--edge-bind-host", /edge binds loopback/],
    ["--edge-allow-non-loopback", /not deployable remotely/],
  ] as const;

  it("REFUSES each one BY NAME, with its reason, and carries the usage text", () => {
    for (const [flag, reason] of removed) {
      const res = parseHostArgs(["--data-dir", "/data", flag]);
      expect(res.kind, `${flag} must be fatal`).toBe("error");
      if (res.kind === "error") {
        expect(res.message).toContain(`${flag} was removed on 2026-09-25`);
        expect(res.message, `${flag}'s reason must be specific to it, not a shared string`).toMatch(reason);
        // The usage text is what an operator reading a container log has to work from, and it is the one
        // thing that tells them what to write instead.
        expect(res.message).toContain("Usage:");
      }
    }
  });

  it("refuses it BEFORE consuming its value, so the flag is what gets named", () => {
    // `--token-file /run/token` must report the flag, not complain about the stray `/run/token`. The
    // removed-flag check runs before any value is read, which is what makes that true.
    const res = parseHostArgs(["--data-dir", "/data", "--token-file", "/run/token"]);
    expect(res.kind).toBe("error");
    if (res.kind === "error") {
      expect(res.message).toContain("--token-file was removed on 2026-09-25");
      expect(res.message).not.toContain("unexpected argument");
    }
  });

  it("refuses the `=` form too, because the split happens before the lookup", () => {
    // A compose file written for the old parser may well have used `--ws-allow-non-loopback=false`, and the
    // split that turns `--flag=value` into `--flag` runs before the table is consulted. If that order ever
    // reversed, this would report an unrecognised `--ws-allow-non-loopback=false` instead — a worse message
    // for a flag the parser demonstrably knew about.
    const res = parseHostArgs(["--data-dir", "/data", "--ws-allow-non-loopback=false"]);
    expect(res.kind).toBe("error");
    if (res.kind === "error") {
      expect(res.message).toContain("--ws-allow-non-loopback was removed on 2026-09-25");
    }
  });

  it("does NOT advertise them in the usage text — a documented flag must work", () => {
    // The reverse of `DOCUMENTED_FLAGS`, and not merely cosmetic: the case above takes every flag out of
    // `HOST_USAGE` and requires the parser to accept it, so a removed flag left in the prose would fail
    // there — but only for whichever one was left, and with a message about documentation. This one fails
    // for all of them at once and names the culprit.
    for (const [flag] of removed) {
      expect(HOST_USAGE, `${flag} is gone, so the usage text must not advertise it`).not.toContain(flag);
    }
  });
});

/**
 * `plan/15` work item 3 — the edge router's `--edge-port`.
 *
 * ## What is left of the block that used to be here
 *
 * Work item 3 added three flags to the parser and the block that stood here asserted all three. Two of them —
 * `--edge-bind-host` and `--edge-allow-non-loopback` — went on 2026-09-25 with the other bind opt-ins, and
 * their diagnosis is asserted in the block above. `--edge-port` survives because it is **not** a policy flag:
 * it chooses which port one loopback listener uses, which is a question an operator whose machine already has
 * something on 80 still has to answer.
 *
 * What is *not* asserted here is the bind policy itself. That belongs to `reverseproxy/edge.ts`, which owns it, and
 * `tests/proxy/edge.test.ts` is where it is pinned — a copy of the rule here would be a copy that can drift.
 */
describe("parseHostArgs — --edge-port (plan/15 work item 3)", () => {
  it("reads --edge-port, and validates it as an integer in range", () => {
    const ok = parseHostArgs(["--data-dir", "/data", "--edge-port", "8080"]);
    expect(ok.kind).toBe("args");
    if (ok.kind === "args") expect(ok.args.edgePort).toBe(8080);

    for (const bad of ["not-a-port", "70000", "-1", "80.5"]) {
      const res = parseHostArgs(["--data-dir", "/data", "--edge-port", bad]);
      expect(res.kind, `${bad} is not a port`).toBe("error");
      if (res.kind === "error") expect(res.message).toMatch(/--edge-port must be an integer from 0 to 65535/);
    }
  });

  it("leaves edgePort UNDEFINED when absent, so the host applies app.json's port", () => {
    // The migration decision (`plan/15` §6.2) has one home: the host reads `settings.port`, which is the port
    // every existing `*.localhost` mapping already points at. A parser that invented a default here would put
    // that decision in a second place, and the two would eventually disagree about which port mappings answer on.
    const res = parseHostArgs(["--data-dir", "/data"]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") expect(res.args.edgePort).toBeUndefined();
  });

  it("works alongside --rpc-socket: the edge's port is independent of the RPC transport", () => {
    // The RPC endpoint and the edge are different servers. A host that serves its RPC over a unix socket — the
    // default, and the safest choice — still has an edge, so the WebSocket-versus-socket rule must not reach
    // this flag. The removed-flag cases above cover the `--edge-bind-host` half of what used to be asserted here.
    const res = parseHostArgs([
      "--data-dir", "/data",
      "--rpc-socket", "/run/bifurc.sock",
      "--edge-port", "8080",
    ]);
    expect(res.kind).toBe("args");
    if (res.kind === "args") {
      expect(res.args.rpcSocket).toBe("/run/bifurc.sock");
      expect(res.args.edgePort).toBe(8080);
    }
  });
});
