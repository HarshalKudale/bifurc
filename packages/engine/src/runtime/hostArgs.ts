/**
 * P14 work item 6 — the argument parser for the deployable host artifact.
 *
 * Split from the executable (`hostCli.ts`) so the **risky half is testable**. The executable runs on
 * import — nothing imports it, which is why it needs no `main` guard, and why it cannot be imported by a
 * test without starting a host. So the parsing, the validation and the usage text live here, where a suite
 * can drive them directly, and `hostCli.ts` is the thin shell that calls `runWorkspaceHost()`.
 *
 * ## Why not a `main` guard in `host.ts` itself
 *
 * The plan sketches the invocation as `node <engine>/runtime/host.js`, which would mean putting this in
 * `host.ts` behind a guard. **That does not survive this package's build.** `tsup.config.ts` emits **both**
 * esm and cjs, and the two formats need different entry checks — `require.main === module` for cjs,
 * `import.meta.url === pathToFileURL(process.argv[1]).href` for esm — while `import.meta` is a syntax error
 * in a cjs output. A guard that works in one leg and not the other is worse than a separate module, and
 * `host.ts` is imported by tests and by `registerAll`, so an unguarded execution there would start a host
 * inside every importing process.
 *
 * ## Fail loudly on an unknown or malformed flag
 *
 * The same rule D16 applies to `BIFURC_PROXY_BASE_PORT`: a container that passed an argument **meant** it,
 * and silently ignoring one is how an operator ends up with a host bound to the wrong socket while the
 * command line looks right. So an unrecognised flag is an error, not a warning, and a flag missing its
 * value is an error rather than an empty string.
 *
 * ## The same rule applied to *combinations*
 *
 * A flag that parses and then does nothing is the same failure one step further along, so the two RPC
 * flags are checked against each other: `--rpc-socket` selects a unix socket or a named pipe and
 * `--ws-port` selects a WebSocket listener, and one host serves one endpoint.
 *
 * **This section used to be four times as long, and its length was the bug.** `plan/15` work items 1–3
 * added a bind host and an opt-in for each of the three listeners — the RPC endpoint, the forward proxy
 * and the edge — plus a TLS pair for the RPC one, and each opt-in needed a coherence check so that an
 * *accepted-and-ignored* flag was refused by name. All of it is gone as of **2026-09-25**: the engine is
 * never deployed remotely, every listener binds loopback, and there is no address left to acknowledge.
 * `REMOVED_FLAGS` below is what an operator who still passes one of them now gets.
 *
 * ## The host's own domain — the flag that was here, and why it is gone
 *
 * `--domain` was the fourth flag added to this parser, and the only one after `--data-dir` that was
 * **required**: the hostname the engine answered as, `localhost` or a `domain.TLD`. It was removed on
 * **2026-09-24**, and the reason is a deployment decision rather than a simplification.
 *
 * **The engine is no longer deployed in the cloud.** It runs either inside the Electron app or as a
 * locally deployed container — a per-user instance, which is the shape the admin console and the login flow
 * will need. So the name it is addressed as is `localhost` in every deployment that exists, which is a fact
 * about the product rather than a choice the operator makes. `lib/net.ts`'s `DEFAULT_HOST_DOMAIN` is that
 * fact, and `runtime/host.ts` composes it directly.
 *
 * Three things made keeping the flag worse than removing it, and the third is the one that decides it:
 *
 * 1. **A flag that can hold one value is the *accepted and ignored* shape this file refuses elsewhere** — it
 *    parses, it reaches the host, and it changes no outcome.
 * 2. **It was not merely inert, it was an invitation.** Naming the engine `engine.example.com` turns every
 *    `*.localhost` mapping into a name RFC 6761 resolves to the *visitor's* own machine — a mapping that
 *    silently stops working for somebody else, which is the failure the old docblock here called "invisible
 *    until a mapping stops working".
 * 3. **It was the last thing standing between "local" and "cloud" in the code**, and the programme has
 *    decided that direction. Leaving the flag would leave the capability, and a capability nothing tests is
 *    a capability that rots.
 *
 * **What read it now reads the constant.** `plan/15` §3.3's routing table still needs to know which `Host` is
 * the engine's own, and §4.3's mapping editor still needs the suffix to suggest (work item 7) — both get
 * `localhost`, and the client still learns it from `hello`'s `hostDomain` rather than guessing it, because
 * §4.1 is explicit that the client does not guess this. See `lib/net.ts#DEFAULT_HOST_DOMAIN`.
 */

