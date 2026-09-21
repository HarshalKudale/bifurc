/**
 * P14 work item 6 — the **deployable host artifact**: the executable a container runs.
 *
 * ```bash
 * node <engine>/dist/runtime/hostCli.js --data-dir /data --rpc-socket /run/bifurc.sock --token-file /run/token
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
 */
import * as fs from "fs";
import { runWorkspaceHost, type WorkspaceHostOptions, type HostTransport } from "./host";
import { HOST_USAGE, parseHostArgs } from "./hostArgs";

/**
 * Read the engine token from a mounted file.
 *
 * Trimmed, because a secret mounted from a file very often ends in a newline and a token with a trailing
 * `\n` fails authentication with a message that does not mention whitespace — the client sends what it
 * read and the engine compares what it generated.
 *
 * A missing or unreadable file is **fatal**: the operator named it, so falling back to a generated token
 * would start a host whose token is not the one the client was given, which is the kind of failure that
 * presents as an unexplained `UNAUTHORIZED` much later.
 */
function readTokenFile(file: string): string {
  try {
    return fs.readFileSync(file, "utf-8").trim();
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    throw new Error(`--token-file ${file} could not be read: ${reason}`);
  }
}

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

  const transport: HostTransport = { kind: "socket", path: args.rpcSocket };
  const opts: WorkspaceHostOptions = {
    dataDir: args.dataDir,
    transport,
    workspaceIds: args.workspaceIds,
    token: args.tokenFile === undefined ? undefined : readTokenFile(args.tokenFile),
    // Default notices already name each workspace and its resolved ports; keeping the host's own channel
    // means the container's log and a test's `onNotice` say the same things in the same order.
  };

  const host = await runWorkspaceHost(opts);
  const status = host.status();

  // One line per workspace, after start, so "which port is it on?" has an answer in the container log
  // rather than requiring a shell into the container to ask `server.status`.
  for (const rt of status.runtimes) {
    console.log(
      `[host] workspace ${rt.workspaceId}: proxy ${rt.proxyPort ?? "-"} webhook ${rt.webhookPort ?? "-"}`,
    );
  }
  console.log(`[host] serving ${status.runtimes.length} workspace(s); rpc ${status.rpcAddress}`);
  for (const s of status.skipped) {
    // A workspace the host could not place. `plan/14` D16 makes a taken *override* fail loudly but not
    // fatally, so this is a warning rather than a startup failure — the others are still serving.
    console.warn(`[host] skipped ${s.workspaceId}: ${s.reason}`);
  }
}

void main(process.argv.slice(2)).catch((err: unknown) => {
  // A startup failure is the container's problem to surface, so it goes to stderr with a non-zero code.
  // `PortRangeExhaustedError` and a socket that cannot bind both land here.
  const reason = err instanceof Error ? err.message : String(err);
  console.error(`[host] failed to start: ${reason}`);
  process.exitCode = 1;
});
