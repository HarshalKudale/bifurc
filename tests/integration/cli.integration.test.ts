import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as http from "http";
import * as os from "os";
import * as path from "path";
import { createWorkspaceHost, type WorkspaceHost } from "@bifurc/engine/runtime/host";
import { setDataRoot, resetDataRootForTests } from "@bifurc/engine/workspace/paths";
import { loadSettings, saveSettings, setSettingsPathOverride } from "@bifurc/engine/workspace/appSettings";
import { readRuntimeInfo } from "@bifurc/engine/runtime/discovery";
import { getFreePort } from "./proxyHarness";
import { main } from "../../packages/cli/src/index";

// The CLI against a real standalone host over its socket (a named pipe on Windows): it finds the engine through
// runtime.json, opens a session, and drives the same commands the UI does.

let root: string;
let host: WorkspaceHost;
let upstream: http.Server;
let upstreamPort: number;

/** Run the CLI, returning its exit code and what it printed. */
async function cli(...argv: string[]): Promise<{ code: number; out: string; err: string }> {
  let out = "";
  let err = "";
  const o = vi.spyOn(process.stdout, "write").mockImplementation((c) => ((out += String(c)), true));
  const e = vi.spyOn(process.stderr, "write").mockImplementation((c) => ((err += String(c)), true));
  try {
    const code = await main(["--data-dir", root, ...argv]);
    return { code, out, err };
  } finally {
    o.mockRestore();
    e.mockRestore();
  }
}
const json = async <T>(...argv: string[]): Promise<T> => {
  const r = await cli(...argv, "--json");
  expect(r.err).toBe("");
  return JSON.parse(r.out) as T;
};

beforeAll(async () => {
  setSettingsPathOverride(null);
  root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-cli-"));
  upstreamPort = await getFreePort();
  upstream = http.createServer((req, res) => {
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ method: req.method, url: req.url, auth: req.headers.authorization ?? null, body }));
    });
  });
  await new Promise<void>((r) => upstream.listen(upstreamPort, "127.0.0.1", r));

  const proxyPort = await getFreePort();
  setDataRoot(root);
  saveSettings({ ...loadSettings(), forwardProxyPort: proxyPort });
  host = createWorkspaceHost({ dataDir: root, edgePort: 0, onNotice: () => undefined });
  await host.start();
}, 30000);

afterAll(async () => {
  await host?.stop();
  await new Promise<void>((r) => upstream.close(() => r()));
  resetDataRootForTests();
  try {
    fs.rmSync(root, { recursive: true, force: true });
  } catch {
    /* best-effort: Windows can still hold a handle on the workspace's git files */
  }
});

