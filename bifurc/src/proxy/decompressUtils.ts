import * as zlib from "zlib";

/**
 * Decompress a response body buffer based on the Content-Encoding header value.
 *
 * HTTP allows stacked encodings (comma-separated), applied outermost-last.
 * We reverse the list so we peel encodings from outermost to innermost.
 *
 * Supported: gzip, x-gzip, deflate, br (brotli), zstd (Node ≥ 22), identity.
 * Unknown encodings and decompression failures return the original buffer so
 * callers always get something displayable.
 */
export function decompressBody(buf: Buffer, contentEncoding: string): Buffer {
    if (!contentEncoding || !buf.length) return buf;

    const encodings = contentEncoding
        .split(",")
        .map((e) => e.trim().toLowerCase())
        .filter(Boolean)
        .reverse(); // peel outermost first

    let result = buf;
    for (const enc of encodings) {
        try {
            if (enc === "gzip" || enc === "x-gzip") {
                result = zlib.gunzipSync(result);
            } else if (enc === "deflate") {
                // HTTP "deflate" is ambiguous: try zlib-wrapped (RFC 1950) then raw (RFC 1951)
                try {
                    result = zlib.inflateSync(result);
                } catch {
                    result = zlib.inflateRawSync(result);
                }
            } else if (enc === "br") {
                result = zlib.brotliDecompressSync(result);
            } else if (enc === "zstd") {
                // zstdDecompressSync available in Node.js ≥ 22
                const z = zlib as unknown as Record<string, (b: Buffer) => Buffer>;
                if (typeof z["zstdDecompressSync"] === "function") {
                    result = z["zstdDecompressSync"](result);
                }
            }
            // "identity" and unrecognised encodings: pass through unchanged
        } catch {
            // Return what we had before this encoding step failed
            return buf;
        }
    }
    return result;
}

/**
 * Clone a headers object with content-encoding removed.
 * Used when storing a decompressed body in the log so viewers don't try to
 * decode it a second time.
 */
export function stripContentEncoding(headers: Record<string, string>): Record<string, string> {
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(headers)) {
        if (k.toLowerCase() !== "content-encoding") out[k] = v;
    }
    return out;
}
