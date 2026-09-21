/**
 * P14 work item 1 — the **host-local** runtime registry: `runtime.json` at the data root.
 *
 * ## What it holds, and the one thing it must never hold
 *
 * `plan/14` §3 / D16: the resolved port of each workspace *on this machine* is remembered so a restart
 * reuses it. A workspace that landed on 81 because 80 was busy **keeps 81** after 80 frees up, because a
 * stable proxy URL is worth more to the browser-proxy workflow than reclaiming the base.
 *
 * It therefore holds a map `wsId -> { resolvedProxyPort?, resolvedWebhookPort?, preferredProxyPort?,
 * preferredWebhookPort? }` and **nothing else**. In particular it must never hold anything about the
 * workspace's *content*.
 *
 * ## Why this is not `workspace.json` — the whole reason the file exists
 *
 * `plan/14` §3 states it as a hard rule: *"The port must not live in `workspace.json`."* That file is
 * git-tracked (`store/gitStore.ts` commits it) and read back as the workspace's identity
 * (`sync/syncManager.ts`), so a port stored there would **sync between machines and collide on the
 * second one**. A port is a property of *this machine hosting this workspace*, not of the workspace's
 * content — so it lives beside the data root, not inside the synced tree.
 *
 * ## Two decisions made here rather than in the caller
 *
 * 1. **Sibling file, not a new directory.** `runtime.json` sits at the data root next to `app.json`,
 *    because it has the same scope: host-local state about this installation. `<dataDir>/runtime.json`.
 * 2. **A corrupt or unreadable registry is not fatal — it is discarded.** The file is pure cache: every
 *    value in it is a *preference* the allocator can re-derive, and the authoritative answer is whatever
 *    the real `listen()` produces at start. Throwing on a bad `runtime.json` would let a half-written
 *    file from a killed process stop the whole host from starting, which is strictly worse than losing a
 *    remembered port. The same reasoning `loadSettings()` uses for a bad `app.json`.
 *
 *    The consequence, stated so it is not a surprise: a discarded registry means every workspace
 *    re-scans from the base, so a host whose `runtime.json` was lost may land its workspaces on
 *    different ports than last time. That is the designed degradation — a *remembered* port is an
 *    optimisation over the scan, never a guarantee.
 */
import * as fs from "fs";
import * as path from "path";
import { dataDir } from "../store/paths";

/** `<dataDir>/runtime.json`. */
export const RUNTIME_REGISTRY_FILENAME = "runtime.json";

/**
 * One workspace's host-local entry.
 *
 * `V`-suffixed names are the *resolved* values — what the workspace actually bound last time, recorded so
 * a restart reuses them. The `preferred*` names are the user's explicit override, which is a different
 * thing: D16's rule is that an override **fails loudly** when taken, so it must survive independent of
 * whatever the allocator last resolved.
 */
export interface WorkspaceRuntimeEntry {
  /** The port the user pinned for this workspace's proxy. D16: a taken override is a hard error. */
  preferredProxyPort?: number;
  /** The port the user pinned for this workspace's webhook receiver. Same rule. */
  preferredWebhookPort?: number;
  /** The port this workspace's proxy last bound. Advisory — reused on restart when still free. */
  resolvedProxyPort?: number;
  /** The port this workspace's webhook receiver last bound. Advisory. */
  resolvedWebhookPort?: number;
}

export interface RuntimeRegistry {
  /** Registry format version, so a future change can migrate rather than discard. */
  version: number;
  workspaces: Record<string, WorkspaceRuntimeEntry>;
}

export const RUNTIME_REGISTRY_VERSION = 1;

/** Where the registry lives for the **currently-set** data root. Throws if no root is set — `store/paths.ts`'s
 * `dataDir()` contract, deliberately not softened here: a registry written to the cwd is worse than an error. */
export function runtimeRegistryPath(): string {
  return path.join(dataDir(), RUNTIME_REGISTRY_FILENAME);
}

