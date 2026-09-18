/**
 * The `framing.ts` codec — the length-prefixed JSON frame format `stdio` (and, later, `socket` and
 * `ws`) carry every message in.
 *
 * This is the one piece of P4 that is genuinely transport-independent, so it is tested directly
 * rather than through a transport: a bug here would show up as three separate transport failures and
 * be diagnosed as three separate transport bugs.
 *
 * It lives outside `tests/conformance/` deliberately. The conformance suite is for the `Transport`
 * *interface* and is run once per transport; the codec has no `Transport` to run against, and adding
 * a "framing" section to the suite would run the same codec assertions once per transport for no
 * extra coverage.
 *
 * The case that matters most is the multi-byte one. `encodeFrame` must prefix the **byte** length,
 * not the character length: a codec using `json.length` passes every ASCII test ever written and
 * desynchronises the stream permanently on the first accented character — and a desynchronised
 * length-prefixed stream does not recover, because every subsequent prefix is read from the middle of
 * a previous message.
 */
import { describe, expect, it } from "vitest";
import {
  createFrameDecoder,
  encodeFrame,
  FRAME_HEADER_BYTES,
  MAX_FRAME_BYTES,
  type FrameError,
} from "../../src/transport/framing";

/** Decode a sequence of chunks, collecting frames and errors separately. */
function decodeAll(chunks: Buffer[]): { got: unknown[]; errs: FrameError[]; decoder: ReturnType<typeof createFrameDecoder> } {
  const got: unknown[] = [];
  const errs: FrameError[] = [];
  const decoder = createFrameDecoder({ onFrame: (m) => got.push(m), onError: (e) => errs.push(e) });
  for (const chunk of chunks) decoder.push(chunk);
  return { got, errs, decoder };
}

/** Build a frame with a hand-written length prefix, for cases `encodeFrame` refuses to produce. */
function rawFrame(body: string, declaredLength = Buffer.byteLength(body, "utf8")): Buffer {
  const header = Buffer.allocUnsafe(FRAME_HEADER_BYTES);
  header.writeUInt32BE(declaredLength, 0);
  return Buffer.concat([header, Buffer.from(body, "utf8")]);
}

describe("encodeFrame", () => {
  it("prefixes the BYTE length, not the character length", () => {
    // "é" is 1 char / 2 bytes, "☕" is 1 char / 3, "😀" is 2 chars / 4. If the prefix counted
    // characters, the decoder would slice mid-codepoint and the payload would come back mangled.
    const payload = { note: "café ☕ 😀 — ünïcödé" };
    const json = JSON.stringify(payload);
    const frame = encodeFrame(payload);

    expect(frame.readUInt32BE(0)).toBe(Buffer.byteLength(json, "utf8"));
    expect(Buffer.byteLength(json, "utf8")).not.toBe(json.length); // the fixture must be multi-byte
  });

  it("refuses a value JSON cannot represent", () => {
    // `JSON.stringify` returns `undefined` rather than a string for these, and writing a frame with
    // no body would desynchronise the stream.
    expect(() => encodeFrame(undefined)).toThrow(/cannot represent/i);
    expect(() => encodeFrame(() => {})).toThrow(/cannot represent/i);
  });

  it("refuses a frame over MAX_FRAME_BYTES", () => {
    expect(() => encodeFrame({ x: "a".repeat(MAX_FRAME_BYTES + 1) })).toThrow(/exceeds MAX_FRAME_BYTES/);
  });

  it("round-trips a simple object", () => {
    const { got, errs } = decodeAll([encodeFrame({ a: 1, b: "two" })]);
    expect(got).toEqual([{ a: 1, b: "two" }]);
    expect(errs).toEqual([]);
  });

  it("round-trips a multi-byte payload", () => {
    const payload = { note: "café ☕ 😀 — ünïcödé" };
    const { got, errs } = decodeAll([encodeFrame(payload)]);
    expect(got).toEqual([payload]);
    expect(errs).toEqual([]);
  });
});

