import { describe, it, expect } from "vitest";
import {
    getAllFormats,
    getFormats,
    getExporter,
    getImporter,
    registerFormat,
} from "@/ipc/importExport/registry";
import type { EntityKind } from "@/ipc/importExport/types";

/**
 * The registry is the single source of truth for what the Import/Export dialog
 * offers. Two failure modes are invisible until a user clicks the button:
 *
 *   1. a format declares `supportsExport: true` but was never given an exporter
 *      (the dialog offers it, then returns "No exporter for ..."), and
 *   2. an `EntityKind` is added to the type union but no format is registered
 *      for it (the panel renders an empty format dropdown).
 *
 * Both are pure data-consistency bugs, so they are cheap to pin down here.
 */

/** Kept in sync with `EntityKind` in types.ts — the assertion below is the guard. */
const ALL_KINDS: EntityKind[] = [
    "workspace",
    "requests",
    "mocks",
    "environments",
    "mappings",
    "proxyRules",
    "websockets",
    "webhooks",
];

describe("importExport registry", () => {
    it("registers at least one format for every entity kind", () => {
        const registered = Object.keys(getAllFormats());
        for (const kind of ALL_KINDS) {
            expect(registered, `no formats registered for kind "${kind}"`).toContain(kind);
        }
    });

    it("registers nothing beyond the known entity kinds", () => {
        // Catches a typo'd kind in a registerFormat() call, which would silently
        // create a format group that no panel can ever reach.
        const registered = Object.keys(getAllFormats()).sort();
        expect(registered).toEqual([...ALL_KINDS].sort());
    });

    it("exposes every registered format through getFormats()", () => {
        for (const kind of ALL_KINDS) {
            expect(getFormats(kind).length, `kind "${kind}" has no formats`).toBeGreaterThan(0);
        }
    });

    it("has well-formed format definitions", () => {
        for (const [kind, formats] of Object.entries(getAllFormats())) {
            for (const f of formats) {
                expect(f.id, `${kind}: empty id`).toBeTruthy();
                expect(f.label, `${kind}/${f.id}: empty label`).toBeTruthy();
                expect(f.extensions.length, `${kind}/${f.id}: no extensions`).toBeGreaterThan(0);
                expect(typeof f.supportsExport).toBe("boolean");
                expect(typeof f.supportsImport).toBe("boolean");
                for (const ext of f.extensions) {
                    expect(ext, `${kind}/${f.id}: extension must not start with a dot`).not.toMatch(/^\./);
                }
            }
        }
    });

    it("uses unique format ids within each kind", () => {
        for (const [kind, formats] of Object.entries(getAllFormats())) {
            const ids = formats.map((f) => f.id);
            expect(new Set(ids).size, `${kind} has duplicate format ids: ${ids.join(", ")}`).toBe(ids.length);
        }
    });

    it("provides an exporter for every format that claims export support", () => {
        for (const [kind, formats] of Object.entries(getAllFormats())) {
            for (const f of formats) {
                if (!f.supportsExport) continue;
                const exporter = getExporter(kind as EntityKind, f.id);
                expect(exporter, `${kind}/${f.id} claims supportsExport but has no exporter`).toBeDefined();
                expect(typeof exporter!.run).toBe("function");
            }
        }
    });

    it("provides an importer for every format that claims import support", () => {
        for (const [kind, formats] of Object.entries(getAllFormats())) {
            for (const f of formats) {
                if (!f.supportsImport) continue;
                const importer = getImporter(kind as EntityKind, f.id);
                expect(importer, `${kind}/${f.id} claims supportsImport but has no importer`).toBeDefined();
                expect(typeof importer!.run).toBe("function");
                expect(typeof importer!.preflight).toBe("function");
            }
        }
    });

    it("does not expose an exporter/importer for a format that does not claim support", () => {
        for (const [kind, formats] of Object.entries(getAllFormats())) {
            for (const f of formats) {
                if (!f.supportsExport) {
                    expect(getExporter(kind as EntityKind, f.id)).toBeUndefined();
                }
                if (!f.supportsImport) {
                    expect(getImporter(kind as EntityKind, f.id)).toBeUndefined();
                }
            }
        }
    });

    it("resolves exporters and importers by their exact format id", () => {
        // "mocks-json" is the canonical Bifurc-JSON format and is registered for
        // both directions, so it exercises both lookups.
        expect(getExporter("mocks", "mocks-json")).toBeDefined();
        expect(getImporter("mocks", "mocks-json")).toBeDefined();
        // A near-miss id must NOT resolve.
        expect(getExporter("mocks", "mocks-json-v1")).toBeUndefined();
        expect(getImporter("mocks", "MOCKS-JSON")).toBeUndefined();
    });

    it("does not leak formats across kinds", () => {
        // "requests-postman" and "mocks-postman" share a label but are distinct ids
        // registered under different kinds; looking one up under the other must fail.
        expect(getExporter("requests", "mocks-postman")).toBeUndefined();
        expect(getExporter("mocks", "requests-postman")).toBeUndefined();
    });

    it("returns empty results for unknown kinds", () => {
        expect(getFormats("not-a-kind" as EntityKind)).toEqual([]);
        expect(getExporter("not-a-kind" as EntityKind, "mocks-json")).toBeUndefined();
        expect(getImporter("not-a-kind" as EntityKind, "mocks-json")).toBeUndefined();
    });

    it("appends newly registered formats instead of replacing existing ones", () => {
        const before = getFormats("webhooks").length;
        registerFormat("webhooks", {
            definition: { id: "webhooks-test-only", label: "Test Only", extensions: ["json"], supportsExport: false, supportsImport: false },
        });
        const after = getFormats("webhooks");
        expect(after.length).toBe(before + 1);
        expect(after[after.length - 1].id).toBe("webhooks-test-only");
    });
});
