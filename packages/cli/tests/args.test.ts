import { describe, it, expect } from "vitest";
import { flag, flags, has, parseArgs } from "../src/args";

describe("parseArgs", () => {
  it("separates positionals from flags in either form", () => {
    const a = parseArgs(["mappings", "add", "api", "127.0.0.1:3000", "--label", "My API", "--data-dir=/d"]);
    expect(a.positionals).toEqual(["mappings", "add", "api", "127.0.0.1:3000"]);
    expect(flag(a, "label")).toBe("My API");
    expect(flag(a, "data-dir")).toBe("/d");
  });

  it("collects repeated flags and expands the curl-style aliases", () => {
    const a = parseArgs(["requests", "send", "--url", "http://x", "-X", "POST", "-H", "a: 1", "-H", "b: 2", "-d", "{}"]);
    expect(flag(a, "method")).toBe("POST");
    expect(flags(a, "header")).toEqual(["a: 1", "b: 2"]);
    expect(flag(a, "body")).toBe("{}");
  });

  it("treats --json and --help as switches", () => {
    const a = parseArgs(["status", "--json", "-h"]);
    expect(has(a, "json")).toBe(true);
    expect(has(a, "help")).toBe(true);
    expect(a.positionals).toEqual(["status"]);
  });

  it("keeps everything after -- as positionals", () => {
    expect(parseArgs(["call", "--", "--not-a-flag"]).positionals).toEqual(["call", "--not-a-flag"]);
  });

  it("refuses a value flag with no value", () => {
    expect(() => parseArgs(["mappings", "add", "--label"])).toThrow(/--label needs a value/);
  });
});
