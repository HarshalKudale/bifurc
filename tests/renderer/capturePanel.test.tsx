// @vitest-environment jsdom
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import CapturePanel from "@/panels/CapturePanel";
import type { AppConfig, RequestLogEntry } from "@/types";

// The capture history is the engine's. The panel reads it (so traffic that passed while the panel was closed
// is there), applies live entries for its own workspace only, and keeps no copy of its own.

const WS = "ws-1";
const entry = (id: string, url: string, workspaceId = WS): RequestLogEntry =>
  ({
    id, ts: 1, method: "GET", url, host: "x", status: 200, via: "proxy", target: null, durationMs: 1,
    reqHeaders: {}, reqBody: "", resHeaders: {}, resBody: "", resStatus: 200, workspaceId,
  }) as RequestLogEntry;

let emitEntry: (e: RequestLogEntry) => void;
let captureList: ReturnType<typeof vi.fn>;

beforeEach(() => {
  localStorage.clear();
  captureList = vi.fn().mockResolvedValue([entry("old", "http://api.test/while-closed")]);
  Object.defineProperty(window, "api", {
    value: {
      captureList,
      captureClear: vi.fn().mockResolvedValue({ ok: true }),
      captureRemove: vi.fn().mockResolvedValue({ ok: true, removed: 1 }),
      onLogEntry: (cb: (e: RequestLogEntry) => void) => { emitEntry = cb; return () => {}; },
      onLogChunk: () => () => {},
      getConfig: vi.fn(),
    },
    configurable: true,
    writable: true,
  });
});

function renderPanel() {
  const cfg = { activeWorkspaceId: WS, mocks: [], mockFolders: [], requests: [], requestFolders: [] } as unknown as AppConfig;
  render(
    <CapturePanel
      activeWorkspaceId={WS}
      wsConfig={cfg}
      onConfigChange={vi.fn()}
      onOpenInMocks={vi.fn()}
      onOpenInRequests={vi.fn()}
    />,
  );
}

describe("CapturePanel", () => {
  it("shows the engine's history for the workspace, including traffic from before it opened", async () => {
    renderPanel();
    expect(await screen.findByText(/while-closed/)).toBeInTheDocument();
    expect(captureList).toHaveBeenCalledWith(WS);
  });

  it("adds live entries for its workspace and ignores another workspace's", async () => {
    renderPanel();
    await screen.findByText(/while-closed/);
    act(() => {
      emitEntry(entry("mine", "http://api.test/live-mine"));
      emitEntry(entry("theirs", "http://api.test/live-theirs", "ws-2"));
    });
    await waitFor(() => expect(screen.getByText(/live-mine/)).toBeInTheDocument());
    expect(screen.queryByText(/live-theirs/)).not.toBeInTheDocument();
  });

  it("keeps no copy in localStorage", async () => {
    renderPanel();
    await screen.findByText(/while-closed/);
    expect(localStorage.length).toBe(0);
  });
});