/** The parsed command line. `dataDir` is the one required field — see `parseHostArgs`. */
export interface HostArgs {
  /** The data root. Required, and never invented: `store/paths.ts` throws rather than defaulting. */
  dataDir: string;
  /**
   * Restrict the host to these workspaces, in this order. Absent means "every workspace in `app.json`",
   * which is the host's own default.
   */
  workspaceIds?: string[];
  /** The RPC socket path. Absent → `defaultSocketPath()` (a named pipe on Windows, a `0600` unix socket). */
  rpcSocket?: string;
  /**
   * The WebSocket bind port. When present, the CLI starts the host in `ws` mode instead of `socket`.
   *
   * **This flag is what selects WebSocket mode.** It used to be joined by `--ws-host`, which existed only
   * to name a non-loopback address and was removed on 2026-09-25 with the bind opt-ins — see the header.
   * With one flag left, "WebSocket flags given without WebSocket mode" is no longer a line anybody can
   * write, which is why that coherence check is gone rather than kept.
   */
  wsPort?: number;
  /**
   * The port the **edge router** binds — `plan/15` work item 3. Omitted → `app.json`'s `port`
   * (`settings.port`, default 80), which is the port existing `*.localhost` mappings already point at.
   *
   * A flag as well as a default because the default is a *migrated* value rather than a chosen one: it
   * is `settings.port` precisely so that no existing mapping has to change, and an operator whose
   * machine already has something on 80 needs a way to say so without editing a JSON file inside the
   * container's volume.
   */
  edgePort?: number;
}

export type HostArgsResult =
  | { kind: "args"; args: HostArgs }
  /** `--help`. Not an error — the caller prints `HOST_USAGE` to **stdout** and exits **0**. */
  | { kind: "help" }
  /** A malformed command line. The message already carries the usage text. Exit **1**. */
  | { kind: "error"; message: string };

/** Printed on `--help`, and on any parse failure so the operator sees what was expected. */
export const HOST_USAGE = `Usage: node <engine>/dist/runtime/hostCli.js --data-dir <path> [options]

A workspace host: N workspaces behind ONE RPC endpoint, no app-level singleton server.

Every listener binds loopback and there is no way to change that: the engine is not deployable in the
cloud, so every client is on this machine. It answers as "localhost" — a subdomain of it
(<sub>.localhost) is a reverse-proxy mapping, and the base domain itself serves the mapping directory.

Required
  --data-dir <path>       The data root. Holds app.json, the workspaces and the blob store.

Options
  --rpc-socket <path>     The RPC endpoint's path. Defaults to the platform's standard location
                          (a named pipe on Windows, a 0600 unix socket elsewhere).
  --ws-port <port>        Serve the RPC endpoint over WebSocket on this port, on loopback, instead of
                          over a socket. Use 0 for an ephemeral port. This flag is what selects
                          WebSocket mode.
  --workspace <id>        Serve only this workspace. Repeatable, and the order is preserved.
                          Omitted means every workspace in app.json.
  --edge-port <port>      The edge router's port — the reverse proxy and the home page, ONE listener
                          for the whole host (plan/15 §3.1). Defaults to the port in app.json (80),
                          which is the port existing *.localhost mappings already point at, so no
                          existing mapping has to change.
  -h, --help              Print this.

Environment
  BIFURC_PROXY_BASE_PORT   Moves the proxy base port (D16). The base is 80 on Windows and 8080
                           elsewhere; each further workspace counts up from it.
  BIFURC_DATA_DIR          The data root, if not passed via --data-dir.

Documented deployment: one container = one workspace, one volume = that workspace's data/ + blobs/.
N workspaces per host is supported but not recommended — one per volume makes the volume, the ports
and the lifecycle line up.`;

