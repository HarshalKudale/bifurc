import { describe, it, expect, beforeEach, vi } from "vitest";

// Polyfill localStorage for node environment
class MockLocalStorage {
  private _store: Record<string, string> = {};
  get length() { return Object.keys(this._store).length; }
  getItem(key: string): string | null { return this._store[key] ?? null; }
  setItem(key: string, value: string): void { this._store[key] = value; }
  removeItem(key: string): void { delete this._store[key]; }
  key(index: number): string | null { return Object.keys(this._store)[index] ?? null; }
  clear(): void { this._store = {}; }
}

const mockStorage = new MockLocalStorage();
(globalThis as any).localStorage = mockStorage;

import { saveDraft, loadDraft, clearDraft, getDraftIds } from "@/hooks/useDraftPersist";

describe("renderer/lib/useDraftPersist.ts — pure helpers", () => {
  beforeEach(() => {
    mockStorage.clear();
  });

  describe("saveDraft()", () => {
    it("saves data to localStorage under the draft prefix", () => {
      saveDraft("tab-1", { method: "GET", url: "http://example.com" });
      const raw = mockStorage.getItem("lp:draft:tab-1");
      expect(raw).not.toBeNull();
      const parsed = JSON.parse(raw!);
      expect(parsed.method).toBe("GET");
    });
  });

  describe("loadDraft()", () => {
    it("returns null when no draft exists for the tab", () => {
      expect(loadDraft("nonexistent")).toBeNull();
    });

    it("returns the saved data", () => {
      saveDraft("tab-2", { url: "http://test.com" });
      const data = loadDraft<{ url: string }>("tab-2");
      expect(data?.url).toBe("http://test.com");
    });
  });

  describe("clearDraft()", () => {
    it("removes the draft from localStorage", () => {
      saveDraft("tab-3", { value: 42 });
      clearDraft("tab-3");
      expect(loadDraft("tab-3")).toBeNull();
      expect(mockStorage.getItem("lp:draft:tab-3")).toBeNull();
    });

    it("prevents future saves for the cleared tab id", () => {
      clearDraft("tab-4");
      saveDraft("tab-4", { value: 99 });
      expect(loadDraft("tab-4")).toBeNull();
    });
  });

  describe("getDraftIds()", () => {
    it("returns empty array when no drafts exist", () => {
      expect(getDraftIds("tab")).toEqual([]);
    });

    it("returns tab ids matching the prefix", () => {
      mockStorage.setItem("lp:draft:req-abc", "{}");
      mockStorage.setItem("lp:draft:req-xyz", "{}");
      mockStorage.setItem("lp:draft:mock-1", "{}");

      const reqIds = getDraftIds("req");
      expect(reqIds).toContain("req-abc");
      expect(reqIds).toContain("req-xyz");
      expect(reqIds).not.toContain("mock-1");
    });
  });
});
