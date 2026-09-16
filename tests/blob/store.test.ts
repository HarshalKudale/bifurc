/**
 * P3 work item 1 — the blob store, against a real temp data root.
 *
 * Real directories, real files, real hashes. The store's entire reason for existing is that it
 * touches the filesystem in a specific way — a real file per blob, under the data root, with
 * metadata written last — so mocking `fs` would test the mock rather than the contract. The two
 * constraints that come from real callers are asserted directly: the content is a **file**
 * (`unzipper.Open.file()` rejects a buffer) and the root is **under `dataDir()`** (or Docker
 * loses in-flight transfers on restart).
 *
 * Time is passed in explicitly (`now`) rather than faked, which is why the store takes it as a
 * parameter: TTL assertions stay exact and no fake timers are needed.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import * as crypto from "crypto";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { BLOB_MAX_INGRESS_BYTES, BLOB_READ_CHUNK_BYTES, BLOB_TTL_MS } from "@bifurc/protocol";
import {
  BlobError,
  assertIngressWithinLimit,
  blobContentPath,
  blobRoot,
  createStaging,
  putBlob,
  readBlob,
  releaseBlob,
  stagingRoot,
  statBlob,
} from "@bifurc/engine/blob/store";
import { dataDir, resetDataRootForTests, setDataRoot } from "@bifurc/engine/store/paths";

const b64 = (input: string | Buffer): string =>
  (typeof input === "string" ? Buffer.from(input, "utf-8") : input).toString("base64");
const sha256 = (input: Buffer): string => crypto.createHash("sha256").update(input).digest("hex");

const T0 = 1_700_000_000_000;

describe("blob/store", () => {
  let root: string;

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-blob-store-"));
    setDataRoot(root);
  });

  afterEach(() => {
    resetDataRootForTests();
    fs.rmSync(root, { recursive: true, force: true });
  });

  /** The common case: stage a string and hand back the id. */
  function stage(content: string, filename = "mocks-export.json", mimeType = "application/json", now = T0) {
    return putBlob(
      { filename, mimeType, size: Buffer.byteLength(content), data: b64(content) },
      now,
    );
  }

  describe("layout", () => {
    it("puts the blob root under dataDir() — the Docker-volume requirement", () => {
      expect(blobRoot()).toBe(path.join(dataDir(), "blobs"));
      // Not merely "somewhere sensible": a root outside the data root is a blob that vanishes
      // when the container restarts.
      expect(path.relative(dataDir(), blobRoot())).toBe("blobs");
      expect(path.isAbsolute(blobRoot())).toBe(true);
    });

    it("throws loudly when the data root was never set, rather than writing to the cwd", () => {
      resetDataRootForTests();
      expect(() => blobRoot()).toThrow(/Data root not initialised/);
    });

    it("stages each blob as a real file, with metadata beside it", () => {
      const { blobId } = stage("hello");
      const content = path.join(blobRoot(), blobId, "content");

      expect(fs.statSync(content).isFile()).toBe(true);
      expect(fs.readFileSync(content, "utf-8")).toBe("hello");
      expect(fs.existsSync(path.join(blobRoot(), blobId, "meta.json"))).toBe(true);
    });

    it("does not create the blob root until something is staged", () => {
      expect(fs.existsSync(blobRoot())).toBe(false);
      stage("x");
      expect(fs.existsSync(blobRoot())).toBe(true);
    });

    it("exposes the content path the zip exporter/importer needs", () => {
      const { blobId } = stage("zip me");
      expect(blobContentPath(blobId)).toBe(path.join(blobRoot(), blobId, "content"));
    });
  });

  describe("blob.put", () => {
    it("returns a blobId and the SHA-256 of the bytes", () => {
      const bytes = Buffer.from("mocks-export.json");
      const res = putBlob({
        filename: "mocks-export.json",
        mimeType: "application/json",
        size: bytes.length,
        data: bytes.toString("base64"),
      });

      expect(res.blobId).toMatch(/^blob_[0-9a-f]{32}$/);
      expect(res.sha256).toBe(sha256(bytes));
    });

    it("round-trips through stat and read", () => {
      const content = JSON.stringify({ schema: "lp-environments-v1", environments: [] });
      const { blobId, sha256: digest } = stage(content);

      const stat = statBlob(blobId, T0);
      expect(stat).toEqual({
        size: Buffer.byteLength(content),
        mimeType: "application/json",
        sha256: digest,
        filename: "mocks-export.json",
        ttlRemainingMs: BLOB_TTL_MS,
      });

      const read = readBlob({ blobId }, T0);
      expect(Buffer.from(read.data, "base64").toString("utf-8")).toBe(content);
      expect(read.eof).toBe(true);
    });

    it("keeps the client-supplied filename verbatim — no derivation from a path", () => {
      // The engine must never learn, or invent, a client-side path. `plan/04` work item 2 fixes
      // `environments-dotenv.ts:55`, which derives a name by splitting a path; the store's part of
      // that fix is that `filename` is simply whatever the client sent.
      const { blobId } = stage("x", "my-env-file");
      expect(statBlob(blobId, T0).filename).toBe("my-env-file");
    });

    it("handles a payload larger than one read chunk", () => {
      const content = "a".repeat(BLOB_READ_CHUNK_BYTES * 2 + 17);
      const { blobId } = stage(content);
      expect(statBlob(blobId, T0).size).toBe(Buffer.byteLength(content));
    });
  });

  describe("blob.put — the ingress limits", () => {
    it("rejects a declared size over the cap before allocating anything", () => {
      let thrown: unknown;
      try {
        putBlob({
          filename: "huge.har",
          mimeType: "application/json",
          size: BLOB_MAX_INGRESS_BYTES + 1,
          data: b64("tiny"),
        });
      } catch (err) {
        thrown = err;
      }

      expect(thrown).toBeInstanceOf(BlobError);
      expect((thrown as BlobError).code).toBe("blob-too-large");
      // Nothing was written — the check runs before the store touches the disk at all.
      expect(fs.existsSync(blobRoot())).toBe(false);
    });

    it("rejects a payload whose base64 is longer than the cap could ever produce", () => {
      expect(() => assertIngressWithinLimit(10, 140_000_000)).toThrow(BlobError);
      try {
        assertIngressWithinLimit(10, 140_000_000);
      } catch (err) {
        expect((err as BlobError).code).toBe("blob-too-large");
        // The message names the *encoded* length, because that is what the client sent.
        expect((err as Error).message).toContain("base64 characters");
      }
    });

    it("accepts a payload exactly at the cap and rejects one byte over", () => {
      const maxB64 = Math.ceil(BLOB_MAX_INGRESS_BYTES / 3) * 4 + 4;
      expect(() => assertIngressWithinLimit(BLOB_MAX_INGRESS_BYTES, maxB64)).not.toThrow();
      expect(() => assertIngressWithinLimit(BLOB_MAX_INGRESS_BYTES + 1, maxB64)).toThrow(BlobError);
      expect(() => assertIngressWithinLimit(BLOB_MAX_INGRESS_BYTES, maxB64 + 1)).toThrow(BlobError);
    });

    it("rejects a declared size that is not a sane non-negative integer", () => {
      for (const bad of [-1, 1.5, NaN, Infinity]) {
        expect(() => assertIngressWithinLimit(bad, 4)).toThrow(BlobError);
      }
    });

    it("rejects a decoded length that disagrees with the declared size", () => {
      // The client said 10 bytes and sent 5. Node's decoder is lenient, so without this
      // comparison the blob would stage as a smaller — but perfectly valid-looking — payload.
      let thrown: unknown;
      try {
        putBlob({ filename: "f.json", mimeType: "application/json", size: 10, data: b64("12345") });
      } catch (err) {
        thrown = err;
      }
      expect((thrown as BlobError).code).toBe("blob-size-mismatch");
      expect((thrown as Error).message).toContain("decoded 5 bytes");
      expect(fs.existsSync(blobRoot())).toBe(false);
    });

    it("rejects a payload of unrecognised characters, which the lenient decoder would silently shorten", () => {
      expect(() =>
        putBlob({ filename: "f.json", mimeType: "application/json", size: 3, data: "!!!!" }),
      ).toThrow(/decoded 0 bytes/);
    });
  });

  describe("blob.read", () => {
    it("walks a blob in slices and reassembles it byte-for-byte", () => {
      const content = Buffer.from(
        Array.from({ length: 5000 }, (_, i) => String.fromCharCode(32 + (i % 95))).join(""),
        "utf-8",
      );
      const { blobId } = putBlob({
        filename: "blob.bin",
        mimeType: "application/octet-stream",
        size: content.length,
        data: content.toString("base64"),
      });

      const sliceSize = 1024;
      const parts: Buffer[] = [];
      let offset = 0;
      let eof = false;
      let guard = 0;
      while (!eof && guard++ < 100) {
        const res = readBlob({ blobId, offset, length: sliceSize }, T0);
        parts.push(Buffer.from(res.data, "base64"));
        offset += sliceSize;
        eof = res.eof;
      }

      const rebuilt = Buffer.concat(parts);
      expect(rebuilt.equals(content)).toBe(true);
      expect(rebuilt.length).toBe(content.length);
    });

    it("defaults the length to BLOB_READ_CHUNK_BYTES and reports eof only at the end", () => {
      const content = "b".repeat(BLOB_READ_CHUNK_BYTES + 100);
      const { blobId } = stage(content);

      const first = readBlob({ blobId }, T0);
      expect(Buffer.from(first.data, "base64").length).toBe(BLOB_READ_CHUNK_BYTES);
      expect(first.eof).toBe(false);

      const second = readBlob({ blobId, offset: BLOB_READ_CHUNK_BYTES }, T0);
      expect(Buffer.from(second.data, "base64").length).toBe(100);
      expect(second.eof).toBe(true);
    });

    it("defaults the offset to 0", () => {
      const { blobId } = stage("abcdef");
      expect(Buffer.from(readBlob({ blobId, length: 3 }, T0).data, "base64").toString()).toBe("abc");
    });

    it("returns an empty slice with eof at exactly the end — the loop terminator", () => {
      const { blobId } = stage("abc");
      const res = readBlob({ blobId, offset: 3 }, T0);
      expect(res.data).toBe("");
      expect(res.eof).toBe(true);
    });

    it("rejects an offset past the end rather than silently returning nothing", () => {
      const { blobId } = stage("abc");
      let thrown: unknown;
      try {
        readBlob({ blobId, offset: 4 }, T0);
      } catch (err) {
        thrown = err;
      }
      expect((thrown as BlobError).code).toBe("blob-invalid-offset");
    });

    it("terminates a short read at the end of the blob", () => {
      const { blobId } = stage("abc");
      const res = readBlob({ blobId, offset: 1, length: 1000 }, T0);
      expect(Buffer.from(res.data, "base64").toString()).toBe("bc");
      expect(res.eof).toBe(true);
    });
  });

  describe("the sliding lease", () => {
    it("stat reports the remainder without extending it", () => {
      const { blobId } = stage("x", "f.json", "application/json", T0);
      const stat = statBlob(blobId, T0 + 1000);
      expect(stat.ttlRemainingMs).toBe(BLOB_TTL_MS - 1000);
      // Asking twice must not have moved the clock — otherwise a polling client could hold a
      // blob alive forever by never downloading it.
      expect(statBlob(blobId, T0 + 1000).ttlRemainingMs).toBe(BLOB_TTL_MS - 1000);
    });

    it("a read extends the lease, so a slow multi-chunk download cannot be swept mid-flight", () => {
      const content = "c".repeat(BLOB_READ_CHUNK_BYTES * 2);
      const { blobId } = stage(content, "f.bin", "application/octet-stream", T0);

      // Read the first chunk one second before the lease would have expired.
      const late = T0 + BLOB_TTL_MS - 1000;
      expect(statBlob(blobId, late).ttlRemainingMs).toBe(1000);
      readBlob({ blobId }, late);
      expect(statBlob(blobId, late).ttlRemainingMs).toBe(BLOB_TTL_MS);
    });

    it("a zero-byte read at the end does not extend the lease", () => {
      const { blobId } = stage("abc", "f.json", "application/json", T0);
      const late = T0 + BLOB_TTL_MS - 1000;
      readBlob({ blobId, offset: 3 }, late);
      expect(statBlob(blobId, late).ttlRemainingMs).toBe(1000);
    });

    it("never reports a negative remainder for an already-expired blob", () => {
      const { blobId } = stage("x", "f.json", "application/json", T0);
      expect(statBlob(blobId, T0 + BLOB_TTL_MS * 5).ttlRemainingMs).toBe(0);
    });
  });

  describe("blob.release", () => {
    it("removes the whole blob directory", () => {
      const { blobId } = stage("x");
      expect(releaseBlob(blobId)).toEqual({ ok: true });
      expect(fs.existsSync(path.join(blobRoot(), blobId))).toBe(false);
    });

    it("reports ok:false when there was nothing to release, without throwing", () => {
      const { blobId } = stage("x");
      releaseBlob(blobId);
      // Already released — not an error, but not a success either. The distinction matters
      // during a retry: "I cleaned up" and "there was nothing there" are different facts.
      expect(releaseBlob(blobId)).toEqual({ ok: false });
    });

    it("makes the blob unreadable afterwards", () => {
      const { blobId } = stage("x");
      releaseBlob(blobId);
      expect(() => readBlob({ blobId }, T0)).toThrow(/No such blob/);
    });
  });

  describe("blob id validation — the path-traversal defence", () => {
    const hostile = [
      "",
      ".",
      "..",
      "../../etc/passwd",
      "..\\..\\windows\\system32",
      "/etc/passwd",
      "blobs/../../x",
      "blob_",
      "blob_short",
      "blob_" + "a".repeat(31),
      "blob_" + "a".repeat(33),
      "blob_" + "A".repeat(32), // uppercase is not what we generate
      "blob_" + "g".repeat(32), // not hex
      "blob_" + "a".repeat(31) + "/",
      "blob_00000000000000000000000000000000/../../x",
    ];

    it.each(hostile)("rejects %j on every entry point", (bad) => {
      expect(() => statBlob(bad, T0)).toThrow(BlobError);
      expect(() => readBlob({ blobId: bad }, T0)).toThrow(BlobError);
      expect(() => releaseBlob(bad)).toThrow(BlobError);
      expect(() => blobContentPath(bad)).toThrow(BlobError);
    });

    it("tags every rejection with blob-invalid-id, so a transport can classify it", () => {
      for (const bad of hostile) {
        try {
          blobContentPath(bad);
          throw new Error(`expected ${JSON.stringify(bad)} to be rejected`);
        } catch (err) {
          expect((err as BlobError).code).toBe("blob-invalid-id");
        }
      }
    });

    it("keeps every accepted path a direct child of the blob root", () => {
      const { blobId } = stage("x");
      const dir = path.dirname(blobContentPath(blobId));
      expect(path.dirname(dir)).toBe(blobRoot());
      expect(path.relative(blobRoot(), blobContentPath(blobId))).toBe(
        path.join(blobId, "content"),
      );
      expect(path.relative(blobRoot(), blobContentPath(blobId)).startsWith("..")).toBe(false);
    });
  });

  describe("not found", () => {
    it("throws blob-not-found for a well-formed id that was never staged", () => {
      const unknown = "blob_" + "0".repeat(32);
      for (const call of [
        () => statBlob(unknown, T0),
        () => readBlob({ blobId: unknown }, T0),
      ]) {
        try {
          call();
          throw new Error("expected a throw");
        } catch (err) {
          expect((err as BlobError).code).toBe("blob-not-found");
        }
      }
    });

    it("treats a blob directory with no metadata as not found — the crashed-put state", () => {
      // `meta.json` is written last precisely so this state is invisible rather than readable as
      // truncated content. The sweep collects the directory.
      const { blobId } = stage("x");
      fs.rmSync(path.join(blobRoot(), blobId, "meta.json"));
      expect(() => statBlob(blobId, T0)).toThrow(/No such blob/);
      expect(() => readBlob({ blobId }, T0)).toThrow(/No such blob/);
    });

    it("treats a blob directory with metadata but no content as not found", () => {
      // `meta.json` is written last, so this state cannot come from `putBlob` — but it is
      // reachable if something else removes the content, and reporting a size and a hash for
      // bytes that are not there is worse than reporting nothing. This is what makes
      // "metadata exists ⇒ the blob is usable" an invariant rather than a hope.
      const { blobId } = stage("x");
      fs.rmSync(path.join(blobRoot(), blobId, "content"));
      expect(() => statBlob(blobId, T0)).toThrow(/no content/);
      expect(() => readBlob({ blobId }, T0)).toThrow(/no content/);
    });

    it("treats unparseable metadata as not found", () => {
      const { blobId } = stage("x");
      fs.writeFileSync(path.join(blobRoot(), blobId, "meta.json"), "{not json", "utf-8");
      expect(() => statBlob(blobId, T0)).toThrow(/unreadable metadata/);
    });

    it("treats metadata belonging to another blob as not found", () => {
      const { blobId } = stage("x");
      const metaPath = path.join(blobRoot(), blobId, "meta.json");
      const meta = JSON.parse(fs.readFileSync(metaPath, "utf-8"));
      fs.writeFileSync(metaPath, JSON.stringify({ ...meta, blobId: "blob_" + "1".repeat(32) }), "utf-8");
      expect(() => statBlob(blobId, T0)).toThrow(/inconsistent metadata/);
    });
  });

  describe("createStaging — the streaming ingress path", () => {
    it("commits a file written by the caller, hashing it without buffering it", () => {
      const handle = createStaging();
      expect(handle.path.startsWith(stagingRoot())).toBe(true);

      const bytes = Buffer.from("PK\u0003\u0004 pretend this is a 200 MB zip");
      fs.writeFileSync(handle.path, bytes);

      const res = handle.commit({ filename: "workspace.zip", mimeType: "application/zip" }, T0);
      expect(res.sha256).toBe(sha256(bytes));
      expect(statBlob(res.blobId, T0)).toEqual({
        size: bytes.length,
        mimeType: "application/zip",
        sha256: sha256(bytes),
        filename: "workspace.zip",
        ttlRemainingMs: BLOB_TTL_MS,
      });
      expect(Buffer.from(readBlob({ blobId: res.blobId }, T0).data, "base64").equals(bytes)).toBe(true);
    });

    it("moves the staged file rather than leaving a copy behind", () => {
      const handle = createStaging();
      fs.writeFileSync(handle.path, "zip bytes");
      const { blobId } = handle.commit({ filename: "w.zip", mimeType: "application/zip" }, T0);
      expect(fs.existsSync(handle.path)).toBe(false);
      expect(fs.existsSync(path.join(blobRoot(), blobId, "content"))).toBe(true);
    });

    it("discards an abandoned staging file", () => {
      const handle = createStaging();
      fs.writeFileSync(handle.path, "abandoned");
      handle.discard();
      expect(fs.existsSync(handle.path)).toBe(false);
    });

    it("is single-use: a second commit throws, and discard after commit is a no-op", () => {
      const handle = createStaging();
      fs.writeFileSync(handle.path, "x");
      handle.commit({ filename: "a", mimeType: "text/plain" }, T0);
      expect(() => handle.commit({ filename: "b", mimeType: "text/plain" }, T0)).toThrow(/already been/);
      expect(() => handle.discard()).not.toThrow();
    });

    it("is single-use the other way round too: commit after discard throws", () => {
      const handle = createStaging();
      handle.discard();
      expect(() => handle.commit({ filename: "a", mimeType: "text/plain" }, T0)).toThrow(/already been/);
    });

    it("rejects an oversized staged file and cleans it up", () => {
      // The cap is overridden because the real one is 100 MB and the staged path exists for
      // payloads too large to hold in memory — exercising it honestly would mean writing a real
      // 100 MB file to disk. The override is the same bound the default resolves to.
      const handle = createStaging({ maxBytes: 16 });
      fs.writeFileSync(handle.path, "seventeen bytes!!");

      expect(() => handle.commit({ filename: "huge.zip", mimeType: "application/zip" }, T0)).toThrow(
        /exceeds the 16-byte ingress limit/,
      );
      expect(fs.existsSync(handle.path)).toBe(false);
    });

    it("accepts a staged file exactly at the cap", () => {
      const handle = createStaging({ maxBytes: 16 });
      fs.writeFileSync(handle.path, "sixteen bytes!!!");
      const { blobId } = handle.commit({ filename: "exact.zip", mimeType: "application/zip" }, T0);
      expect(statBlob(blobId, T0).size).toBe(16);
    });

    it("creates the staging directory under the blob root, not beside it", () => {
      const handle = createStaging();
      expect(path.dirname(handle.path)).toBe(stagingRoot());
      expect(path.relative(blobRoot(), stagingRoot())).toBe(".staging");
      handle.discard();
    });
  });
});
