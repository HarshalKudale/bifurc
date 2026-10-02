import { afterEach, describe, expect, it } from "vitest";
import { isCacheDisabled, setCacheDisabled, stripCacheHeaders } from "@bifurc/engine/utils/cachePolicy";

afterEach(() => setCacheDisabled(true));

describe("cache policy", () => {
  it("is on by default", () => {
    expect(isCacheDisabled()).toBe(true);
  });

  it("strips the validators that make a server answer 304", () => {
    const headers: Record<string, string> = {
      "if-none-match": '"abc"', "if-modified-since": "Wed, 01 Jan 2025 00:00:00 GMT",
      "cache-control": "max-age=0", pragma: "no-cache", accept: "*/*",
    };
    stripCacheHeaders(headers);
    expect(headers).toEqual({ accept: "*/*" });
  });

  it("leaves the headers alone once switched off", () => {
    setCacheDisabled(false);
    const headers: Record<string, string> = { "if-none-match": '"abc"' };
    stripCacheHeaders(headers);
    expect(headers).toEqual({ "if-none-match": '"abc"' });
  });
});
