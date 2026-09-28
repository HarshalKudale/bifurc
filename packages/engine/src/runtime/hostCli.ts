/**
 * P14 work item 6 — the **deployable host artifact**: the executable a container runs.
 *
 * ```bash
 * node <engine>/dist/runtime/hostCli.js --data-dir /data --rpc-socket /run/bifurc.sock
 * node <engine>/dist/runtime/hostCli.js --data-dir /data --ws-port 4747
 * ```
 *
 * Documented deployment: **one container = one workspace**, one volume = that workspace's `data/` +
 * `blobs/`. N workspaces per host is supported and exercised by the integration suite, but one per volume
 * is what makes the volume, the ports and the lifecycle line up — which is why P9 (the container) comes
 * after this phase.
 *
 * ## Why this is not `host.ts`
 *
 * The plan sketches `node <engine>/runtime/host.js`. Putting the CLI in `host.ts` would need a `main`
 * guard, and **this package emits both esm and cjs** (`tsup.config.ts`), whose entry checks differ —
 * `require.main === module` versus `import.meta.url === pathToFileURL(process.argv[1]).href` — while
 * `import.meta` is a syntax error in a cjs output. Worse, `host.ts` is imported by tests and by
 * `registerAll`, so an unguarded execution there would start a host inside every importing process.
 * So the executable is its own module and **nothing imports it** — which is exactly why it needs no
 * guard, and why `hostArgs.ts` exists to keep the testable half separate.
 *
 * ## What it deliberately does not do
 *
 * It does not install signal handlers — `runWorkspaceHost()` does, and exits `128 + signo` on
 * `SIGTERM`/`SIGINT` (the shell convention). A CLI that also caught them would race that, and the
 * exit code is the container orchestrator's only signal about *why* the process stopped.
 *
 * It does not read `app.json` itself. The host owns the workspace list and the bootstrap sequence
 * (`bootstrapWorkspaces()`), so the CLI's whole job is argv → `WorkspaceHostOptions`.
 *
 * ## What it stopped reading on 2026-09-25
 *
 * Three things, all for the same reason — the engine is not deployable remotely, so every listener binds
 * loopback and there is no boundary for a credential to guard:
 *
 * - **`--token-file`** and its `readTokenFile()`, which existed so a container could mount the per-start
 *   secret. The secret itself is gone (`transport/auth/token.ts` was deleted).
 * - **`--ws-tls-cert`/`--ws-tls-key`** and their `readPemFile()`. TLS on the RPC channel was the *second
 *   condition* of the non-loopback bind, not a feature in its own right — see `transport/ws.ts`'s header.
 * - **`--ws-host`, `--proxy-bind-host`, `--edge-bind-host`** and the three `--*-allow-non-loopback` opt-ins.
 *   The bind host is not a choice any more, so passing it through would be passing through a value with one
 *   legal setting.
 *
 * `hostArgs.ts`'s `REMOVED_FLAGS` names all of them to an operator who still passes one, so the failure is a
 * diagnosis rather than an `unrecognised flag`.
 */
import { runWorkspaceHost, type WorkspaceHostOptions, type HostTransport } from "./host";
import { HOST_USAGE, parseHostArgs } from "./hostArgs";

