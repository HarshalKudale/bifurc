import { describe, it, expect } from "vitest";
import { searchMappings } from "@/components/search/searchModules";
import type { LocalMapping } from "@/types";

// Global search read `mapping.subdomain`, a field mappings never had: results showed a blank title and a
// mapping could not be found by its domain. The renderer typecheck (then not a CI gate) had been saying so.

const mapping = (over: Partial<LocalMapping> = {}): LocalMapping =>
  ({ id: "m1", domain: "api.localhost", target: "127.0.0.1:4000", label: "", enabled: true, ...over }) as LocalMapping;

describe("searchMappings()", () => {
  it("finds a mapping by its domain and titles the result with it", () => {
    const [hit] = searchMappings("api", [mapping()]);
    expect(hit).toMatchObject({ id: "m1", title: "api.localhost" });
  });

  it("still finds a mapping by target and label", () => {
    expect(searchMappings("4000", [mapping()])).toHaveLength(1);
    expect(searchMappings("billing", [mapping({ label: "Billing" })])).toHaveLength(1);
  });
});
