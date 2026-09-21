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
 */

/** The parsed command line. `dataDir` is the only required field — see `parseHostArgs`. */
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
  /** Read the engine token from this file — for a container that mounts a secret. Absent → `ensureEngineToken()`. */
  tokenFile?: string;
}

export type HostArgsResult =
  | { kind: "args"; args: HostArgs }
  /** `--help`. Not an error — the caller prints `HOST_USAGE` to **stdout** and exits **0**. */
  | { kind: "help" }
  /** A malformed command line. The message already carries the usage text. Exit **1**. */
  | { kind: "error"; message: string };

/** Printed on `--help`, and on any parse failure so the operator sees what was expected. */
export const HOST_USAGE = `Usage: node <engine>/dist/runtime/hostCli.js --data-dir <path> [options]

A workspace host: N workspaces behind ONE RPC endpoint, no companion server.

Required
  --data-dir <path>       The data root. Holds app.json, the workspaces and the blob store.

Options
  --rpc-socket <path>     The RPC endpoint's path. Defaults to the platform's standard location
                          (a named pipe on Windows, a 0600 unix socket elsewhere).
  --token-file <path>     Read the engine token from this file instead of generating one.
                          For a container that mounts the token as a secret.
  --workspace <id>        Serve only this workspace. Repeatable, and the order is preserved.
                          Omitted means every workspace in app.json.
  -h, --help              Print this.

Environment
  BIFURC_ENGINE_TOKEN      The token, if not passed via --token-file.
  BIFURC_PROXY_BASE_PORT   Moves the proxy base port (D16). The base is 80 on Windows and 8080
                           elsewhere; each further workspace counts up from it.
  BIFURC_DATA_DIR          The data root, if not passed via --data-dir.

Documented deployment: one container = one workspace, one volume = that workspace's data/ + blobs/.
N workspaces per host is supported but not recommended — one per volume makes the volume, the ports
and the lifecycle line up.`;

/** Flags that take a value. Kept as a table so "does this flag need an argument?" has one answer. */
const VALUE_FLAGS = new Set(["--data-dir", "--rpc-socket", "--token-file", "--workspace"]);

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
  let tokenFile: string | undefined;
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
      case "--token-file":
        tokenFile = value;
        break;
      case "--workspace":
        workspaceIds.push(value);
        break;
    }
  }

  if (dataDir === undefined || dataDir === "") {
    return {
      kind: "error",
      message: `--data-dir is required (or set BIFURC_DATA_DIR).\n\n${HOST_USAGE}`,
    };
  }

  return {
    kind: "args",
    args: {
      dataDir,
      rpcSocket,
      tokenFile,
      // Omitted rather than empty when nothing was passed, so the host's own default applies and
      // "serve every workspace" is not expressed as an empty list — which the host would read as
      // "serve none".
      workspaceIds: workspaceIds.length > 0 ? workspaceIds : undefined,
    },
  };
}