describe("bifurc CLI", () => {
  it("finds the engine through runtime.json", async () => {
    expect(readRuntimeInfo(root)?.pid).toBe(process.pid);
    const status = await json<{ engine: { pid: number }; server: { running: boolean } }>("status");
    expect(status.engine.pid).toBe(process.pid);
    expect(status.server.running).toBe(true);
  });

  it("adds and lists mappings, and reports the engine's refusal", async () => {
    const added = await cli("mappings", "add", "api", `127.0.0.1:${upstreamPort}`, "--label", "API");
    expect(added.code).toBe(0);
    expect(added.out).toContain("api.localhost");

    const list = await json<{ domain: string; label: string }[]>("mappings", "list");
    expect(list).toEqual([expect.objectContaining({ domain: "api.localhost", label: "API" })]);

    const dup = await cli("mappings", "add", "API.localhost", "127.0.0.1:1");
    expect(dup.code).toBe(1);
    expect(dup.err).toMatch(/BAD_REQUEST: api\.localhost is already mapped/);
  });

  it("lists local services", async () => {
    const services = await json<{ port: number }[]>("services", "list");
    expect(services.some((s) => s.port === upstreamPort)).toBe(true);
  });

  it("saves a request and sends it by id — the engine loads it, headers and body included", async () => {
    await cli("requests", "add", "--url", `http://127.0.0.1:${upstreamPort}/hello`, "--method", "POST",
      "--name", "Hello", "-H", "Authorization: Bearer abc", "--body", '{"x":1}');
    const [saved] = await json<{ id: string; name: string }[]>("requests", "list");
    expect(saved.name).toBe("Hello");

    const res = await json<{ status: number; body: string }>("requests", "send", saved.id);
    expect(res.status).toBe(200);
    expect(JSON.parse(Buffer.from(res.body, "base64").toString("utf8"))).toEqual({
      method: "POST", url: "/hello", auth: "Bearer abc", body: '{"x":1}',
    });
  });

  it("sends an unsaved request as a full payload", async () => {
    const res = await json<{ status: number; body: string }>(
      "requests", "send", "--url", `http://127.0.0.1:${upstreamPort}/inline`, "-X", "PUT", "-d", "raw",
    );
    expect(JSON.parse(Buffer.from(res.body, "base64").toString("utf8"))).toMatchObject({ method: "PUT", body: "raw" });
  });

  it("passes any protocol command through `call`, and refuses unknown ones", async () => {
    expect(await json<{ running: boolean }>("call", "proxy.status")).toEqual({ running: true });
    const bad = await cli("call", "not.a.command");
    expect(bad.code).toBe(1);
    expect(bad.err).toMatch(/unknown command "not\.a\.command"/);
  });

  it("resolves variables with Global underneath, auto-selects a lone environment and keeps it when another is added", async () => {
    const cfg = await json<{ activeWorkspaceId: string }>("call", "config.get");
    const wsId = cfg.activeWorkspaceId;
    const make = (name: string, vars: Record<string, string>) =>
      json<{ id: string }>("call", "entity.create", JSON.stringify({
        kind: "environments", workspaceId: wsId,
        entity: { name, variables: Object.entries(vars).map(([key, value], i) => ({ id: `${name}${i}`, key, value })) },
      }));

    await cli("requests", "add", "--url", "http://127.0.0.1:{{port}}/{{who}}", "--name", "Vars");
    const found = await json<{ id: string; name: string }[]>("requests", "list");
    const reqId = found.find((r) => r.name === "Vars")!.id;
    const sendPath = async (): Promise<string> => {
      const res = await json<{ status: number; body: string }>("requests", "send", reqId);
      return JSON.parse(Buffer.from(res.body, "base64").toString("utf8")).url;
    };

    // Global holds the port; the lone extra environment is selected without anyone choosing it.
    await json("call", "entity.update", JSON.stringify({
      kind: "environments",
      entity: { id: "__global__", name: "Global", workspaceId: wsId, createdAt: 0,
        variables: [{ id: "g1", key: "port", value: String(upstreamPort) }, { id: "g2", key: "who", value: "global" }] },
    }));
    const dev = await make("dev", { who: "dev" });
    expect(await sendPath()).toBe("/dev");

    // A second environment must not silently deselect the first.
    const stage = await make("stage", { who: "stage" });
    expect(await sendPath()).toBe("/dev");
    const listed = await json<{ id: string; active: string }[]>("env", "list");
    expect(listed.find((e) => e.id === dev.id)?.active).toBe("*");
    expect(listed.find((e) => e.id === "__global__")?.active).toBe("base");

    // Switching is engine state, shared with every client, by name.
    expect((await cli("env", "use", "stage")).code).toBe(0);
    expect(await sendPath()).toBe("/stage");
    expect(loadSettings().workspaces.find((w) => w.id === wsId)?.activeEnvironmentId).toBe(stage.id);
    expect((await cli("env", "use", "Global")).code).toBe(1);
  });

  it("stops the engine gracefully, and then says none is running", async () => {
    const stopped = await cli("engine", "stop");
    expect(stopped.code).toBe(0);
    expect(readRuntimeInfo(root)).toBeNull();
    const after = await cli("status", "--no-start");
    expect(after.code).toBe(1);
    expect(after.err).toMatch(/No engine is running/);
  });
});
