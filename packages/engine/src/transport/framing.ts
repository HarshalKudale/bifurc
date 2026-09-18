/**
 * P4 work item 2, step 2 — length-prefixed JSON framing.
 *
 * `plan/05` orders `stdio` second precisely because it "validates the framing": no ports, no auth, no
 * TLS — so if the frame codec is wrong, that is the only thing that can be wrong. The same codec then
 * carries `socket` and (over a WebSocket binary message) `ws`, so it is worth getting right once.
 *
 * ## The format
 *
 * ```
 *   +--------+--------+--------+--------+-------------------------------+
 *   |         uint32 big-endian         |  UTF-8 JSON, exactly N bytes  |
 *   +--------+--------+--------+--------+-------------------------------+
 * ```
 *
 * A length prefix rather than newline-delimited JSON, for one concrete reason: **JSON is not
 * newline-safe.** `JSON.stringify` escapes `\n` inside strings, so newline framing *mostly* works —
 * but a payload containing a literal U+2028/U+2029, or any bug in an upstream serializer, silently
 * splits one message into two and the stream never resynchronises. A length prefix is self-describing
 * and cannot desynchronise.
 *
 * ## Two guards that are not optional
 *
 * 1. **`MAX_FRAME_BYTES`.** The length prefix is attacker-controlled the moment this runs over a
 *    socket, so `readUInt32BE()` must never be trusted as an allocation size. Without a cap, a peer
 *    sends `0xFFFFFFFF` and the engine reserves 4 GiB before receiving a single byte. The decoder
 *    reports `oversized_frame` and **stops** rather than buffering.
 * 2. **An error callback instead of a throw.** `push()` is called from a stream's `data` handler;
 *    throwing there unwinds into the stream machinery and produces an unhandled `error` event on a
 *    socket nobody is watching. Failures are reported to the caller, which decides what to do.
 *
 * This is the frame-level half of `plan/05` work item 5's "Request size" row; the blob layer (P3)
 * enforces the payload-level half.
 */
import { EngineError } from "@bifurc/protocol";

/**
 * The largest single frame this codec will accept, in bytes.
 *
 * 8 MiB, chosen against the two things that actually travel in a frame rather than picked round:
 * `blob.put` sends a chunk of at most `BLOB_CHUNK_BYTES` of base64, and base64 inflates by 4/3 — so
 * the cap has to clear that with room for the surrounding JSON. It is deliberately *not* the blob
 * size limit: blobs are chunked precisely so that no single frame has to carry a whole one.
 */
export const MAX_FRAME_BYTES = 8 * 1024 * 1024;

/** The 4-byte big-endian length prefix. */
export const FRAME_HEADER_BYTES = 4;

export type FrameErrorCode = "oversized_frame" | "invalid_json" | "not_an_object";

export class FrameError extends Error {
  constructor(
    public readonly code: FrameErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "FrameError";
  }
}

/**
 * A frame-level failure projected onto the engine's error type, so a caller only ever sees one.
 *
 * `oversized_frame` and `invalid_json` are both the **peer's** fault, and the code is preserved as
 * `BAD_REQUEST` so a caller can tell "this stream is garbage" from "this command failed" — a
 * distinction both `stdio` and `socket` need, and neither should own. Anything that is not a
 * `FrameError` is a genuine engine fault and keeps `ENGINE_ERROR`.
 *
 * Here rather than in a transport because it is a statement about *this* codec's error type. Two
 * copies of it would be two places for the `BAD_REQUEST`/`ENGINE_ERROR` split to drift, which is the
 * drift class this layer already carries too much of.
 */
export function frameFailure(err: unknown): EngineError {
  if (err instanceof FrameError) return new EngineError("BAD_REQUEST", err.message);
  return new EngineError("ENGINE_ERROR", err instanceof Error ? err.message : String(err));
}

/**
 * Encode one message as a frame.
 *
 * `Buffer.byteLength(json, "utf8")` is used rather than `json.length`: the prefix counts **bytes**,
 * and a payload with any non-ASCII character has more bytes than characters. Getting this wrong
 * desynchronises the stream for every message that follows, which is the worst kind of framing bug
 * because it works in every ASCII-only test.
 */
export function encodeFrame(message: unknown): Buffer {
  const json = JSON.stringify(message);
  if (json === undefined) {
    // `JSON.stringify(undefined)` and a bare function/symbol return `undefined`, not a string.
    throw new FrameError("invalid_json", "Cannot frame a value that JSON cannot represent.");
  }
  const body = Buffer.from(json, "utf8");
  if (body.length > MAX_FRAME_BYTES) {
    throw new FrameError(
      "oversized_frame",
      `Frame of ${body.length} bytes exceeds MAX_FRAME_BYTES (${MAX_FRAME_BYTES}).`,
    );
  }
  const header = Buffer.allocUnsafe(FRAME_HEADER_BYTES);
  header.writeUInt32BE(body.length, 0);
  return Buffer.concat([header, body]);
}