describe("createFrameDecoder", () => {
  it("decodes several frames arriving in one chunk", () => {
    const chunk = Buffer.concat([encodeFrame({ n: 1 }), encodeFrame({ n: 2 }), encodeFrame({ n: 3 })]);
    const { got } = decodeAll([chunk]);
    expect(got).toEqual([{ n: 1 }, { n: 2 }, { n: 3 }]);
  });

  it("decodes a frame delivered one byte at a time", () => {
    // The worst-case fragmentation: every boundary falls in the middle of something.
    const frame = encodeFrame({ note: "café ☕" });
    const chunks = Array.from({ length: frame.length }, (_, i) => frame.subarray(i, i + 1));
    const { got, errs } = decodeAll(chunks);
    expect(got).toEqual([{ note: "café ☕" }]);
    expect(errs).toEqual([]);
  });

  it("decodes a frame whose header is split across chunks", () => {
    const frame = encodeFrame({ ok: true });
    const { got, errs } = decodeAll([frame.subarray(0, 2), frame.subarray(2, 5), frame.subarray(5)]);
    expect(got).toEqual([{ ok: true }]);
    expect(errs).toEqual([]);
  });

  it("keeps frame order when a chunk ends mid-frame", () => {
    const a = encodeFrame({ n: 1 });
    const b = encodeFrame({ n: 2 });
    const joined = Buffer.concat([a, b]);
    const { got, errs } = decodeAll([joined.subarray(0, a.length + 3), joined.subarray(a.length + 3)]);
    expect(got).toEqual([{ n: 1 }, { n: 2 }]);
    expect(errs).toEqual([]);
  });

  it("reports pendingBytes for an incomplete frame, and 0 once drained", () => {
    const frame = encodeFrame({ partial: true });
    const { got, decoder } = decodeAll([frame.subarray(0, frame.length - 3)]);
    expect(got).toHaveLength(0);
    expect(decoder.pendingBytes).toBe(frame.length - 3);
    decoder.push(frame.subarray(frame.length - 3));
    expect(decoder.pendingBytes).toBe(0);
  });

  it("rejects an oversized frame BEFORE buffering its body", () => {
    // The length prefix is attacker-controlled the moment this runs over a socket. Trusting it as an
    // allocation size means a peer sends 0xFFFFFFFF and the engine reserves 4 GiB before receiving a
    // single byte of body — so the check has to happen on the header alone.
    const header = Buffer.allocUnsafe(FRAME_HEADER_BYTES);
    header.writeUInt32BE(0xffffffff, 0);
    const { got, errs, decoder } = decodeAll([header]);

    expect(got).toHaveLength(0);
    expect(errs).toHaveLength(1);
    expect(errs[0]!.code).toBe("oversized_frame");
    expect(decoder.failed).toBe(true);
    expect(decoder.pendingBytes).toBe(0);
  });

  it("rejects a frame one byte over a custom limit", () => {
    // Exercises the `maxFrameBytes` override, so the boundary is verified rather than assumed.
    const body = JSON.stringify({ x: "y" });
    const errors: FrameError[] = [];
    const decoder = createFrameDecoder({
      onFrame: () => {
        throw new Error("must not deliver");
      },
      onError: (e) => errors.push(e),
      maxFrameBytes: Buffer.byteLength(body, "utf8") - 1,
    });
    decoder.push(rawFrame(body));
    expect(errors[0]!.code).toBe("oversized_frame");
    expect(decoder.failed).toBe(true);
  });

  it("rejects invalid JSON and stops", () => {
    const { got, errs, decoder } = decodeAll([rawFrame("{not json")]);
    expect(got).toHaveLength(0);
    expect(errs[0]!.code).toBe("invalid_json");
    expect(decoder.failed).toBe(true);
  });

  it("rejects a body that is not a JSON object", () => {
    // Rejected at the codec so every consumer gets the same guarantee: `onFrame` only ever receives
    // a plain object, and no call site has to re-check.
    for (const bad of ["[]", "null", "42", '"a string"']) {
      const { got, errs } = decodeAll([rawFrame(bad)]);
      expect(got, `${bad} must not be delivered`).toHaveLength(0);
      expect(errs[0]!.code, `${bad} must report not_an_object`).toBe("not_an_object");
    }
  });

  it("releases the buffer on failure, including a large rejected body", () => {
    // The oversized case only ever retains 4 bytes, so asserting on it alone would pass even if the
    // decoder never released anything. This is the case that matters: a rejected frame can hold up to
    // MAX_FRAME_BYTES of a dead stream's body, and the decoder is owned by the session object, so it
    // outlives the stream that produced those bytes.
    const bigBody = "{".repeat(64 * 1024); // honest length prefix, invalid JSON
    const first = decodeAll([rawFrame(bigBody)]);
    expect(first.errs[0]!.code).toBe("invalid_json");
    expect(first.decoder.pendingBytes).toBe(0);

    const second = decodeAll([rawFrame("[1,2,3]")]);
    expect(second.errs[0]!.code).toBe("not_an_object");
    expect(second.decoder.pendingBytes).toBe(0);
  });

  it("ignores every push after a failure", () => {
    // A failed decoder must stay failed. Continuing would mean acting on a partially-understood
    // stream, and the frames after the failure cannot be trusted to align.
    const header = Buffer.allocUnsafe(FRAME_HEADER_BYTES);
    header.writeUInt32BE(0xffffffff, 0);
    const { got, errs, decoder } = decodeAll([header]);
    expect(errs).toHaveLength(1);

    decoder.push(encodeFrame({ wouldBeFine: true }));

    expect(got).toHaveLength(0);
    expect(errs).toHaveLength(1);
  });

  it("does not throw out of push(), so a stream's data handler cannot be unwound", () => {
    // `push()` is called from a stream `data` handler. Throwing there unwinds into the stream
    // machinery and produces an unhandled `error` event on a socket nobody is watching.
    const decoder = createFrameDecoder({ onFrame: () => {}, onError: () => {} });
    expect(() => decoder.push(rawFrame("{not json"))).not.toThrow();
    expect(() => decoder.push(Buffer.alloc(0))).not.toThrow();
  });
});
