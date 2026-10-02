import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@bifurc/engine/workspace/fs/fsRead", () => ({ readAllEntities: vi.fn(() => []) }));
vi.mock("@bifurc/engine/workspace/appSettings", () => ({ loadSettings: vi.fn(() => ({ workspaces: [] })) }));

import { readAllEntities } from "@bifurc/engine/workspace/fs/fsRead";
import { loadSettings } from "@bifurc/engine/workspace/appSettings";
import { runRequestPipeline } from "@bifurc/engine/rest/runPipeline";
import { encodeRequestBody, resolveRequestBody } from "@bifurc/engine/rest/requestBody";

const WS = "ws-1";

function useEnv(vars: Record<string, string>) {
  const env = {
    id: "__global__", name: "g", createdAt: 0, workspaceId: WS,
    variables: Object.entries(vars).map(([key, value]) => ({ id: key, key, value })),
  };
  vi.mocked(readAllEntities).mockReturnValue([env] as never);
  vi.mocked(loadSettings).mockReturnValue({ workspaces: [{ id: WS, name: WS, activeEnvironmentId: "__global__" }] } as never);
}

beforeEach(() => {
  vi.mocked(readAllEntities).mockReset();
  vi.mocked(readAllEntities).mockReturnValue([] as never);
  vi.mocked(loadSettings).mockReset();
  vi.mocked(loadSettings).mockReturnValue({ workspaces: [] } as never);
});

describe("encodeRequestBody", () => {
  it("encodes a text body as UTF-8 base64 and leaves the headers", () => {
    const out = encodeRequestBody({ headers: { a: "b" }, body: "héllo" });
    expect(Buffer.from(out.bodyBase64, "base64").toString("utf8")).toBe("héllo");
    expect(out.headers).toEqual({ a: "b" });
  });

  it("sends no body for a blank text body", () => {
    expect(encodeRequestBody({ headers: {}, body: "   \n" }).bodyBase64).toBe("");
    expect(encodeRequestBody({ headers: {} }).bodyBase64).toBe("");
  });

  it("passes a base64 body through untouched, even if it would look blank as text", () => {
    const b64 = Buffer.from([0, 255, 1, 2]).toString("base64");
    expect(encodeRequestBody({ headers: {}, body: b64, bodyEncoding: "base64" }).bodyBase64).toBe(b64);
  });

  it("builds a parseable multipart payload with a boundary header and exact file bytes", () => {
    const file = Buffer.from([0, 1, 2, 0xff, 0x0d, 0x0a, 0xfe]);
    const { bodyBase64, headers } = encodeRequestBody({
      headers: { "Content-Type": "application/json", "X-Keep": "1" },
      body: "ignored",
      bodyEncoding: "base64",
      bodyParts: [
        { name: "title", kind: "text", value: "hello" },
        { name: "upload", kind: "file", value: file.toString("base64"), fileName: "a.bin", contentType: "application/x-thing" },
      ],
    });
    expect(Object.keys(headers).filter((k) => k.toLowerCase() === "content-type")).toEqual(["content-type"]);
    expect(headers["X-Keep"]).toBe("1");
    const m = /^multipart\/form-data; boundary=(.+)$/.exec(headers["content-type"]);
    expect(m).not.toBeNull();
    const boundary = m![1];
    expect(boundary.startsWith("----bifurc")).toBe(true);

    const payload = Buffer.from(bodyBase64, "base64");
    const delim = Buffer.from(`--${boundary}`);
    const text = payload.toString("latin1");
    expect(text.endsWith(`--${boundary}--\r\n`)).toBe(true);

    // split on the delimiter at byte level
    const segments: Buffer[] = [];
    let from = 0;
    for (;;) {
      const i = payload.indexOf(delim, from);
      if (i < 0) break;
      segments.push(payload.subarray(from, i));
      from = i + delim.length;
    }
    const parts = segments.slice(1); // before the first delimiter is empty
    expect(parts).toHaveLength(2);

    const split = (seg: Buffer) => {
      const sep = seg.indexOf("\r\n\r\n");
      return { head: seg.subarray(2, sep).toString("utf8"), data: seg.subarray(sep + 4, seg.length - 2) };
    };
    const p1 = split(parts[0]);
    expect(p1.head).toBe('Content-Disposition: form-data; name="title"');
    expect(p1.data.toString("utf8")).toBe("hello");
    const p2 = split(parts[1]);
    expect(p2.head).toBe('Content-Disposition: form-data; name="upload"; filename="a.bin"\r\nContent-Type: application/x-thing');
    expect(p2.data.equals(file)).toBe(true);
  });

  it("defaults a file part's content type", () => {
    const { bodyBase64 } = encodeRequestBody({
      headers: {},
      bodyParts: [{ name: "f", kind: "file", value: "AA==", fileName: "x" }],
    });
    expect(Buffer.from(bodyBase64, "base64").toString("latin1")).toContain("Content-Type: application/octet-stream");
  });
});

describe("variable resolution", () => {
  it("touches text bodies, part names, text values and file names, never base64 data", () => {
    useEnv({ v: "RESOLVED" });
    const resolved = resolveRequestBody(
      {
        body: "{{v}}",
        bodyParts: [
          { name: "n-{{v}}", kind: "text", value: "t-{{v}}" },
          { name: "f", kind: "file", value: "e3t2fX0=", fileName: "{{v}}.bin" },
        ],
      },
      { id: "e", name: "e", createdAt: 0, workspaceId: WS, variables: [{ id: "v", key: "v", value: "RESOLVED" }] },
    );
    expect(resolved.body).toBe("RESOLVED");
    expect(resolved.bodyParts).toEqual([
      { name: "n-RESOLVED", kind: "text", value: "t-RESOLVED" },
      { name: "f", kind: "file", value: "e3t2fX0=", fileName: "RESOLVED.bin" },
    ]);

    const env = { id: "e", name: "e", createdAt: 0, workspaceId: WS, variables: [{ id: "v", key: "v", value: "R" }] };
    const b64 = Buffer.from("{{v}}").toString("base64");
    expect(resolveRequestBody({ body: b64, bodyEncoding: "base64" }, env).body).toBe(b64);
  });

  it("the pipeline resolves a text body but not a base64 one, and a pre-script keeps the new fields", async () => {
    useEnv({ v: "R" });
    const b64 = Buffer.from("{{v}}").toString("base64");
    const send = vi.fn(async () => ({ status: 200, headers: {}, body: "" }));
    await runRequestPipeline({
      workspaceId: WS,
      request: { method: "POST", url: "http://x", headers: {}, body: b64, bodyEncoding: "base64", bodyParts: [{ name: "{{v}}", kind: "text", value: "{{v}}" }] },
      scripts: { pre: "lp.request.url = lp.request.url + '/v2';" },
      send,
    });
    const sent = (send.mock.calls[0] as unknown[])[0] as { body: string; bodyEncoding?: string; bodyParts?: unknown; headers: Record<string, string>; url: string };
    expect(sent.body).toBe(b64);
    expect(sent.bodyEncoding).toBe("base64");
    expect(sent.bodyParts).toEqual([{ name: "R", kind: "text", value: "R" }]);
    expect(sent.url).toBe("http://x/v2");
  });
});
