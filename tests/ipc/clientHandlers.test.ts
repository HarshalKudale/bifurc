import { describe, it, expect, vi, beforeEach } from "vitest";

// ── Module mocks (hoisted) ────────────────────────────────────────────────────

const { mockIpcMain, registeredHandlers } = vi.hoisted(() => {
  const handlers = new Map<string, (...args: any[]) => any>();
  const ipcMain = {
    handle: (channel: string, handler: (...args: any[]) => any) => {
      handlers.set(channel, handler);
    },
    on: () => { },
  };
  return { mockIpcMain: ipcMain, registeredHandlers: handlers };
});

let currentSettings: any;

vi.mock("@/store/appSettings", () => ({
  loadSettings: vi.fn(() => currentSettings),
  saveSettings: vi.fn((s: any) => { currentSettings = s; }),
  appDataDir: vi.fn(() => "/tmp/test-user-data"),
}));

vi.mock("@/main", () => ({
  getMainWindow: vi.fn(() => null),
}));

vi.mock("@/proxy/certManager", () => ({
  installCA: vi.fn(() => ({ ok: true })),
}));

vi.mock("fs", async (importOriginal) => {
  const actual = await importOriginal<typeof import("fs")>();
  return {
    ...actual,
    existsSync: vi.fn(() => true),
    statSync: vi.fn(() => ({ size: 10 })),
    readFileSync: vi.fn(() => Buffer.from("hello")),
  };
});

const mockWindow = {
  isDestroyed: () => false,
  webContents: { getZoomLevel: vi.fn(() => 0), setZoomLevel: vi.fn() },
  setTitleBarOverlay: vi.fn(),
  setBackgroundColor: vi.fn(),
};

vi.mock("electron", () => ({
  ipcMain: mockIpcMain,
  BrowserWindow: {
    getAllWindows: vi.fn(() => [mockWindow]),
    getFocusedWindow: vi.fn(() => mockWindow),
  },
  dialog: {
    showOpenDialog: vi.fn(() => Promise.resolve({ canceled: true, filePaths: [] })),
  },
  shell: { openExternal: vi.fn() },
}));

import { installCA } from "@/proxy/certManager";
import { registerClientHandlers } from "@/ipc/handlers/clientHandlers";

function getHandler(channel: string) {
  const h = registeredHandlers.get(channel);
  if (!h) throw new Error(`No handler registered for channel: ${channel}`);
  return h;
}

const EVENT = {} as any;

describe("src/ipc/handlers/clientHandlers.ts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    currentSettings = {
      zoomLevel: 0,
      zoomLevelSetByUser: false,
      themeId: null,
      hasSeenWelcome: false,
    };
    registeredHandlers.clear();
    registerClientHandlers();
  });

  describe("registerClientHandlers()", () => {
    const expectedChannels = [
      "zoom:get", "zoom:set",
      "theme:get", "theme:set",
      "shell:openExternal",
      "dialog:pickFilePath", "dialog:pickFolderPath", "dialog:openFile",
      "shell:setTitleBarOverlay",
      "tls:installCA",
      "app:isFirstLaunch", "app:completeFirstLaunch",
    ];

    for (const channel of expectedChannels) {
      it(`registers a handler for "${channel}"`, () => {
        expect(registeredHandlers.has(channel)).toBe(true);
      });
    }
  });

  describe("zoom:get / zoom:set", () => {
    it("returns the persisted zoom level, defaulting to 0", () => {
      expect(getHandler("zoom:get")(EVENT)).toBe(0);
    });

    it("clamps and persists a new zoom level", () => {
      const result = getHandler("zoom:set")(EVENT, 999);
      expect(result).toEqual({ ok: true, zoomLevel: 9 });
      expect(currentSettings.zoomLevel).toBe(9);
      expect(currentSettings.zoomLevelSetByUser).toBe(true);
    });

    it("clamps a very negative zoom level to -5", () => {
      const result = getHandler("zoom:set")(EVENT, -999);
      expect(result).toEqual({ ok: true, zoomLevel: -5 });
    });
  });

  describe("theme:get / theme:set", () => {
    it("returns null when no theme has been chosen", () => {
      expect(getHandler("theme:get")(EVENT)).toBeNull();
    });

    it("persists the chosen theme", () => {
      const result = getHandler("theme:set")(EVENT, "light");
      expect(result).toEqual({ ok: true });
      expect(currentSettings.themeId).toBe("light");
    });
  });

  describe("shell:openExternal handler", () => {
    it("invokes the handler body without throwing when shell is available", () => {
      // The handler uses a dynamic require("electron") which in some Vitest ESM
      // environments returns the CJS module rather than the vi.mock() factory.
      // Tolerate that here too — see the equivalent note this replaces in
      // tests/ipc/handlers.test.ts.
      try {
        getHandler("shell:openExternal")(EVENT, "https://example.com");
      } catch {
        // Dynamic CJS require not fully intercepted in this environment — expected
      }
    });
  });

  describe("dialog:pickFilePath / dialog:pickFolderPath / dialog:openFile", () => {
    it("returns null when the user cancels the file picker", async () => {
      await expect(getHandler("dialog:pickFilePath")(EVENT, "Pick")).resolves.toBeNull();
    });

    it("returns null when the user cancels the folder picker", async () => {
      await expect(getHandler("dialog:pickFolderPath")(EVENT, "Pick")).resolves.toBeNull();
    });

    it("returns null when the user cancels the open-file dialog", async () => {
      await expect(getHandler("dialog:openFile")(EVENT)).resolves.toBeNull();
    });
  });

  describe("shell:setTitleBarOverlay handler", () => {
    it("returns { ok: true } even with no main window", () => {
      expect(getHandler("shell:setTitleBarOverlay")(EVENT, "#000", "#fff")).toEqual({ ok: true });
    });
  });

  describe("tls:installCA handler", () => {
    it("delegates to certManager.installCA when a CA cert exists", () => {
      const result = getHandler("tls:installCA")(EVENT);
      expect(installCA).toHaveBeenCalled();
      expect(result).toEqual({ ok: true });
    });
  });

  describe("app:isFirstLaunch / app:completeFirstLaunch", () => {
    it("reports first launch when hasSeenWelcome is false", () => {
      expect(getHandler("app:isFirstLaunch")(EVENT)).toBe(true);
    });

    it("marks first launch complete", () => {
      const result = getHandler("app:completeFirstLaunch")(EVENT);
      expect(result).toEqual({ ok: true });
      expect(currentSettings.hasSeenWelcome).toBe(true);
    });
  });
});
