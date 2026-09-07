import { describe, it, expect } from "vitest";
import {
  mkRowId,
  headersToRows,
  rowsToHeaders,
  b64ToText,
  textToB64,
  tryFormat,
  statusColor,
  methodColor,
  methodBg,
  METHODS,
  MOCK_METHODS,
} from "@/lib/utils";

// Polyfill btoa/atob for node environment
import { Buffer } from "buffer";
if (!(globalThis as any).btoa) {
  (globalThis as any).btoa = (str: string) => Buffer.from(str, "binary").toString("base64");
  (globalThis as any).atob = (str: string) => Buffer.from(str, "base64").toString("binary");
}

describe("renderer/lib/utils.ts", () => {
  describe("mkRowId()", () => {
    it("returns a non-empty string", () => {
      const id = mkRowId();
      expect(typeof id).toBe("string");
      expect(id.length).toBeGreaterThan(0);
    });

    it("returns unique values on successive calls", () => {
      const ids = new Set(Array.from({ length: 10 }, () => mkRowId()));
      expect(ids.size).toBe(10);
    });
  });

  describe("headersToRows()", () => {
    it("converts a headers object to rows", () => {
      const rows = headersToRows({ "content-type": "application/json" });
      expect(rows).toHaveLength(1);
      expect(rows[0].key).toBe("content-type");
      expect(rows[0].value).toBe("application/json");
      expect(rows[0].enabled).toBe(true);
    });

    it("filters out keys in the skip set", () => {
      const skip = new Set(["host", "connection"]);
      const rows = headersToRows({ host: "example.com", "x-custom": "yes", connection: "keep-alive" }, skip);
      expect(rows).toHaveLength(1);
      expect(rows[0].key).toBe("x-custom");
    });

    it("returns all rows when skip is not provided", () => {
      const rows = headersToRows({ a: "1", b: "2" });
      expect(rows).toHaveLength(2);
    });

    it("returns empty array for empty headers", () => {
      expect(headersToRows({})).toHaveLength(0);
    });
  });

  describe("rowsToHeaders()", () => {
    it("converts enabled rows with non-empty keys to headers object", () => {
      const rows = [
        { id: "r1", enabled: true, key: "content-type", value: "text/plain" },
        { id: "r2", enabled: false, key: "x-disabled", value: "yes" },
        { id: "r3", enabled: true, key: "  ", value: "whitespace-only" },
      ];
      const result = rowsToHeaders(rows);
      expect(result["content-type"]).toBe("text/plain");
      expect(result["x-disabled"]).toBeUndefined();
      expect(Object.keys(result)).toHaveLength(1);
    });

    it("trims keys before using them", () => {
      const rows = [{ id: "r1", enabled: true, key: "  x-header  ", value: "v" }];
      const result = rowsToHeaders(rows);
      expect(result["x-header"]).toBe("v");
    });

    it("returns empty object for empty rows", () => {
      expect(rowsToHeaders([])).toEqual({});
    });
  });

  describe("b64ToText()", () => {
    it("decodes a base64 string to UTF-8 text", () => {
      const b64 = btoa("hello world");
      expect(b64ToText(b64)).toBe("hello world");
    });

    it("returns empty string for empty input", () => {
      expect(b64ToText("")).toBe("");
    });
  });

  describe("textToB64()", () => {
    it("encodes text to base64", () => {
      const b64 = textToB64("hello");
      expect(typeof b64).toBe("string");
      expect(b64.length).toBeGreaterThan(0);
    });

    it("returns empty string for whitespace-only input", () => {
      expect(textToB64("   ")).toBe("");
    });

    it("round-trips with b64ToText", () => {
      const original = "test message 123";
      expect(b64ToText(textToB64(original))).toBe(original);
    });
  });

  describe("tryFormat()", () => {
    it("pretty-prints valid JSON", () => {
      const result = tryFormat('{"a":1,"b":2}');
      expect(result).toBe(JSON.stringify({ a: 1, b: 2 }, null, 2));
    });

    it("returns input unchanged for invalid JSON", () => {
      const input = "not json at all";
      expect(tryFormat(input)).toBe(input);
    });

    it("returns input unchanged for whitespace-only string", () => {
      expect(tryFormat("  ")).toBe("  ");
    });
  });

  describe("statusColor()", () => {
    it("returns signal green for 2xx status codes", () => {
      expect(statusColor(200)).toBe("var(--c-signal)");
      expect(statusColor(201)).toBe("var(--c-signal)");
    });

    it("returns amber for 3xx status codes", () => {
      expect(statusColor(301)).toBe("var(--c-amber)");
    });

    it("returns destructive red for 4xx and 5xx status codes", () => {
      expect(statusColor(404)).toBe("var(--c-destructive)");
      expect(statusColor(500)).toBe("var(--c-destructive)");
    });
  });

  describe("methodColor()", () => {
    it("returns signal green for GET", () => {
      expect(methodColor("GET")).toBe("var(--c-signal)");
    });

    it("returns amber for POST", () => {
      expect(methodColor("POST")).toBe("var(--c-amber)");
    });

    it("returns muted foreground for unknown methods", () => {
      expect(methodColor("UNKNOWN")).toBe("var(--c-muted-foreground)");
    });

    it("is case-insensitive", () => {
      expect(methodColor("get")).toBe(methodColor("GET"));
    });
  });

  describe("methodBg()", () => {
    it("returns a background color string for GET", () => {
      const bg = methodBg("GET");
      expect(typeof bg).toBe("string");
      expect(bg).toMatch(/oklch/);
    });

    it("returns a fallback for unknown methods", () => {
      expect(methodBg("UNKNOWN")).toBe("oklch(var(--muted-foreground) / 0.13)");
    });
  });

  describe("METHODS constant", () => {
    it("includes standard HTTP methods", () => {
      expect(METHODS).toContain("GET");
      expect(METHODS).toContain("POST");
      expect(METHODS).toContain("DELETE");
    });
  });

  describe("MOCK_METHODS constant", () => {
    it("starts with *", () => {
      expect(MOCK_METHODS[0]).toBe("*");
    });

    it("includes all METHODS after *", () => {
      for (const m of METHODS) {
        expect(MOCK_METHODS).toContain(m);
      }
    });
  });
});
