import { describe, it, expect } from "vitest";
import * as zlib from "zlib";
import { decompressBody, stripContentEncoding } from "@bifurc/engine/proxy/decompressUtils";

/**
 * `decompressBody` is on the hot path of every proxied response: whatever it
 * returns is what the user sees in the inspector. It must therefore never throw
 * and never return an empty/garbled buffer when the upstream sends something
 * unexpected — the previous behaviour of returning the raw bytes is the contract
 * these tests pin down.
 */

const PLAIN = Buffer.from("hello bifurc", "utf-8");

describe("decompressBody", () => {
    it("returns the buffer untouched when there is no content-encoding", () => {
        expect(decompressBody(PLAIN, "")).toEqual(PLAIN);
    });

    it("returns the buffer untouched when the body is empty", () => {
        const empty = Buffer.alloc(0);
        expect(decompressBody(empty, "gzip")).toEqual(empty);
    });

    it("decodes gzip", () => {
        const gz = zlib.gzipSync(PLAIN);
        expect(decompressBody(gz, "gzip").toString("utf-8")).toBe("hello bifurc");
    });

    it("decodes x-gzip (legacy alias)", () => {
        const gz = zlib.gzipSync(PLAIN);
        expect(decompressBody(gz, "x-gzip").toString("utf-8")).toBe("hello bifurc");
    });

    it("decodes zlib-wrapped deflate (RFC 1950)", () => {
        const def = zlib.deflateSync(PLAIN);
        expect(decompressBody(def, "deflate").toString("utf-8")).toBe("hello bifurc");
    });

    it("decodes raw deflate (RFC 1951) as a fallback", () => {
        // Many servers send raw deflate under the name "deflate"; the HTTP spec
        // is ambiguous so the implementation must try both.
        const raw = zlib.deflateRawSync(PLAIN);
        expect(decompressBody(raw, "deflate").toString("utf-8")).toBe("hello bifurc");
    });

    it("decodes brotli", () => {
        const br = zlib.brotliCompressSync(PLAIN);
        expect(decompressBody(br, "br").toString("utf-8")).toBe("hello bifurc");
    });

    it("passes identity through unchanged", () => {
        expect(decompressBody(PLAIN, "identity")).toEqual(PLAIN);
    });

    it("passes an unknown encoding through unchanged", () => {
        expect(decompressBody(PLAIN, "made-up-encoding")).toEqual(PLAIN);
    });

    it("is case- and whitespace-insensitive", () => {
        const gz = zlib.gzipSync(PLAIN);
        expect(decompressBody(gz, "  GZIP  ").toString("utf-8")).toBe("hello bifurc");
    });

    it("peels stacked encodings outermost-first", () => {
        // "gzip, br" means gzip was applied first, brotli last (outermost).
        // The decoder must reverse the list and unwrap brotli before gzip.
        const stacked = zlib.brotliCompressSync(zlib.gzipSync(PLAIN));
        expect(decompressBody(stacked, "gzip, br").toString("utf-8")).toBe("hello bifurc");
    });

    it("ignores empty entries in the encoding list", () => {
        const gz = zlib.gzipSync(PLAIN);
        expect(decompressBody(gz, "gzip,,  ,").toString("utf-8")).toBe("hello bifurc");
    });

    it("returns the original buffer when the payload is corrupt", () => {
        const garbage = Buffer.from("this is not gzip at all", "utf-8");
        expect(decompressBody(garbage, "gzip")).toEqual(garbage);
    });

    it("returns the original buffer when the encoding does not match the payload", () => {
        const gz = zlib.gzipSync(PLAIN);
        expect(decompressBody(gz, "br")).toEqual(gz);
    });

    it("supports zstd when the Node runtime provides it", () => {
        const z = zlib as unknown as Record<string, (b: Buffer) => Buffer>;
        if (typeof z["zstdCompressSync"] !== "function") {
            // Older runtimes: the decoder must degrade gracefully, not throw.
            expect(decompressBody(PLAIN, "zstd")).toEqual(PLAIN);
            return;
        }
        const compressed = z["zstdCompressSync"](PLAIN);
        expect(decompressBody(compressed, "zstd").toString("utf-8")).toBe("hello bifurc");
    });
});

describe("stripContentEncoding", () => {
    it("removes content-encoding", () => {
        expect(stripContentEncoding({ "content-encoding": "gzip", "content-type": "text/plain" }))
            .toEqual({ "content-type": "text/plain" });
    });

    it("removes it regardless of header casing", () => {
        expect(stripContentEncoding({ "Content-Encoding": "br", "x-keep": "1" }))
            .toEqual({ "x-keep": "1" });
    });

    it("leaves other headers untouched when there is nothing to strip", () => {
        const headers = { "content-type": "application/json", "x-a": "b" };
        expect(stripContentEncoding(headers)).toEqual(headers);
    });

    it("returns a new object (does not mutate the caller's headers)", () => {
        const headers = { "content-encoding": "gzip" };
        const out = stripContentEncoding(headers);
        expect(headers).toEqual({ "content-encoding": "gzip" });
        expect(out).not.toBe(headers);
    });
});