/**
 * Flags that take a value. Kept as a table so "does this flag need an argument?" has one answer.
 */
const VALUE_FLAGS = new Set([
  "--data-dir",
  "--rpc-socket",
  "--ws-port",
  "--workspace",
  "--edge-port",
]);

/**
 * Flags that **used to** be parsed here, with the reason each went, so an operator upgrading a compose
 * file gets a diagnosis instead of `unrecognised flag`.
 *
 * The generic message is true and useless for this class of argument: it sends somebody hunting for a typo
 * in a flag they copied from a page that was correct when it was written. The header's rule is that a
 * container which passed an argument **meant** it — these are the only arguments whose meaning this file
 * can still recover, because it is what used to give them one.
 *
 * All nine went on **2026-09-25** for the same underlying reason: the engine is never deployed remotely,
 * so there is no address to publish and no boundary for a token to guard. The token half is
 * `transport/auth/token.ts`'s deletion; the bind half is the unconditional loopback guard in
 * `transport/ws.ts`, `proxy/server.ts` and `reverseproxy/edge.ts`.
 */
const REMOVED_FLAGS = new Map<string, string>([
  ["--ws-host", "every listener binds loopback, so the bind host is not a choice"],
  ["--ws-allow-non-loopback", "the engine is not deployable remotely, so there is nothing to publish"],
  ["--ws-tls-cert", "TLS was only required to satisfy the non-loopback bind, which no longer exists"],
  ["--ws-tls-key", "TLS was only required to satisfy the non-loopback bind, which no longer exists"],
  [
    "--token-file",
    "the engine token is gone — the RPC surface binds loopback, so there is no boundary to guard",
  ],
  ["--proxy-bind-host", "the forward proxy binds loopback, always"],
  ["--proxy-allow-non-loopback", "the engine is not deployable remotely, so there is nothing to publish"],
  ["--edge-bind-host", "the edge binds loopback, always"],
  ["--edge-allow-non-loopback", "the engine is not deployable remotely, so there is nothing to publish"],
]);

/**
 * Parse `argv` (already sliced past `node` and the script path).
 *
 * Accepts both `--flag value` and `--flag=value`. The `=` form is worth supporting rather than rejecting:
 * it is what people write for a path containing spaces, and refusing it would produce a confusing
 * "unrecognised flag `--data-dir=/data`".
 */
