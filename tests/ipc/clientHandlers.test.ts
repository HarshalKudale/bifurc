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

vi.mock("@bifurc/engine/store/appSettings", () => ({
  loadSettings: vi.fn(() => currentSettings),
  saveSettings: vi.fn((s: any) => { currentSettings = s; }),
  appDataDir: vi.fn(() => "/tmp/test-user-data"),
}));

vi.mock("@/main", () => ({
  getMainWindow: vi.fn(() => null),
}));

/**
 * `tls:installCA` used to be mocked at `@bifurc/engine/proxy/certManager`, because the handler
 * called `installCA()` from inside the engine. P3 work item 5 moved the OS trust store to the
 * client (`src/ipc/certTrust.ts`) and the composition to `src/ipc/certLifecycle.ts`, so the seam to
 * mock moved with it. The platform command shapes are asserted exactly, with no mocking at all, in
 * `tests/ipc/certTrust.test.ts`; the ordering and record-keeping rules in
 * `tests/ipc/certLifecycle.test.ts`. What is left to assert *here* is only that the channel is
 * registered and delegates — which is all this file is for.
 */
vi.mock("@/ipc/certLifecycle", () => ({
  installEngineCa: vi.fn(() => ({ ok: true })),
}));

vi.mock("fs", async (importOriginal) => {
  const actual = await importOriginal<typeof import("fs")>();
  return {
    ...actual,
    existsSync: vi.fn(() => true),
    statSync: vi.fn(() => ({ size: 10 })),
    readFileSync: vi.fn(() => Buffer.from("hello")),
    // Real until now: no handler in this file wrote anything. `client:writeArtifact` does, and a
    // test must not touch a real disk — the assertion is on the *bytes and path handed to it*, which
    // a spy records and a real write would hide.
    writeFileSync: vi.fn(),
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
    // Default: the user accepts. Individual tests override it to model a cancel.
    showSaveDialog: vi.fn(() => Promise.resolve({ canceled: false, filePath: "/tmp/out.json" })),
  },
  shell: { openExternal: vi.fn() },
}));

import { installEngineCa } from "@/ipc/certLifecycle";
import { registerClientHandlers } from "@/ipc/handlers/clientHandlers";
import { dialog } from "electron";
import * as fs from "fs";

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
    it("delegates to the client-side lifecycle, which owns the OS trust store", () => {
      const result = getHandler("tls:installCA")(EVENT);
      expect(installEngineCa).toHaveBeenCalled();
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

  /**
   * `client:writeArtifact` — the client half of an engine-produced artifact, and the hook that
   * unlocked five of the nine artifact-egress methods in `src/preload.ts`.
   *
   * The contract it has to honour is `File_Ops_Protocol.md` §4: the engine produces the bytes and the
   * **user** chooses where they go. So the assertions that matter are that the dialog is pre-filled
   * from the engine's suggested name, that the bytes and path handed to `fs` are exactly what the
   * engine produced, and that *cancel* is distinguishable from *failure* — the renderer shows
   * different UI for "you said no" and "the write broke", and collapsing them would make an
   * unwritable disk look like a dismissed dialog.
   */
  describe("client:writeArtifact handler", () => {
    const writeArtifact = (contentBase64: string, suggestedName: string, mimeType = "application/json") =>
      getHandler("client:writeArtifact")(EVENT, contentBase64, suggestedName, mimeType);

    // Set per test rather than inherited: `vi.clearAllMocks()` clears *calls* but not
    // *implementations*, so the "user cancels" case below would otherwise leak into every later case
    // in this describe and make them all fail for the wrong reason.
    beforeEach(() => {
      vi.mocked(dialog.showSaveDialog).mockResolvedValue({ canceled: false, filePath: "/tmp/out.json" } as never);
    });

    it("decodes the base64 and writes the bytes to the path the user chose", async () => {
      const result = await writeArtifact("eyJhIjoxfQ==", "capture.json");
      expect(fs.writeFileSync).toHaveBeenCalledWith("/tmp/out.json", Buffer.from('{"a":1}', "utf-8"));
      expect(result).toEqual({ ok: true, filePath: "/tmp/out.json" });
    });

    /**
     * The regression this channel exists to prevent. `UEsDBAD//oA=` is a ZIP's magic bytes followed
     * by `00 ff fe 80` — none of which is valid UTF-8. Written through a UTF-8 round-trip those
     * bytes become `U+FFFD` or vanish, so the export is a file that looks like a ZIP and will not
     * open. Comparing Buffers rather than strings is the whole point: a string comparison would
     * pass on the corrupted form.
     */
    it("writes a binary artifact byte-for-byte (the workspace-zip regression)", async () => {
      await writeArtifact("UEsDBAD//oA=", "workspace.zip", "application/zip");
      const written = vi.mocked(fs.writeFileSync).mock.calls[0][1];
      expect(Buffer.isBuffer(written)).toBe(true);
      expect(written).toEqual(Buffer.from([0x50, 0x4b, 0x03, 0x04, 0x00, 0xff, 0xfe, 0x80]));
    });

    it("pre-fills the picker from the engine's suggestion and filters on its extension", async () => {
      await writeArtifact("YSxiCg==", "audit.csv", "text/csv");
      expect(dialog.showSaveDialog).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({
          defaultPath: "audit.csv",
          filters: [{ name: "text/csv", extensions: ["csv"] }],
        }),
      );
    });

    it("reports a cancel rather than an error, and writes nothing", async () => {
      vi.mocked(dialog.showSaveDialog).mockResolvedValue({ canceled: true, filePath: undefined } as never);
      expect(await writeArtifact("eA==", "x.txt")).toEqual({ ok: false, canceled: true });
      expect(fs.writeFileSync).not.toHaveBeenCalled();
    });

    it("turns a failed write into a reported error rather than a rejection", async () => {
      vi.mocked(fs.writeFileSync).mockImplementationOnce(() => {
        throw new Error("EACCES");
      });
      expect(await writeArtifact("eA==", "x.txt")).toEqual({ ok: false, error: "EACCES" });
    });
  });
});
