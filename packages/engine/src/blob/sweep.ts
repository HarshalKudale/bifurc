/**
 * P3 work item 1 — the blob TTL sweep (`plan/04-phase-3-file-ops.md`).
 *
 * `blob.release` is the fast path out of staging and it is the client's responsibility. The sweep
 * is the safety net for when the client does not hold up its end: it crashes mid-transfer, the
 * network drops, or the user closes the window between an upload and the import that consumes it.
 * Without it a Docker volume accumulates orphaned staging directories forever — the failure is
 * silent and unbounded, which is the worst combination for something that only shows up in
 * long-running deployments.
 *
 * ## What it reclaims
 *
 * | Entry | Reclaimed when |
 * |---|---|
 * | `<blobId>/` with `meta.json` | `createdAt + ttl <= now` |
 * | `<blobId>/` without `meta.json` | the directory's mtime is older than the TTL — a `put` that died mid-write |
 * | `.staging/stage_<hex>` | mtime older than the TTL — a `createStaging()` handle nobody finished |
 * | any other file directly under the root | always; nothing legitimate lives there |
 *
 * The mtime fallback exists only for entries that have no metadata to read. For a real blob,
 * `createdAt` is authoritative and mtime is irrelevant — which is why `readBlob` refreshes the
 * metadata rather than the file's timestamps (see `store.ts`).
 *
 * ## Two rules it holds to
 *
 * **It never creates the blob root.** A sweep that ran after the data root was moved or deleted
 * would otherwise resurrect an empty directory, and "the sweep recreated it" is a confusing thing
 * to find while debugging a Docker volume.
 *
 * **It never throws.** It runs on a timer inside the engine, and a sweeper that can take the
 * process down because one directory was locked by Windows is strictly worse than a sweeper that
 * skips a pass. `sweepBlobs()` reports per-entry failures through `onError` and carries on.
 */
import * as fs from "fs";
import * as path from "path";
import { BLOB_TTL_MS } from "@bifurc/protocol";
import { BLOB_STAGING_DIR_NAME, blobRoot, readMetaFromDir } from "./store";

/**
 * How often the periodic sweeper runs when the caller does not say.
 *
 * One sixth of the TTL, so an expired blob is reclaimed within ~10 minutes of expiring rather
 * than lingering for another full hour. A sweep is a single `readdir` plus a few stats, so the
 * extra passes cost far less than a Docker volume that only ever grows.
 *
 * It lives here rather than beside the store's layout constants because `startBlobSweeper()` is
 * its only consumer — a constant belongs with the code that decides it, and `store.ts` has no
 * opinion about how often anything is swept.
 */
export const BLOB_SWEEP_INTERVAL_MS = BLOB_TTL_MS / 6;

export interface BlobSweepResult {
  /** Directories and files removed. */
  removed: number;
  /** Bytes those entries accounted for — from metadata where available, otherwise from the
   * directory's on-disk size. Approximate for the fallback case, exact for real blobs. */
  reclaimedBytes: number;
}

export interface BlobSweepOptions {
  /** Override for "now". Exists so TTL behaviour is testable without fake timers. */
  now?: number;
  /** Override the lease term. Production callers omit it; the protocol constant is the contract. */
  ttlMs?: number;
  /** Called for each entry that could not be removed. The sweep continues regardless. */
  onError?: (err: unknown, entry: string) => void;
}

/** Total size of everything under `dir`, for entries that have no metadata to trust. */
function directorySize(dir: string): number {
  let total = 0;
  const stack = [dir];
  while (stack.length > 0) {
    const current = stack.pop()!;
    let entries: fs.Dirent[];
    try {
      entries = fs.readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const full = path.join(current, entry.name);
      try {
        if (entry.isDirectory()) stack.push(full);
        else total += fs.statSync(full).size;
      } catch {
        /* raced with something else; not worth reporting */
      }
    }
  }
  return total;
}

function isOlderThan(fullPath: string, now: number, ttlMs: number): boolean {
  try {
    return fs.statSync(fullPath).mtimeMs + ttlMs <= now;
  } catch {
    // Unstattable — treat as reclaimable rather than as immortal.
    return true;
  }
}

