/**
 * P3 work item 1 — the blob TTL sweep, against a real temp data root.
 *
 * Time is supplied explicitly (`opts.now`) rather than faked, so every expiry assertion is exact
 * and readable: `now = T0 + BLOB_TTL_MS` is the boundary, and `- 1` is one millisecond short of
 * it. The one place fake timers are used is the periodic sweeper, where the *interval* is the
 * thing under test rather than the arithmetic.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { BLOB_TTL_MS } from "@bifurc/protocol";
import {
  blobRoot,
  createStaging,
  putBlob,
  readBlob,
  releaseBlob,
  stagingRoot,
} from "@bifurc/engine/blob/store";
import { BLOB_SWEEP_INTERVAL_MS, startBlobSweeper, sweepBlobs } from "@bifurc/engine/blob/sweep";
import { resetDataRootForTests, setDataRoot } from "@bifurc/engine/store/paths";

const T0 = 1_700_000_000_000;

/** Backdate an entry's mtime, which is the sweep's only signal for metadata-less entries. */
function backdate(target: string, byMs: number): void {
  const when = new Date(Date.now() - byMs);
  fs.utimesSync(target, when, when);
}

describe("blob/sweep", () => {
  let root: string;

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-blob-sweep-"));
    setDataRoot(root);
  });

  afterEach(() => {
    vi.useRealTimers();
    resetDataRootForTests();
    fs.rmSync(root, { recursive: true, force: true });
  });

  function stage(content: string, now = T0) {
    return putBlob(
      { filename: "f.json", mimeType: "application/json", size: Buffer.byteLength(content), data: Buffer.from(content).toString("base64") },
      now,
    );
  }

  const blobDir = (blobId: string) => path.join(blobRoot(), blobId);

  describe("expiry", () => {
    it("reclaims a blob whose lease has run out, and reports what it freed", () => {
      const { blobId } = stage("stale", T0);
      const res = sweepBlobs({ now: T0 + BLOB_TTL_MS });
      expect(res).toEqual({ removed: 1, reclaimedBytes: 5 });
      expect(fs.existsSync(blobDir(blobId))).toBe(false);
    });

    it("keeps a blob one millisecond short of expiry — the boundary is inclusive, not early", () => {
      const { blobId } = stage("live", T0);
      expect(sweepBlobs({ now: T0 + BLOB_TTL_MS - 1 })).toEqual({ removed: 0, reclaimedBytes: 0 });
      expect(fs.existsSync(blobDir(blobId))).toBe(true);
    });

    it("keeps a fresh blob and reclaims the stale one in the same pass", () => {
      const stale = stage("old", T0);
      const live = stage("new", T0 + BLOB_TTL_MS);

      const res = sweepBlobs({ now: T0 + BLOB_TTL_MS });
      expect(res.removed).toBe(1);
      expect(fs.existsSync(blobDir(stale.blobId))).toBe(false);
      expect(fs.existsSync(blobDir(live.blobId))).toBe(true);
    });

    it("honours a read-refreshed lease — the sliding-lease contract, end to end", () => {
      const { blobId } = stage("downloading", T0);
      // A read at the last moment pushes the lease out, so a sweep that would otherwise have
      // collected it now leaves it alone.
      readBlob({ blobId }, T0 + BLOB_TTL_MS - 1);
      expect(sweepBlobs({ now: T0 + BLOB_TTL_MS }).removed).toBe(0);
      expect(sweepBlobs({ now: T0 + BLOB_TTL_MS * 2 }).removed).toBe(1);
    });

    it("respects a custom ttl", () => {
      const { blobId } = stage("x", T0);
      expect(sweepBlobs({ now: T0 + 500, ttlMs: 1000 }).removed).toBe(0);
      expect(sweepBlobs({ now: T0 + 1000, ttlMs: 1000 }).removed).toBe(1);
      expect(fs.existsSync(blobDir(blobId))).toBe(false);
    });
  });

  describe("entries with no metadata — the crashed-put state", () => {
    it("reclaims a blob directory whose metadata was never written, once its mtime is old", () => {
      const { blobId } = stage("x", T0);
      fs.rmSync(path.join(blobDir(blobId), "meta.json"));
      backdate(blobDir(blobId), BLOB_TTL_MS + 1000);

      const res = sweepBlobs({ now: Date.now() });
      expect(res.removed).toBe(1);
      expect(fs.existsSync(blobDir(blobId))).toBe(false);
    });

    it("leaves a metadata-less directory alone while it is still young", () => {
      const { blobId } = stage("x", T0);
      fs.rmSync(path.join(blobDir(blobId), "meta.json"));
      // Fresh mtime — a `put` may well be mid-write right now.
      expect(sweepBlobs({ now: Date.now() }).removed).toBe(0);
      expect(fs.existsSync(blobDir(blobId))).toBe(true);
    });

    it("reclaims a stray file directly under the blob root immediately", () => {
      // Nothing legitimate lives at that level — every blob is a directory — so there is no
      // reason to wait for a TTL.
      fs.mkdirSync(blobRoot(), { recursive: true });
      fs.writeFileSync(path.join(blobRoot(), "leftover.tmp"), "junk");
      const res = sweepBlobs({ now: T0 });
      expect(res).toEqual({ removed: 1, reclaimedBytes: 4 });
      expect(fs.existsSync(path.join(blobRoot(), "leftover.tmp"))).toBe(false);
    });
  });

  describe("staging", () => {
    it("reclaims an abandoned staged file once it is old", () => {
      const handle = createStaging();
      fs.writeFileSync(handle.path, "half a zip");
      backdate(handle.path, BLOB_TTL_MS + 1000);

      const res = sweepBlobs({ now: Date.now() });
      expect(res.removed).toBe(1);
      expect(fs.existsSync(handle.path)).toBe(false);
    });

    it("leaves an in-flight staged file alone — a client is writing to it right now", () => {
      const handle = createStaging();
      fs.writeFileSync(handle.path, "half a zip");
      expect(sweepBlobs({ now: Date.now() }).removed).toBe(0);
      expect(fs.existsSync(handle.path)).toBe(true);
    });

    it("keeps the staging directory itself, so a live handle stays usable", () => {
      const handle = createStaging();
      fs.writeFileSync(handle.path, "abandoned half-zip");
      backdate(handle.path, BLOB_TTL_MS + 1000);
      sweepBlobs({ now: Date.now() });
      // The file is gone but the directory survives. `writeFileSync` is the assertion: it throws
      // ENOENT if the sweep removed the directory along with its contents.
      expect(fs.existsSync(stagingRoot())).toBe(true);
      fs.writeFileSync(handle.path, "written after the sweep");
      expect(handle.commit({ filename: "w.zip", mimeType: "application/zip" }, T0).blobId).toMatch(
        /^blob_/,
      );
    });

    it("does not resurrect a blob that was already released", () => {
      const released = stage("gone", T0);
      expect(releaseBlob(released.blobId)).toEqual({ ok: true });
      const abandoned = createStaging();
      fs.writeFileSync(abandoned.path, "x");
      // Real `Date.now()`: a staged file has no metadata, so its age is its mtime, and backdating
      // is measured against the real clock.
      backdate(abandoned.path, BLOB_TTL_MS + 1000);

      // Only the abandoned staged file is counted — there is nothing left of the released blob.
      expect(sweepBlobs({ now: Date.now() }).removed).toBe(1);
      expect(fs.existsSync(blobDir(released.blobId))).toBe(false);
    });
  });

  describe("robustness", () => {
    it("is a no-op on a missing blob root, and does not create it", () => {
      expect(fs.existsSync(blobRoot())).toBe(false);
      expect(sweepBlobs({ now: T0 })).toEqual({ removed: 0, reclaimedBytes: 0 });
      // A sweep that recreated the root would resurrect an empty directory after a data-root
      // move — confusing to find while debugging a Docker volume.
      expect(fs.existsSync(blobRoot())).toBe(false);
    });

    it("is a no-op on an empty blob root", () => {
      fs.mkdirSync(blobRoot(), { recursive: true });
      expect(sweepBlobs({ now: T0 })).toEqual({ removed: 0, reclaimedBytes: 0 });
    });

    it("completes a pass with a clean error channel when every entry is removable", () => {
      const stale = stage("stale", T0);
      const live = stage("live", T0 + BLOB_TTL_MS);
      const failures: string[] = [];

      const res = sweepBlobs({
        now: T0 + BLOB_TTL_MS,
        // Forcing a real removal failure would need a locked file, which is platform-dependent —
        // so this asserts the channel is silent on the happy path, and the per-entry try/catch in
        // `sweepBlobs()` is what guarantees a failure cannot abort the pass.
        onError: (_err, entry) => failures.push(entry),
      });

      expect(res.removed).toBe(1);
      expect(failures).toEqual([]);
      expect(fs.existsSync(blobDir(stale.blobId))).toBe(false);
      expect(fs.existsSync(blobDir(live.blobId))).toBe(true);
    });

    it("counts bytes from metadata for a real blob, and from disk for a metadata-less directory", () => {
      const sized = stage("1234567890", T0);
      const orphan = path.join(blobRoot(), "blob_" + "9".repeat(32));
      fs.mkdirSync(orphan, { recursive: true });
      fs.writeFileSync(path.join(orphan, "content"), "123");
      // The orphan has no metadata, so it is aged by mtime against the real clock; the real blob
      // is aged by the `createdAt` it was staged with. Both expire in this one pass.
      backdate(orphan, BLOB_TTL_MS + 1000);

      const res = sweepBlobs({ now: Date.now() });
      expect(res.removed).toBe(2);
      expect(res.reclaimedBytes).toBe(10 + 3);
      expect(fs.existsSync(blobDir(sized.blobId))).toBe(false);
      expect(fs.existsSync(orphan)).toBe(false);
    });
  });

  describe("startBlobSweeper", () => {
    it("defaults to a sixth of the TTL", () => {
      expect(BLOB_SWEEP_INTERVAL_MS).toBe(BLOB_TTL_MS / 6);
    });

    it("runs on the interval and stops when told", async () => {
      vi.useFakeTimers();
      const now = Date.now();
      const stale = stage("stale", now - BLOB_TTL_MS - 1);
      const stop = startBlobSweeper({ intervalMs: 1000 });

      // Nothing happens until the interval elapses — the sweeper does not run on construction.
      expect(fs.existsSync(blobDir(stale.blobId))).toBe(true);

      await vi.advanceTimersByTimeAsync(1000);
      expect(fs.existsSync(blobDir(stale.blobId))).toBe(false);

      stop();
      const after = stage("after-stop", now - BLOB_TTL_MS - 1);
      await vi.advanceTimersByTimeAsync(5000);
      expect(fs.existsSync(blobDir(after.blobId))).toBe(true);
    });

    it("has an idempotent stop function — teardown can be reached twice", () => {
      const stop = startBlobSweeper({ intervalMs: 1000 });
      expect(() => {
        stop();
        stop();
      }).not.toThrow();
    });

    it("reports a failure to start a pass instead of throwing on the timer", async () => {
      vi.useFakeTimers();
      const errors: unknown[] = [];
      // Unset the data root: the only way `sweepBlobs()` can fail before it looks at any entry.
      resetDataRootForTests();
      const stop = startBlobSweeper({ intervalMs: 1000, onError: (err) => errors.push(err) });

      await vi.advanceTimersByTimeAsync(1000);
      expect(errors.length).toBe(1);
      expect(String(errors[0])).toContain("Data root not initialised");
      stop();
      // Restore for `afterEach`'s cleanup.
      setDataRoot(root);
    });
  });
});
