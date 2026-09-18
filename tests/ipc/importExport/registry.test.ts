import { describe, it, expect } from "vitest";
import {
    getAllFormats,
    getFormats,
    getEntry,
    getExporter,
    getImporter,
    getPathExporter,
    getPathImporter,
    registerFormat,
} from "@bifurc/engine/importExport/registry";
import type { EntityKind } from "@bifurc/engine/importExport/types";

/**
 * The registry is the single source of truth for what the Import/Export dialog
 * offers. Three failure modes are invisible until a user clicks the button:
 *
 *   1. a format declares `supportsExport: true` but was never given an exporter
 *      (the dialog offers it, then returns "No exporter for ..."),
 *   2. an `EntityKind` is added to the type union but no format is registered
 *      for it (the panel renders an empty format dropdown), and
 *   3. a format's implementation is filed in the wrong slot — `workspace-zip` is
 *      path-shaped and every other format is content-shaped, and a mix-up there
 *      compiles fine and only fails when someone exports a real workspace.
 *
 * All three are pure data-consistency bugs, so they are cheap to pin down here.
 *
 * The registry has **two** shapes per direction since P3 work item 2:
 * `exporter`/`importer` take and return content, and `pathExporter`/`pathImporter`
 * take a path. The assertions below accept either, and then pin down exactly which
 * format is allowed to use the path-shaped pair.
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
                // A format is served either content-shaped (`exporter`, 33 of 34 files) or
                // path-shaped (`pathExporter`, `workspace-zip` alone). Requiring `exporter`
                // unconditionally would fail on the one documented exception; requiring neither
                // would let a format that promises export but has no implementation through.
                const exporter = getExporter(kind as EntityKind, f.id);
                const pathExporter = getPathExporter(kind as EntityKind, f.id);
                expect(
                    exporter ?? pathExporter,
                    `${kind}/${f.id} claims supportsExport but has neither an exporter nor a pathExporter`,
                ).toBeDefined();
                expect(typeof (exporter ?? pathExporter)!.run).toBe("function");
            }
        }
    });

    it("provides an importer for every format that claims import support", () => {
        for (const [kind, formats] of Object.entries(getAllFormats())) {
            for (const f of formats) {
                if (!f.supportsImport) continue;
                const importer = getImporter(kind as EntityKind, f.id);
                const pathImporter = getPathImporter(kind as EntityKind, f.id);
                expect(
                    importer ?? pathImporter,
                    `${kind}/${f.id} claims supportsImport but has neither an importer nor a pathImporter`,
                ).toBeDefined();
                expect(typeof (importer ?? pathImporter)!.run).toBe("function");
                expect(typeof (importer ?? pathImporter)!.preflight).toBe("function");
            }
        }
    });

    it("does not expose an exporter/importer for a format that does not claim support", () => {
        for (const [kind, formats] of Object.entries(getAllFormats())) {
            for (const f of formats) {
                if (!f.supportsExport) {
                    expect(getExporter(kind as EntityKind, f.id)).toBeUndefined();
                    expect(getPathExporter(kind as EntityKind, f.id)).toBeUndefined();
                }
                if (!f.supportsImport) {
                    expect(getImporter(kind as EntityKind, f.id)).toBeUndefined();
                    expect(getPathImporter(kind as EntityKind, f.id)).toBeUndefined();
                }
            }
        }
    });

    it("serves workspace-zip through the path-shaped slots and nothing else", () => {
        // `workspace-zip` is the one genuinely non-mechanical case (P3 work item 2): `archiver`
        // pipes into a WriteStream and `unzipper.Open.file()` rejects a buffer, so neither half can
        // honour the content-shaped interface. The risk this guards is a future edit "fixing" it
        // into `exporter`/`importer` — which would compile, then fail at runtime on a real archive.
        expect(getPathExporter("workspace", "workspace-zip")).toBeDefined();
        expect(getPathImporter("workspace", "workspace-zip")).toBeDefined();
        expect(getExporter("workspace", "workspace-zip")).toBeUndefined();
        expect(getImporter("workspace", "workspace-zip")).toBeUndefined();
        // Its sibling in the same kind IS content-shaped, so the split is per-format, not per-kind.
        expect(getExporter("workspace", "workspace-json")).toBeDefined();
        expect(getPathExporter("workspace", "workspace-json")).toBeUndefined();
    });

    it("exposes no path-shaped slot outside workspace-zip", () => {
        for (const [kind, formats] of Object.entries(getAllFormats())) {
            for (const f of formats) {
                if (kind === "workspace" && f.id === "workspace-zip") continue;
                expect(
                    getPathExporter(kind as EntityKind, f.id),
                    `${kind}/${f.id} has a pathExporter but is not the documented exception`,
                ).toBeUndefined();
                expect(
                    getPathImporter(kind as EntityKind, f.id),
                    `${kind}/${f.id} has a pathImporter but is not the documented exception`,
                ).toBeUndefined();
            }
        }
    });

    it("resolves the whole entry, which is what the command layer branches on", () => {
        // `export.create` / `import.preflight` / `import.commit` use `getEntry()` rather than the
        // narrowed getters, because they must decide inline-vs-staging *before* calling anything.
        // `getEntry` is therefore load-bearing for every one of the 34 formats, not just the zip.
        const zip = getEntry("workspace", "workspace-zip");
        expect(zip?.pathExporter).toBeDefined();
        expect(zip?.exporter).toBeUndefined();

        const json = getEntry("environments", "environments-json");
        expect(json?.exporter).toBeDefined();
        expect(json?.importer).toBeDefined();
        expect(json?.pathExporter).toBeUndefined();

        expect(getEntry("workspace", "no-such-format")).toBeUndefined();
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
        expect(getPathExporter("not-a-kind" as EntityKind, "workspace-zip")).toBeUndefined();
        expect(getPathImporter("not-a-kind" as EntityKind, "workspace-zip")).toBeUndefined();
        expect(getEntry("not-a-kind" as EntityKind, "mocks-json")).toBeUndefined();
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