export interface FrameDecoderOptions {
  /** Called once per complete frame, in order. */
  onFrame(message: unknown): void;
  /** Called once per failure. The decoder does **not** throw out of `push()`. */
  onError(error: FrameError): void;
  /** Override for tests. Defaults to `MAX_FRAME_BYTES`. */
  maxFrameBytes?: number;
}

export interface FrameDecoder {
  /** Feed bytes. Handles partial frames and several frames in one chunk. */
  push(chunk: Buffer): void;
  /** Bytes buffered but not yet a complete frame — for diagnostics and tests. */
  readonly pendingBytes: number;
  /** True once the decoder has failed; every later `push()` is ignored. */
  readonly failed: boolean;
}

/**
 * A streaming decoder.
 *
 * Stateful and deliberately not a `Transform`: the caller owns the stream, and a `Transform` would
 * give the decoder a second, invisible lifecycle that has to be ended correctly. `stdio`'s stdin has
 * no "end" in the way a file does.
 */
export function createFrameDecoder(opts: FrameDecoderOptions): FrameDecoder {
  const max = opts.maxFrameBytes ?? MAX_FRAME_BYTES;
  /**
   * The leftover bytes from the previous chunk — the whole point of a streaming decoder.
   *
   * Annotated rather than inferred: `Buffer.alloc(0)` infers `Buffer<ArrayBuffer>`, and the `chunk`
   * parameter and `Buffer.concat()` both yield `Buffer<ArrayBufferLike>`, which is not assignable to
   * it. The explicit `Buffer` is the wide form, so all three agree.
   */
  let buffer: Buffer = Buffer.alloc(0);
  let failed = false;

  /**
   * The **single** failure path, mirroring the single-failure-path rule `Transport.request()` follows
   * for the opposite reason: there, one path is what keeps `window.api` byte-identical; here, one path
   * is what guarantees the buffer is always released.
   *
   * Releasing matters more than it looks. A rejected `invalid_json` frame can hold up to
   * `MAX_FRAME_BYTES` of a dead stream's body, and the decoder is owned by the session object, so
   * without this the bytes stay referenced until the transport is collected. Nothing will ever be
   * delivered again either way — the buffer is dead weight the moment `failed` is set.
   */
  function fail(error: FrameError): void {
    failed = true;
    buffer = Buffer.alloc(0);
    opts.onError(error);
  }

  return {
    get pendingBytes(): number {
      return buffer.length;
    },
    get failed(): boolean {
      return failed;
    },

    push(chunk: Buffer): void {
      if (failed) return;
      buffer = buffer.length === 0 ? chunk : Buffer.concat([buffer, chunk]);

      // Loop, not `if`: one chunk routinely contains several frames, and a chunk boundary routinely
      // falls in the middle of one.
      while (!failed) {
        if (buffer.length < FRAME_HEADER_BYTES) return;

        const length = buffer.readUInt32BE(0);

        // Checked *before* waiting for the body. A peer that declares 4 GiB must be rejected now,
        // not after it has sent 4 GiB.
        if (length > max) {
          fail(
            new FrameError(
              "oversized_frame",
              `Frame declares ${length} bytes, above the ${max}-byte limit. Closing the stream.`,
            ),
          );
          return;
        }

        const end = FRAME_HEADER_BYTES + length;
        if (buffer.length < end) return; // incomplete body — wait for more

        const body = buffer.subarray(FRAME_HEADER_BYTES, end);
        buffer = buffer.subarray(end);

        let parsed: unknown;
        try {
          parsed = JSON.parse(body.toString("utf8"));
        } catch (e) {
          // A JSON error is fatal for this stream, not skippable: the frame boundaries are still
          // intact (the length prefix was honest), but we cannot know what the sender meant, and
          // continuing would mean acting on a partially-understood message.
          fail(new FrameError("invalid_json", `Frame body is not valid JSON: ${(e as Error).message}`));
          return;
        }

        if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
          // Rejected here rather than at the call site so that every consumer of this codec gets the
          // same guarantee: `onFrame` only ever receives a plain object.
          fail(new FrameError("not_an_object", "Frame body is not a JSON object."));
          return;
        }

        opts.onFrame(parsed);
      }
    },
  };
}