/**
 * One pass. Returns what it reclaimed.
 *
 * A missing blob root is a no-op, not an error: `sweepBlobs()` is called on a timer from the
 * engine's lifecycle, and "there is nothing staged yet" is the normal state of a fresh install.
 */
export function sweepBlobs(opts: BlobSweepOptions = {}): BlobSweepResult {
  const now = opts.now ?? Date.now();
  const ttlMs = opts.ttlMs ?? BLOB_TTL_MS;
  const root = blobRoot();

  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return { removed: 0, reclaimedBytes: 0 };
  }

  let removed = 0;
  let reclaimedBytes = 0;

  for (const entry of entries) {
    const full = path.join(root, entry.name);

    if (entry.name === BLOB_STAGING_DIR_NAME) {
      const staging = sweepStaging(full, now, ttlMs, opts.onError);
      removed += staging.removed;
      reclaimedBytes += staging.reclaimedBytes;
      continue;
    }

    if (!entry.isDirectory()) {
      // A loose file directly under the blob root is never legitimate — every blob is a
      // directory. Reclaim it without consulting a TTL.
      try {
        const size = fs.statSync(full).size;
        fs.rmSync(full, { force: true });
        removed++;
        reclaimedBytes += size;
      } catch (err) {
        opts.onError?.(err, full);
      }
      continue;
    }

    const meta = readMetaFromDir(full);
    const expired = meta ? meta.createdAt + ttlMs <= now : isOlderThan(full, now, ttlMs);
    if (!expired) continue;

    const bytes = meta?.size ?? directorySize(full);
    try {
      fs.rmSync(full, { recursive: true, force: true });
      removed++;
      reclaimedBytes += bytes;
    } catch (err) {
      opts.onError?.(err, full);
    }
  }

  return { removed, reclaimedBytes };
}

/**
 * Clear abandoned staged files. Unlike a blob, a staged file has no metadata — its age is the
 * only signal available, and mtime is exactly the right one here.
 */
function sweepStaging(
  dir: string,
  now: number,
  ttlMs: number,
  onError: BlobSweepOptions["onError"],
): BlobSweepResult {
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return { removed: 0, reclaimedBytes: 0 };
  }

  let removed = 0;
  let reclaimedBytes = 0;
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (!isOlderThan(full, now, ttlMs)) continue;
    try {
      const size = fs.statSync(full).size;
      fs.rmSync(full, { recursive: true, force: true });
      removed++;
      reclaimedBytes += size;
    } catch (err) {
      onError?.(err, full);
    }
  }
  return { removed, reclaimedBytes };
}

export interface BlobSweeperOptions {
  /** Passed to `sweepBlobs()`. */
  sweep?: BlobSweepOptions;
  /** How often to run. Defaults to `BLOB_SWEEP_INTERVAL_MS` (a sixth of the TTL). */
  intervalMs?: number;
  /** Called when a pass could not even start — e.g. the data root is not set. */
  onError?: (err: unknown) => void;
}

/**
 * Start the periodic sweep. Returns the stop function.
 *
 * The timer is **unref'd**. The sweeper is a background chore, and a `setInterval` that keeps the
 * event loop alive would mean an engine, a CLI (P8) or a Docker container (P9) that never exits
 * unless someone remembers to stop it. With `unref()` the worst case of a missed `stop()` is a
 * stray timer that dies with the process, which is the correct failure mode for housekeeping.
 *
 * The returned function is idempotent — calling it twice is harmless, which matters because
 * `createEngine().stop()` and the shell's `before-quit` can both reach for teardown.
 */
export function startBlobSweeper(opts: BlobSweeperOptions = {}): () => void {
  const intervalMs = opts.intervalMs ?? BLOB_SWEEP_INTERVAL_MS;
  let stopped = false;

  const run = (): void => {
    if (stopped) return;
    try {
      sweepBlobs(opts.sweep);
    } catch (err) {
      // `sweepBlobs()` is itself failure-tolerant per entry; reaching here means the blob root
      // could not be resolved at all (an unset data root). Not fatal, and not silent.
      opts.onError?.(err);
    }
  };

  const timer = setInterval(run, intervalMs);
  timer.unref?.();

  return () => {
    stopped = true;
    clearInterval(timer);
  };
}