export function parseHostArgs(argv: readonly string[]): HostArgsResult {
  let dataDir: string | undefined = process.env.BIFURC_DATA_DIR;
  let rpcSocket: string | undefined;
  let wsPort: number | undefined;
  let edgePort: number | undefined;
  const workspaceIds: string[] = [];

  for (let i = 0; i < argv.length; i += 1) {
    const raw = argv[i];

    if (raw === "-h" || raw === "--help") {
      return { kind: "help" };
    }

    // Split the `=` form once, so `--data-dir=/data` and `--data-dir /data` reach the same branch.
    const eq = raw.indexOf("=");
    const flag = eq === -1 ? raw : raw.slice(0, eq);
    let value = eq === -1 ? undefined : raw.slice(eq + 1);

    if (!flag.startsWith("--")) {
      return { kind: "error", message: `unexpected argument ${JSON.stringify(raw)}.\n\n${HOST_USAGE}` };
    }

    /**
     * A removed flag is refused **by name and with its reason**, and the check runs before the
     * unknown-flag branch so the specific answer wins. It also runs before any value is consumed, so
     * `--token-file /run/token` reports the flag rather than complaining about the stray `/run/token`.
     */
    const removedReason = REMOVED_FLAGS.get(flag);
    if (removedReason !== undefined) {
      return {
        kind: "error",
        message: `${flag} was removed on 2026-09-25: ${removedReason}.\n\n${HOST_USAGE}`,
      };
    }

    if (!VALUE_FLAGS.has(flag)) {
      // An unrecognised flag is fatal rather than ignored — see the header. A typo'd `--data-dirr` that
      // was silently skipped would leave the host on the default data root, which is the failure that
      // looks like "my workspaces vanished".
      return { kind: "error", message: `unrecognised flag ${JSON.stringify(flag)}.\n\n${HOST_USAGE}` };
    }

    if (value === undefined) {
      const next = argv[i + 1];
      // A value that looks like another flag is a missing value, not a value. Without this, `--data-dir
      // --help` would take the literal string "--help" as the data root and create a directory called that.
      if (next === undefined || next.startsWith("--")) {
        return { kind: "error", message: `${flag} needs a value.\n\n${HOST_USAGE}` };
      }
      value = next;
      i += 1;
    }
    if (value === "") {
      return { kind: "error", message: `${flag} needs a value.\n\n${HOST_USAGE}` };
    }

    switch (flag) {
      case "--data-dir":
        dataDir = value;
        break;
      case "--rpc-socket":
        rpcSocket = value;
        break;
      case "--ws-port": {
        const parsed = Number(value);
        if (!Number.isInteger(parsed) || parsed < 0 || parsed > 65535) {
          return { kind: "error", message: `--ws-port must be an integer from 0 to 65535.\n\n${HOST_USAGE}` };
        }
        wsPort = parsed;
        break;
      }
      case "--workspace":
        workspaceIds.push(value);
        break;
      case "--edge-port": {
        // Validated the way `--ws-port` is, and for the same reason: a non-numeric port reaches
        // `listen()` as `NaN` and surfaces as an obscure `ERR_SOCKET_BAD_PORT` from inside the host,
        // after the data root has been touched. This one is a command-line fact.
        const parsed = Number(value);
        if (!Number.isInteger(parsed) || parsed < 0 || parsed > 65535) {
          return { kind: "error", message: `--edge-port must be an integer from 0 to 65535.\n\n${HOST_USAGE}` };
        }
        edgePort = parsed;
        break;
      }
    }
  }

  if (dataDir === undefined || dataDir === "") {
    return {
      kind: "error",
      message: `--data-dir is required (or set BIFURC_DATA_DIR).\n\n${HOST_USAGE}`,
    };
  }

  /**
   * ## The one coherence rule left
   *
   * A host serves **one** RPC endpoint, and these two flags select different kinds: a unix socket or named
   * pipe, or a WebSocket listener. Both together is a line that cannot mean what it says, so it is refused
   * rather than resolved by precedence.
   *
   * **The *policy* is still not restated here.** A bind policy belongs to the module that binds, and since
   * 2026-09-25 there is no policy to state: loopback is not a default the parser could contradict, it is
   * the only value the guards accept. See `transport/ws.ts`, `proxy/server.ts` and `reverseproxy/edge.ts`.
   */
  if (rpcSocket !== undefined && wsPort !== undefined) {
    return {
      kind: "error",
      message:
        `--rpc-socket cannot be combined with --ws-port — a host serves one RPC endpoint, and these ` +
        `select different kinds of endpoint.\n\n${HOST_USAGE}`,
    };
  }

  return {
    kind: "args",
    args: {
      dataDir,
      rpcSocket,
      wsPort,
      // Omitted rather than empty when nothing was passed, so the host's own default applies and
      // "serve every workspace" is not expressed as an empty list — which the host would read as
      // "serve none".
      workspaceIds: workspaceIds.length > 0 ? workspaceIds : undefined,
      // Left `undefined` when absent so the host applies its own default — `app.json`'s `port` — rather
      // than the parser inventing one here, which would put the migration decision in a second place.
      edgePort,
    },
  };
}
