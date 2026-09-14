import { beforeEach, describe, expect, it } from "vitest";
import {
  readStorage,
  readStorageRaw,
  removeStorage,
  writeStorage,
  writeStorageRaw,
} from "@/lib/storage";

class MockLocalStorage {
  private store: Record<string, string> = {};

  getItem(key: string): string | null {
    return this.store[key] ?? null;
  }

  setItem(key: string, value: string): void {
    this.store[key] = value;
  }

  removeItem(key: string): void {
    delete this.store[key];
  }

  clear(): void {
    this.store = {};
  }
}

const mockStorage = new MockLocalStorage();
(globalThis as { localStorage: MockLocalStorage }).localStorage = mockStorage;

describe("renderer/lib/storage.ts", () => {
  beforeEach(() => {
    mockStorage.clear();
  });

  it("writes and reads JSON values", () => {
    writeStorage("prefs", { open: true, panel: "requests" });

    expect(readStorage("prefs", { open: false, panel: "services" })).toEqual({
      open: true,
      panel: "requests",
    });
  });

  it("returns the fallback for malformed JSON", () => {
    writeStorageRaw("bad-json", "{");

    expect(readStorage("bad-json", { ok: false })).toEqual({ ok: false });
  });

  it("supports raw values and removals", () => {
    writeStorageRaw("theme", "terminal");
    expect(readStorageRaw("theme")).toBe("terminal");

    removeStorage("theme");
    expect(readStorageRaw("theme")).toBeNull();
  });
});
