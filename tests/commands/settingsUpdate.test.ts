import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

// `settings.update` against a real `app.json` and the real protocol schema. Only the three listeners are
// stubbed: what is under test is *which* of them a patch moves, not the sockets themselves.

const listeners = vi.hoisted(() => ({
  proxyRunning: true,
  proxyError: null as string | null,
  webhookRunning: false,
}));

vi.mock("@bifurc/engine/proxy/server", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@bifurc/engine/proxy/server")>()),
  isRunning: vi.fn(() => listeners.proxyRunning),
  getServerError: vi.fn(() => listeners.proxyError),
  startServer: vi.fn(),
  reloadConfig: vi.fn(),
}));
vi.mock("@bifurc/engine/reverseproxy/edge", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@bifurc/engine/reverseproxy/edge")>()),
  isEdgeRunning: vi.fn(() => true),
  restartEdge: vi.fn(),
}));
vi.mock("@bifurc/engine/webhooks/webhookServer", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@bifurc/engine/webhooks/webhookServer")>()),
  isWebhookServerRunning: vi.fn(() => listeners.webhookRunning),
  startWebhookServer: vi.fn(),
}));

import { CommandRegistry } from "@bifurc/engine/kernel/registry";
import { EngineEventBus } from "@bifurc/engine/kernel/eventBus";
import { registerConfigCommands } from "@bifurc/engine/workspace/entities/configCommands";
import { loadSettings, saveSettings, setSettingsPathOverride } from "@bifurc/engine/workspace/appSettings";
import { startServer } from "@bifurc/engine/proxy/server";
import { restartEdge } from "@bifurc/engine/reverseproxy/edge";
import { startWebhookServer } from "@bifurc/engine/webhooks/webhookServer";

describe("settings.update", () => {
  let tmpDir: string;
  let registry: CommandRegistry;
  let bus: EngineEventBus;

  const update = (patch: Record<string, unknown>) => registry.invoke("settings.update", patch, { bus });

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-settings-update-"));
    setSettingsPathOverride(path.join(tmpDir, "app.json"));
    saveSettings({
      ...loadSettings(),
      port: 80,
      forwardProxyPort: 8080,
      webhookPort: 9101,
      zoomLevel: 2,
      workspaces: [{ id: "ws1", name: "One", activeEnvironmentId: null }],
      activeWorkspaceId: "ws1",
    });
    Object.assign(listeners, { proxyRunning: true, proxyError: null, webhookRunning: false });
    vi.clearAllMocks();
    registry = new CommandRegistry();
    bus = new EngineEventBus();
    registerConfigCommands(registry);
  });

  afterEach(() => {
    setSettingsPathOverride(null);
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes only the fields in the patch", () => {
    expect(update({ minimizeToTray: true })).toEqual({ ok: true });
    const s = loadSettings();
    expect(s.minimizeToTray).toBe(true);
    expect(s).toMatchObject({ port: 80, forwardProxyPort: 8080, zoomLevel: 2, activeWorkspaceId: "ws1" });
    expect(s.workspaces.map((w) => w.id)).toEqual(["ws1"]);
  });

  it("moves no listener for a change that is not a listener setting", () => {
    update({ minimizeToTray: true });
    expect(restartEdge).not.toHaveBeenCalled();
    expect(startServer).not.toHaveBeenCalled();
    expect(startWebhookServer).not.toHaveBeenCalled();
  });

  it("moves the edge, and only the edge, when its port changes", () => {
    update({ port: 8000 });
    expect(restartEdge).toHaveBeenCalledWith(8000);
    expect(startServer).not.toHaveBeenCalled();
  });

  it("moves a running forward proxy to its new port", () => {
    update({ forwardProxyPort: 8081 });
    expect(startServer).toHaveBeenCalledWith(8081);
    expect(restartEdge).not.toHaveBeenCalled();
  });

  it("leaves a forward proxy the user stopped stopped, and persists its new port", () => {
    listeners.proxyRunning = false;
    update({ forwardProxyPort: 8081 });
    expect(startServer).not.toHaveBeenCalled();
    expect(loadSettings().forwardProxyPort).toBe(8081);
  });

  it("retries a forward proxy whose bind failed, since a new port is how that is fixed", () => {
    listeners.proxyRunning = false;
    listeners.proxyError = "EADDRINUSE";
    update({ forwardProxyPort: 8081 });
    expect(startServer).toHaveBeenCalledWith(8081);
  });

  it("restarts a running forward proxy when TLS changes", () => {
    update({ tlsEnabled: true });
    expect(startServer).toHaveBeenCalledWith(8080);
  });

  it("moves the webhook receiver only when it is running", () => {
    update({ webhookPort: 9200 });
    expect(startWebhookServer).not.toHaveBeenCalled();
    listeners.webhookRunning = true;
    update({ webhookPort: 9300 });
    expect(startWebhookServer).toHaveBeenCalledWith(9300);
  });

  it("emits settings.changed", () => {
    const seen = vi.fn();
    bus.onTyped("settings.changed", seen);
    update({ minimizeToTray: false });
    expect(seen).toHaveBeenCalledTimes(1);
  });

  it("rejects an out-of-range port and an unknown field without writing", () => {
    expect(() => update({ port: 70000 })).toThrow(/Invalid payload/);
    expect(() => update({ workspaces: [] })).toThrow(/Invalid payload/);
    expect(loadSettings().port).toBe(80);
  });
});