async function main(argv: readonly string[]): Promise<void> {
  const parsed = parseHostArgs(argv);

  if (parsed.kind === "help") {
    // stdout and exit 0: `--help` is a successful request, not a malformed command line.
    process.stdout.write(`${HOST_USAGE}\n`);
    return;
  }
  if (parsed.kind === "error") {
    process.stderr.write(`${parsed.message}\n`);
    process.exitCode = 1;
    return;
  }

  const { args } = parsed;

  /**
   * `--ws-port` **is** the WebSocket mode switch, and it is the only one.
   *
   * It used to share that job with `--ws-host`, which existed to name a non-loopback address; with the bind
   * hosts gone (see the header) a port on its own is unambiguous, and the host applies `DEFAULT_WS_HOST`
   * itself. Note the absence of any bind host, `tls` or `allowNonLoopback` here: those were the fields a
   * caller could use to widen the bind, and `transport/ws.ts` no longer has them.
   */
  const transport: HostTransport =
    args.wsPort !== undefined ? { kind: "ws", port: args.wsPort } : { kind: "socket", path: args.rpcSocket };

  const opts: WorkspaceHostOptions = {
    dataDir: args.dataDir,
    // **No `domain`.** The host answers as `localhost`, which it composes itself, and the parser no longer has
    // a flag for it — removed 2026-09-24 because the engine is no longer deployed in the cloud
    // (`hostArgs.ts`'s header records the reasoning). There is nothing here to pass through.
    transport,
    workspaceIds: args.workspaceIds,
    // Default notices already name each workspace and its resolved ports; keeping the host's own channel
    // means the container's log and a test's `onNotice` say the same things in the same order.
    //
    // The edge's port is the one bind-ish number that is still an operator's choice, and it is a *port*
    // rather than an *address*: the edge is ONE listener for the whole host (`plan/15` §3.1), its default is
    // a migrated value rather than a chosen one (`app.json`'s `port`, so existing `*.localhost` mappings do
    // not have to change), and an operator whose machine already has something on 80 needs a way to say so
    // without editing JSON inside a volume.
    edgePort: args.edgePort,
  };

  const host = await runWorkspaceHost(opts);
  const status = host.status();

  // The host's own name, first, because it is what a reader needs to interpret the edge line that follows:
  // `plan/15` §3.3's routing table and §4.3's mapping editor both key off it, so what a mapping's domain looks
  // like depends on it.
  //
  // **It is no longer a value the operator supplied.** Until 2026-09-24 this line reported `--domain`, which
  // was the one startup value that came from the command line rather than from the machine; the flag is gone
  // (the engine is not deployable in the cloud), so this is the constant the host composed. Printed anyway
  // rather than dropped, and for the same reason as before: it is what tells an operator that
  // `<sub>.localhost` is the shape of a mapping on this engine.
  console.log(
    `[host] domain ${status.domain} — the name this engine answers as (mappings: <sub>.${status.domain})`,
  );

  // The edge first, because it is one listener for the whole host rather than one per workspace — so it
  // reads as the host's own address, ahead of the port lines below.
  //
  // A failed bind is a **warning**, unlike a busy proxy port which is routine. `WorkspaceHostStatus.edgeError`
  // carries the reasoning; in one line: if the edge is down, nothing on the reverse path resolves, which is
  // the product surface rather than one of several.
  if (status.edgeError !== null) {
    console.warn(
      `[host] edge on ${status.edgeAddress ?? "(unknown)"} FAILED to bind: ${status.edgeError}` +
        ` — no *.localhost mapping will resolve`,
    );
  } else if (status.edgeAddress !== null) {
    console.log(`[host] edge ${status.edgeAddress} — reverse proxy and home page for the active workspace`);
  }

  /**
   * ## The port lines, rewritten for `plan/17`
   *
   * There used to be **one line per workspace**, each naming that workspace's proxy and webhook port —
   * because each had its own. There is now one listener per port for the whole host, so per-workspace lines
   * would print the same two numbers N times. The ports are printed once, and the workspaces are listed
   * separately as what they now are: **config scope**.
   *
   * The forward proxy gets its own line because it is the one number an operator has to **act on** — it goes
   * into a browser's proxy setting — so it is stated even when it failed, with the failure attached rather
   * than replaced by a dash. The webhook is printed only when it is up: `plan/17` §1 makes it off by default,
   * so "not running" is the ordinary state and a line announcing it on every start would be noise.
   */
  if (status.forwardProxyError !== null) {
    console.warn(
      `[host] forward proxy FAILED to bind ${status.forwardProxyPort ?? "(unknown)"}: ` +
        `${status.forwardProxyError} — the browser's proxy setting points here`,
    );
  } else if (status.forwardProxyPort !== null) {
    console.log(
      `[host] forward proxy ${status.forwardProxyPort} — point the browser's proxy setting here`,
    );
  }
  if (status.webhookPort !== null) {
    console.log(`[host] webhook receiver ${status.webhookPort}`);
  }
  if (status.companionPort !== null) {
    console.log(`[host] companion ${status.companionPort}`);
  }
  console.log(
    `[host] serving ${status.workspaces.length} workspace(s): ` +
      `${status.workspaces.map((w) => w.workspaceId).join(", ") || "(none)"}; rpc ${status.rpcAddress}`,
  );
}

void main(process.argv.slice(2)).catch((err: unknown) => {
  // A startup failure is the container's problem to surface, so it goes to stderr with a non-zero code. A
  // socket that cannot bind, and the companion scan running out of room, both land here.
  const reason = err instanceof Error ? err.message : String(err);
  console.error(`[host] failed to start: ${reason}`);
  process.exitCode = 1;
});
