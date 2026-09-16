/**
 * P3 work item 1 — the four `blob.*` commands bound to the store.
 *
 * The point of these tests is the *seam*: that the frozen protocol schemas in `@bifurc/protocol`
 * validate a payload before the store sees it, and that a valid payload reaches the real store.
 * A fresh `CommandRegistry` is constructed per test rather than using the process-wide
 * `commandRegistry` singleton, because `register()` is strict about double registration and the
 * singleton is shared with every other suite in the run.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { CommandRegistry, type CommandContext } from "@bifurc/engine/commands/registry";
import { registerBlobCommands } from "@bifurc/engine/blob/commands";
import { blobRoot, putBlob, statBlob } from "@bifurc/engine/blob/store";
import { resetDataRootForTests, setDataRoot } from "@bifurc/engine/store/paths";

const b64 = (input: string): string => Buffer.from(input, "utf-8").toString("base64");

describe("blob/commands", () => {
  let root: string;
  let registry: CommandRegistry;
  let ctx: CommandContext;

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-blob-cmd-"));
    setDataRoot(root);
    registry = new CommandRegistry();
    // The blob handlers do not touch the bus, but the context is part of the registry's contract.
    ctx = { bus: {} as CommandContext["bus"] };
  });

  afterEach(() => {
    resetDataRootForTests();
    fs.rmSync(root, { recursive: true, force: true });
  });

  it("registers exactly the four blob commands", () => {
    registerBlobCommands(registry);
    expect(registry.list().sort()).toEqual(["blob.put", "blob.read", "blob.release", "blob.stat"]);
  });

  it("refuses a second registration, so two implementations cannot compete for one wire name", () => {
    registerBlobCommands(registry);
    expect(() => registerBlobCommands(registry)).toThrow(/already registered/);
  });

  it("stages a payload through blob.put and reports it back through blob.stat", () => {
    registerBlobCommands(registry);
    const content = JSON.stringify({ schema: "lp-environments-v1", environments: [] });

    const put = registry.invoke(
      "blob.put",
      {
        filename: "environments-export.json",
        mimeType: "application/json",
        size: Buffer.byteLength(content),
        data: b64(content),
      },
      ctx,
    ) as { blobId: string; sha256: string };

    expect(put.blobId).toMatch(/^blob_/);
    expect(statBlob(put.blobId).size).toBe(Buffer.byteLength(content));

    const stat = registry.invoke("blob.stat", { blobId: put.blobId }, ctx) as {
      filename: string;
      mimeType: string;
      sha256: string;
    };
    expect(stat.filename).toBe("environments-export.json");
    expect(stat.mimeType).toBe("application/json");
    expect(stat.sha256).toBe(put.sha256);
  });

  it("reads a slice through blob.read, terminated by eof", () => {
    registerBlobCommands(registry);
    const { blobId } = putBlob({
      filename: "f.bin",
      mimeType: "application/octet-stream",
      size: 6,
      data: b64("abcdef"),
    });

    const read = registry.invoke("blob.read", { blobId, offset: 2, length: 3 }, ctx) as {
      data: string;
      eof: boolean;
    };
    expect(Buffer.from(read.data, "base64").toString()).toBe("cde");
    expect(read.eof).toBe(false);

    const tail = registry.invoke("blob.read", { blobId, offset: 6 }, ctx) as { data: string; eof: boolean };
    expect(tail).toEqual({ data: "", eof: true });
  });

  it("releases through blob.release", () => {
    registerBlobCommands(registry);
    const { blobId } = putBlob({
      filename: "f.json",
      mimeType: "application/json",
      size: 2,
      data: b64("{}"),
    });

    expect(registry.invoke("blob.release", { blobId }, ctx)).toEqual({ ok: true });
    expect(registry.invoke("blob.release", { blobId }, ctx)).toEqual({ ok: false });
    expect(fs.existsSync(path.join(blobRoot(), blobId))).toBe(false);
  });

  describe("the protocol schema is the gate", () => {
    beforeEach(() => registerBlobCommands(registry));

    it("rejects a blob.put with no filename, before the store is reached", () => {
      expect(() =>
        registry.invoke("blob.put", { mimeType: "application/json", size: 2, data: b64("{}") }, ctx),
      ).toThrow();
      // Nothing was staged — the payload never got past the schema.
      expect(fs.existsSync(blobRoot())).toBe(false);
    });

    it("rejects a negative size at the schema, which is stricter than the store needs to be", () => {
      expect(() =>
        registry.invoke(
          "blob.put",
          { filename: "f.json", mimeType: "application/json", size: -1, data: b64("{}") },
          ctx,
        ),
      ).toThrow();
    });

    it("rejects an unknown property — the schemas are strict", () => {
      expect(() =>
        registry.invoke(
          "blob.put",
          { filename: "f.json", mimeType: "application/json", size: 2, data: b64("{}"), filePath: "/tmp/x" },
          ctx,
        ),
      ).toThrow();
    });

    it("rejects a blob.read with an unknown property", () => {
      expect(() => registry.invoke("blob.read", { blobId: "blob_x", path: "/etc/passwd" }, ctx)).toThrow();
    });

    it("rejects an unregistered command outright", () => {
      expect(() => registry.invoke("blob.nope" as never, {}, ctx)).toThrow();
    });
  });

  it("propagates a store error rather than swallowing it, so a transport can classify it", () => {
    registerBlobCommands(registry);
    let thrown: unknown;
    try {
      registry.invoke(
        "blob.put",
        { filename: "f.json", mimeType: "application/json", size: 10, data: b64("{}") },
        ctx,
      );
    } catch (err) {
      thrown = err;
    }
    // The declared size and the payload disagree — the store's integrity check, surfaced.
    expect((thrown as { code?: string }).code).toBe("blob-size-mismatch");
  });
});
