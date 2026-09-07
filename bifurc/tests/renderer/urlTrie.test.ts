import { describe, it, expect } from "vitest";
import { urlSegments, compressNode, buildTrieItems, collectLeafIds } from "@/lib/urlTrie";
import type { TrieNode } from "@/lib/urlTrie";

describe("renderer/lib/urlTrie.ts", () => {
  describe("urlSegments()", () => {
    it("splits a URL into host + path segments", () => {
      const segs = urlSegments("http://example.com/api/v1/users");
      expect(segs).toEqual(["example.com", "api", "v1", "users"]);
    });

    it("strips trailing slash before splitting", () => {
      const segs = urlSegments("http://example.com/api/");
      expect(segs).toEqual(["example.com", "api"]);
    });

    it("returns just the host when there is no path", () => {
      const segs = urlSegments("http://example.com");
      expect(segs).toEqual(["example.com"]);
    });

    it("returns the raw string when the URL is invalid", () => {
      const segs = urlSegments("not-a-url");
      expect(segs).toEqual(["not-a-url"]);
    });
  });

  describe("compressNode()", () => {
    it("returns node as-is when it has items", () => {
      const node: TrieNode<{ id: string }> = {
        name: "api",
        children: new Map(),
        items: [{ id: "item1" }],
      };
      const result = compressNode(node);
      expect(result.name).toBe("api");
      expect(result.items).toHaveLength(1);
    });

    it("compresses a chain of single-child nodes with no items", () => {
      const grandchild: TrieNode<{ id: string }> = { name: "users", children: new Map(), items: [{ id: "u1" }] };
      const child: TrieNode<{ id: string }> = { name: "v1", children: new Map([["users", grandchild]]), items: [] };
      const root: TrieNode<{ id: string }> = { name: "api", children: new Map([["v1", child]]), items: [] };

      const result = compressNode(root);
      expect(result.name).toBe("api/v1/users");
      expect(result.items).toHaveLength(1);
    });

    it("does not compress when node has multiple children", () => {
      const c1: TrieNode<{ id: string }> = { name: "a", children: new Map(), items: [] };
      const c2: TrieNode<{ id: string }> = { name: "b", children: new Map(), items: [] };
      const root: TrieNode<{ id: string }> = { name: "root", children: new Map([["a", c1], ["b", c2]]), items: [] };

      const result = compressNode(root);
      expect(result.name).toBe("root");
      expect(result.children.size).toBe(2);
    });
  });

  describe("buildTrieItems()", () => {
    const entries = [
      { id: "r1", url: "http://api.example.com/users" },
      { id: "r2", url: "http://api.example.com/users/1" },
      { id: "r3", url: "http://other.example.com/data" },
    ];

    it("returns items and branchIds", () => {
      const { items, branchIds } = buildTrieItems(
        entries,
        (e) => e.url,
        "leaf",
        (e) => e.id,
      );
      expect(typeof items).toBe("object");
      expect(Array.isArray(branchIds)).toBe(true);
    });

    it("includes a root item", () => {
      const { items } = buildTrieItems(entries, (e) => e.url, "leaf", (e) => e.id);
      expect(items["root"]).toBeDefined();
      expect(items["root"].isFolder).toBe(true);
    });

    it("creates leaf items for each entry", () => {
      const { items } = buildTrieItems(entries, (e) => e.url, "leaf", (e) => e.id);
      expect(items["leaf-r1"]).toBeDefined();
      expect(items["leaf-r1"].isFolder).toBe(false);
      expect(items["leaf-r1"].data.item).toEqual(entries[0]);
    });

    it("handles empty entries array", () => {
      const { items, branchIds } = buildTrieItems([], (e: any) => e.url, "leaf", (e: any) => e.id);
      expect(items["root"]).toBeDefined();
      expect(items["root"].children).toHaveLength(0);
      expect(branchIds).toContain("root");
    });

    it("handles entries with invalid URLs", () => {
      const invalidEntries = [{ id: "bad1", url: "not-a-url" }];
      const { items } = buildTrieItems(invalidEntries, (e) => e.url, "leaf", (e) => e.id);
      expect(items["leaf-bad1"]).toBeDefined();
    });
  });

  describe("collectLeafIds()", () => {
    it("returns empty array for missing item", () => {
      const result = collectLeafIds("nonexistent", {});
      expect(result).toEqual([]);
    });

    it("returns leaf item id directly", () => {
      const items: any = {
        "leaf-r1": { index: "leaf-r1", isFolder: false, data: { item: { id: "r1" } } },
      };
      const result = collectLeafIds("leaf-r1", items);
      expect(result).toEqual(["r1"]);
    });

    it("recursively collects leaf ids from a folder", () => {
      const items: any = {
        "folder": { index: "folder", isFolder: true, children: ["leaf-r1", "leaf-r2"], data: { name: "f" } },
        "leaf-r1": { index: "leaf-r1", isFolder: false, data: { item: { id: "r1" } } },
        "leaf-r2": { index: "leaf-r2", isFolder: false, data: { item: { id: "r2" } } },
      };
      const result = collectLeafIds("folder", items);
      expect(result).toEqual(["r1", "r2"]);
    });

    it("handles items with no children field", () => {
      const items: any = {
        "folder": { index: "folder", isFolder: true, data: { name: "f" } },
      };
      const result = collectLeafIds("folder", items);
      expect(result).toEqual([]);
    });
  });
});
