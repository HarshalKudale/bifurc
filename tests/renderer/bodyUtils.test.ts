import { describe, it, expect } from "vitest";
import {
  contentTypeToMode,
  modeToContentType,
  parseFormBody,
  serializeFormBody,
  BodyMode,
} from "@/lib/bodyUtils";

// ── contentTypeToMode ─────────────────────────────────────────────────────────

describe("contentTypeToMode()", () => {
  it("returns 'json' for application/json", () => {
    expect(contentTypeToMode("application/json")).toBe("json");
  });

  it("returns 'json' for application/json with charset suffix", () => {
    expect(contentTypeToMode("application/json; charset=utf-8")).toBe("json");
  });

  it("returns 'form' for application/x-www-form-urlencoded", () => {
    expect(contentTypeToMode("application/x-www-form-urlencoded")).toBe("form");
  });

  it("returns 'html' for text/html", () => {
    expect(contentTypeToMode("text/html")).toBe("html");
  });

  it("returns 'html' for text/html with charset", () => {
    expect(contentTypeToMode("text/html; charset=utf-8")).toBe("html");
  });

  it("returns 'xml' for text/xml", () => {
    expect(contentTypeToMode("text/xml")).toBe("xml");
  });

  it("returns 'xml' for application/xml", () => {
    expect(contentTypeToMode("application/xml")).toBe("xml");
  });

  it("returns 'xml' for application/xhtml+xml", () => {
    expect(contentTypeToMode("application/xhtml+xml")).toBe("xml");
  });

  it("returns 'text' for text/plain", () => {
    expect(contentTypeToMode("text/plain")).toBe("text");
  });

  it("returns 'image' for image content types", () => {
    expect(contentTypeToMode("image/png")).toBe("image");
    expect(contentTypeToMode("image/jpeg")).toBe("image");
    expect(contentTypeToMode("image/svg+xml")).toBe("image");
  });

  it("returns 'binary' for binary content types", () => {
    expect(contentTypeToMode("application/octet-stream")).toBe("binary");
    expect(contentTypeToMode("audio/mpeg")).toBe("binary");
    expect(contentTypeToMode("application/pdf")).toBe("binary");
  });

  it("returns 'multipart' for multipart/form-data", () => {
    expect(contentTypeToMode("multipart/form-data; boundary=----WebKitFormBoundary")).toBe("multipart");
  });

  it("returns 'text' for unrecognized content-type", () => {
    expect(contentTypeToMode("application/x-custom")).toBe("text");
  });

  it("returns 'json' for null", () => {
    expect(contentTypeToMode(null)).toBe("json");
  });

  it("returns 'json' for undefined", () => {
    expect(contentTypeToMode(undefined)).toBe("json");
  });

  it("returns 'json' for empty string", () => {
    expect(contentTypeToMode("")).toBe("json");
  });

  it("is case-insensitive", () => {
    expect(contentTypeToMode("Application/JSON")).toBe("json");
    expect(contentTypeToMode("TEXT/HTML")).toBe("html");
  });
});

// ── modeToContentType ─────────────────────────────────────────────────────────

describe("modeToContentType()", () => {
  it("maps json → application/json", () => {
    expect(modeToContentType("json")).toBe("application/json");
  });

  it("maps text → text/plain", () => {
    expect(modeToContentType("text")).toBe("text/plain");
  });

  it("maps html → text/html", () => {
    expect(modeToContentType("html")).toBe("text/html");
  });

  it("maps xml → application/xml", () => {
    expect(modeToContentType("xml")).toBe("application/xml");
  });

  it("maps form → application/x-www-form-urlencoded", () => {
    expect(modeToContentType("form")).toBe("application/x-www-form-urlencoded");
  });

  it("maps none → null", () => {
    expect(modeToContentType("none")).toBeNull();
  });

  it("round-trips through contentTypeToMode for all non-null modes", () => {
    const modes: BodyMode[] = ["json", "text", "html", "xml", "form"];
    for (const m of modes) {
      const ct = modeToContentType(m);
      expect(ct).not.toBeNull();
      expect(contentTypeToMode(ct!)).toBe(m);
    }
  });
});

// ── parseFormBody ─────────────────────────────────────────────────────────────

describe("parseFormBody()", () => {
  it("parses a simple key=value pair", () => {
    expect(parseFormBody("foo=bar")).toEqual([{ key: "foo", value: "bar" }]);
  });

  it("parses multiple pairs", () => {
    expect(parseFormBody("a=1&b=2&c=3")).toEqual([
      { key: "a", value: "1" },
      { key: "b", value: "2" },
      { key: "c", value: "3" },
    ]);
  });

  it("decodes percent-encoded keys and values", () => {
    const result = parseFormBody("first%20name=John%20Doe");
    expect(result).toEqual([{ key: "first name", value: "John Doe" }]);
  });

  it("handles value with equals sign", () => {
    const result = parseFormBody("token=abc%3Ddef");
    expect(result).toEqual([{ key: "token", value: "abc=def" }]);
  });

  it("handles pair with no value", () => {
    expect(parseFormBody("key=")).toEqual([{ key: "key", value: "" }]);
  });

  it("handles pair with no equals sign as key with empty value", () => {
    const result = parseFormBody("standalone");
    expect(result).toEqual([{ key: "standalone", value: "" }]);
  });

  it("returns empty array for empty string", () => {
    expect(parseFormBody("")).toEqual([]);
  });

  it("returns empty array for whitespace-only string", () => {
    expect(parseFormBody("   ")).toEqual([]);
  });

  it("filters out pairs with empty keys", () => {
    expect(parseFormBody("=value")).toEqual([]);
  });
});

// ── serializeFormBody ─────────────────────────────────────────────────────────

describe("serializeFormBody()", () => {
  it("serializes a single pair", () => {
    expect(serializeFormBody([{ key: "foo", value: "bar" }])).toBe("foo=bar");
  });

  it("serializes multiple pairs joined with &", () => {
    expect(serializeFormBody([
      { key: "a", value: "1" },
      { key: "b", value: "2" },
    ])).toBe("a=1&b=2");
  });

  it("percent-encodes spaces in key and value", () => {
    expect(serializeFormBody([{ key: "first name", value: "John Doe" }]))
      .toBe("first%20name=John%20Doe");
  });

  it("encodes equals sign in value", () => {
    expect(serializeFormBody([{ key: "token", value: "abc=def" }]))
      .toBe("token=abc%3Ddef");
  });

  it("handles empty value", () => {
    expect(serializeFormBody([{ key: "key", value: "" }])).toBe("key=");
  });

  it("filters out pairs with empty or whitespace-only keys", () => {
    expect(serializeFormBody([
      { key: "", value: "orphan" },
      { key: "  ", value: "spaces" },
      { key: "valid", value: "ok" },
    ])).toBe("valid=ok");
  });

  it("returns empty string for empty array", () => {
    expect(serializeFormBody([])).toBe("");
  });

  it("round-trips through parseFormBody", () => {
    const original = [
      { key: "name", value: "Alice" },
      { key: "role", value: "admin" },
      { key: "tag", value: "a&b" },
    ];
    const serialized = serializeFormBody(original);
    const parsed = parseFormBody(serialized);
    expect(parsed).toEqual(original);
  });
});