/**
 * Read the registry, returning an empty one when it is absent, unreadable, malformed, or from a future
 * version. **Never throws** — see the header's point 2.
 */
export function loadRuntimeRegistry(): RuntimeRegistry {
  const empty: RuntimeRegistry = { version: RUNTIME_REGISTRY_VERSION, workspaces: {} };
  let raw: string;
  try {
    raw = fs.readFileSync(runtimeRegistryPath(), "utf-8");
  } catch {
    return empty; // Absent — an ordinary first start.
  }
  try {
    const parsed = JSON.parse(raw) as Partial<RuntimeRegistry>;
    // A future version may have moved or reinterpreted a field. Re-deriving is cheap and always correct;
    // misreading a newer file is neither.
    if (parsed.version !== RUNTIME_REGISTRY_VERSION) return empty;
    if (parsed.workspaces === undefined || typeof parsed.workspaces !== "object" || parsed.workspaces === null) {
      return empty;
    }
    // Per-entry validation rather than trusting the cast: a port read back as `"8080"` (a string from a
    // hand-edited file) would flow into `listen()` as a truthy non-number and bind something surprising.
    const workspaces: Record<string, WorkspaceRuntimeEntry> = {};
    for (const [wsId, entry] of Object.entries(parsed.workspaces)) {
      if (entry === null || typeof entry !== "object") continue;
      // `Object.entries` on a `Partial<RuntimeRegistry>` field gives us `WorkspaceRuntimeEntry`, but the value
      // on disk is unvalidated JSON — so it is read as an index-signature record and every field is checked.
      const raw = entry as Record<string, unknown>;
      workspaces[wsId] = {
        ...portField(raw, "preferredProxyPort"),
        ...portField(raw, "preferredWebhookPort"),
        ...portField(raw, "resolvedProxyPort"),
        ...portField(raw, "resolvedWebhookPort"),
      };
    }
    return { version: RUNTIME_REGISTRY_VERSION, workspaces };
  } catch {
    return empty; // Malformed JSON, or a truncated write from a killed process.
  }
}

/** Keep one numeric port field, drop anything else. Returns `{}` rather than `{key: undefined}` so the
 * spread above does not create present-but-undefined properties. */
function portField<K extends keyof WorkspaceRuntimeEntry>(
  entry: Record<string, unknown>,
  key: K,
): Pick<WorkspaceRuntimeEntry, K> | Record<string, never> {
  const value = entry[key];
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > 65535) {
    return {};
  }
  return { [key]: value } as Pick<WorkspaceRuntimeEntry, K>;
}

/**
 * Write the registry. Atomic-ish: writes a sibling `.tmp` and renames over the target, so a process killed
 * mid-write leaves the previous good file rather than a truncated one. That is the failure the header's
 * "corrupt is discarded" rule cleans up after — this is the cheaper half of the same fix, and the two
 * together mean a killed host neither loses ports nor refuses to start.
 */
export function saveRuntimeRegistry(registry: RuntimeRegistry): void {
  const target = runtimeRegistryPath();
  const tmp = `${target}.tmp`;
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(tmp, JSON.stringify(registry, null, 2), "utf-8");
  fs.renameSync(tmp, target);
}

/** An entry for `wsId`, creating it if absent. Mutates and returns the registry — callers batch writes. */
export function entryFor(registry: RuntimeRegistry, wsId: string): WorkspaceRuntimeEntry {
  registry.workspaces[wsId] ??= {};
  return registry.workspaces[wsId];
}

/**
 * Record what a workspace actually bound. Called after a successful start, so the next restart's scan is
 * seeded by reality rather than by intent.
 */
export function recordResolvedPorts(
  registry: RuntimeRegistry,
  wsId: string,
  ports: { proxyPort: number | null; webhookPort: number | null },
): void {
  const entry = entryFor(registry, wsId);
  if (ports.proxyPort !== null) entry.resolvedProxyPort = ports.proxyPort;
  if (ports.webhookPort !== null) entry.resolvedWebhookPort = ports.webhookPort;
}
