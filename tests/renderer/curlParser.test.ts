import { describe, it, expect } from "vitest";
import { parseCurl, SKIP_CURL_HEADERS } from "@/lib/curlParser";

describe("renderer/lib/curlParser.ts", () => {
  describe("SKIP_CURL_HEADERS", () => {
    it("contains common headers to skip", () => {
      expect(SKIP_CURL_HEADERS.has("host")).toBe(true);
      expect(SKIP_CURL_HEADERS.has("content-length")).toBe(true);
      expect(SKIP_CURL_HEADERS.has("connection")).toBe(true);
    });
  });

  describe("parseCurl()", () => {
    it("parses a basic GET request", () => {
      const result = parseCurl("curl https://example.com/api");
      expect(result.url).toBe("https://example.com/api");
      expect(result.method).toBe("GET");
      expect(result.body).toBe("");
    });

    it("parses explicit method with -X flag", () => {
      const result = parseCurl("curl -X DELETE https://example.com/item/1");
      expect(result.method).toBe("DELETE");
    });

    it("parses explicit method with --request flag", () => {
      const result = parseCurl("curl --request PUT https://example.com/item");
      expect(result.method).toBe("PUT");
    });

    it("parses headers with -H flag", () => {
      const result = parseCurl(`curl -H "Content-Type: application/json" https://example.com`);
      expect(result.headers["content-type"]).toBe("application/json");
    });

    it("parses headers with --header flag", () => {
      const result = parseCurl(`curl --header "Accept: text/plain" https://example.com`);
      expect(result.headers["accept"]).toBe("text/plain");
    });

    it("parses body with -d flag", () => {
      const result = parseCurl(`curl -X POST -d '{"key":"val"}' https://example.com`);
      expect(result.body).toBe('{"key":"val"}');
    });

    it("parses body with --data-raw flag", () => {
      const result = parseCurl(`curl --data-raw 'hello' https://example.com`);
      expect(result.body).toBe("hello");
    });

    it("parses body with --data-binary flag", () => {
      const result = parseCurl(`curl --data-binary 'bytes' https://example.com`);
      expect(result.body).toBe("bytes");
    });

    it("parses body with --data-ascii flag", () => {
      const result = parseCurl(`curl --data-ascii 'text' https://example.com`);
      expect(result.body).toBe("text");
    });

    it("parses URL via --url flag", () => {
      const result = parseCurl(`curl --url https://api.example.com/v1`);
      expect(result.url).toBe("https://api.example.com/v1");
    });

    it("infers POST when body is present without -X", () => {
      const result = parseCurl(`curl -d 'data' https://example.com`);
      expect(result.method).toBe("POST");
    });

    it("handles multiline curl with line continuations", () => {
      const result = parseCurl(
        "curl -X POST \\\n  -H 'Content-Type: application/json' \\\n  https://example.com"
      );
      expect(result.method).toBe("POST");
      expect(result.headers["content-type"]).toBe("application/json");
    });

    it("handles double-quoted strings with escape sequences", () => {
      // The tokenizer strips backslash and takes the following char literally
      const result = parseCurl(`curl -d "line1\\nline2" https://example.com`);
      expect(result.body).toBe("line1nline2");
    });

    it("handles cookie with -b flag", () => {
      const result = parseCurl(`curl -b "session=abc" https://example.com`);
      expect(result.headers["cookie"]).toBe("session=abc");
    });

    it("appends to existing cookie header with -b", () => {
      const result = parseCurl(`curl -b "a=1" -b "b=2" https://example.com`);
      expect(result.headers["cookie"]).toBe("a=1; b=2");
    });

    it("ignores unknown flags", () => {
      const result = parseCurl(`curl -s -L https://example.com/api`);
      expect(result.url).toBe("https://example.com/api");
    });

    it("returns empty url when no url token found", () => {
      const result = parseCurl(`curl -X GET`);
      expect(result.url).toBe("");
    });

    it("handles single-quoted strings in tokenizer", () => {
      const result = parseCurl(`curl -H 'Authorization: Bearer token' https://example.com`);
      expect(result.headers["authorization"]).toBe("Bearer token");
    });
  });
});
