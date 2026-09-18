import type {
  EntityKind,
  FormatDefinition,
  FormatsMap,
  ExportResult,
  PathExportResult,
  PreflightResult,
  ImportResult,
  CollisionStrategy,
  ImportSource,
} from "./types";

// ─────────────────────────────────────────────────────────────────────────────
// The content-shaped interface — 32 of the 34 files
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Renders a workspace's entities into a string. Takes no destination: the engine decides whether
 * the result goes out inline or into a blob (`export.create`), and the client decides where it
 * lands on its own disk (`File_Ops_Protocol.md` §4).
 */
export interface ExporterFn {
  run(wsId: string): Promise<ExportResult>;
}

/**
 * `preflight` is a dry run against the same bytes `run` will consume, which is why both take the
 * `ImportSource` rather than a path: with a blob the client uploads **once** and the engine reads
 * the staged copy twice, instead of the old model where `preflight` read the file and `run` read it
 * again from the same path (`File_Ops_Protocol.md` §2.3).
 */
export interface ImporterFn {
  preflight(wsId: string, source: ImportSource): PreflightResult;
  run(wsId: string, source: ImportSource, strategy: CollisionStrategy): Promise<ImportResult>;
}

// ─────────────────────────────────────────────────────────────────────────────
// The path-shaped exception — `workspace-zip`, both directions
// ─────────────────────────────────────────────────────────────────────────────

/**
 * `workspace-zip` cannot conform to the interface above, and it is not a matter of taste:
 *
 *  - the **exporter** pipes `archiver` into a `WriteStream` and never holds the archive in memory,
 *    so it needs a destination to stream into — it cannot return content;
 *  - the **importer** calls `unzipper.Open.file()`, which rejects a buffer outright.
 *
 * So the pair keeps a path-based signature. The paths are engine-local (a staging handle from
 * `blob/store.createStaging()` for export, `blobContentPath()` for import) and never cross the
 * wire, so this is not the leak §8 describes — but it *is* an exception, and it is confined to
 * these two types plus the two registry slots below so that "the importer interface takes content"
 * stays true everywhere else.
 *
 * `plan/04` work item 2 sequences these two files **last**, after the 32 mechanical ones have
 * proven the new interface, precisely because they are the only genuinely non-mechanical case.
 */
export interface PathExporterFn {
  run(wsId: string, destPath: string): Promise<PathExportResult>;
}

export interface PathImporterFn {
  preflight(wsId: string, sourcePath: string): PreflightResult;
  run(wsId: string, sourcePath: string, strategy: CollisionStrategy): Promise<ImportResult>;
}

export interface FormatEntry {
  definition: FormatDefinition;
  exporter?: ExporterFn;
  importer?: ImporterFn;
  /** Only `workspace-zip` uses these. See `PathExporterFn`. */
  pathExporter?: PathExporterFn;
  pathImporter?: PathImporterFn;
}

type Registry = Map<EntityKind, FormatEntry[]>;

const registry: Registry = new Map();

export function registerFormat(kind: EntityKind, entry: FormatEntry): void {
  if (!registry.has(kind)) registry.set(kind, []);
  registry.get(kind)!.push(entry);
}

export function getFormats(kind: EntityKind): FormatDefinition[] {
  return (registry.get(kind) ?? []).map((e) => e.definition);
}

export function getAllFormats(): FormatsMap {
  const out: FormatsMap = {};
  for (const [kind, entries] of registry.entries()) {
    out[kind] = entries.map((e) => e.definition);
  }
  return out;
}

/**
 * The whole entry, for callers that must branch on *how* a format is served.
 *
 * `export.create` / `import.preflight` / `import.commit` need exactly that: a content-shaped
 * exporter is called with no destination and its result is turned into an inline payload or a blob,
 * whereas `workspace-zip`'s needs a staging path handed to it first. Keeping that decision here
 * means the command layer never has to know which format is which.
 */
export function getEntry(kind: EntityKind, formatId: string): FormatEntry | undefined {
  return registry.get(kind)?.find((e) => e.definition.id === formatId);
}

export function getExporter(kind: EntityKind, formatId: string): ExporterFn | undefined {
  return getEntry(kind, formatId)?.exporter;
}

export function getImporter(kind: EntityKind, formatId: string): ImporterFn | undefined {
  return getEntry(kind, formatId)?.importer;
}

export function getPathExporter(kind: EntityKind, formatId: string): PathExporterFn | undefined {
  return getEntry(kind, formatId)?.pathExporter;
}

export function getPathImporter(kind: EntityKind, formatId: string): PathImporterFn | undefined {
  return getEntry(kind, formatId)?.pathImporter;
}

// ── Register all built-in formats ─────────────────────────────────────────

import * as workspaceJsonExp from "./exporters/workspace-json";
import * as workspaceZipExp  from "./exporters/workspace-zip";
import * as workspaceJsonImp from "./importers/workspace-json";
import * as workspaceZipImp  from "./importers/workspace-zip";

import * as reqsPostmanExp    from "./exporters/requests-postman";
import * as reqsOpenApiExp    from "./exporters/requests-openapi";
import * as reqsCurlExp       from "./exporters/requests-curl";
import * as reqsHarExp        from "./exporters/requests-har";
import * as reqsInsomniaExp   from "./exporters/requests-insomnia";
import * as reqsPostmanImp    from "./importers/requests-postman";
import * as reqsOpenApiImp    from "./importers/requests-openapi";
import * as reqsCurlImp       from "./importers/requests-curl";
import * as reqsHarImp        from "./importers/requests-har";
import * as reqsInsomniaImp   from "./importers/requests-insomnia";

import * as mocksPostmanExp   from "./exporters/mocks-postman";
import * as mocksJsonExp      from "./exporters/mocks-json";
import * as mocksWireMockExp  from "./exporters/mocks-wiremock";
import * as mocksPostmanImp   from "./importers/mocks-postman";
import * as mocksJsonImp      from "./importers/mocks-json";
import * as mocksWireMockImp  from "./importers/mocks-wiremock";

import * as envsJsonExp       from "./exporters/environments-json";
import * as envsPostmanExp    from "./exporters/environments-postman";
import * as envsDotenvExp     from "./exporters/environments-dotenv";
import * as envsJsonImp       from "./importers/environments-json";
import * as envsPostmanImp    from "./importers/environments-postman";
import * as envsDotenvImp     from "./importers/environments-dotenv";

import * as mappingsJsonExp   from "./exporters/mappings-json";
import * as mappingsJsonImp   from "./importers/mappings-json";

import * as proxyJsonExp      from "./exporters/proxyrules-json";
import * as proxyJsonImp      from "./importers/proxyrules-json";

import * as wsJsonExp         from "./exporters/websockets-json";
import * as wsJsonImp         from "./importers/websockets-json";

import * as hooksJsonExp      from "./exporters/webhooks-json";
import * as hooksJsonImp      from "./importers/webhooks-json";

// Workspace
registerFormat("workspace", { definition: { id: "workspace-json", label: "Bifurc JSON", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: workspaceJsonExp, importer: workspaceJsonImp });
// `workspace-zip` is the one format that does not use `exporter`/`importer` — see `PathExporterFn`.
registerFormat("workspace", { definition: { id: "workspace-zip", label: "ZIP Archive", extensions: ["zip"], supportsExport: true, supportsImport: true }, pathExporter: workspaceZipExp, pathImporter: workspaceZipImp });

// Requests
registerFormat("requests", { definition: { id: "requests-postman", label: "Postman Collection v2.1", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: reqsPostmanExp, importer: reqsPostmanImp });
registerFormat("requests", { definition: { id: "requests-openapi", label: "OpenAPI 3.x", extensions: ["json", "yaml", "yml"], supportsExport: true, supportsImport: true }, exporter: reqsOpenApiExp, importer: reqsOpenApiImp });
registerFormat("requests", { definition: { id: "requests-curl", label: "cURL Commands", extensions: ["sh", "txt", "curl"], supportsExport: true, supportsImport: true }, exporter: reqsCurlExp, importer: reqsCurlImp });
registerFormat("requests", { definition: { id: "requests-har", label: "HAR 1.2", extensions: ["har", "json"], supportsExport: true, supportsImport: true }, exporter: reqsHarExp, importer: reqsHarImp });
registerFormat("requests", { definition: { id: "requests-insomnia", label: "Insomnia v4", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: reqsInsomniaExp, importer: reqsInsomniaImp });

// Mocks
registerFormat("mocks", { definition: { id: "mocks-postman", label: "Postman Collection v2.1", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: mocksPostmanExp, importer: mocksPostmanImp });
registerFormat("mocks", { definition: { id: "mocks-json", label: "Bifurc JSON", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: mocksJsonExp, importer: mocksJsonImp });
registerFormat("mocks", { definition: { id: "mocks-wiremock", label: "WireMock Stubs", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: mocksWireMockExp, importer: mocksWireMockImp });

// Environments
registerFormat("environments", { definition: { id: "environments-json", label: "Bifurc JSON", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: envsJsonExp, importer: envsJsonImp });
registerFormat("environments", { definition: { id: "environments-postman", label: "Postman Environment", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: envsPostmanExp, importer: envsPostmanImp });
registerFormat("environments", { definition: { id: "environments-dotenv", label: "dotenv (.env)", extensions: ["env", "txt"], supportsExport: true, supportsImport: true }, exporter: envsDotenvExp, importer: envsDotenvImp });

// Mappings
registerFormat("mappings", { definition: { id: "mappings-json", label: "Bifurc JSON", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: mappingsJsonExp, importer: mappingsJsonImp });

// Proxy Rules
registerFormat("proxyRules", { definition: { id: "proxyrules-json", label: "Bifurc JSON", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: proxyJsonExp, importer: proxyJsonImp });

// WebSockets
registerFormat("websockets", { definition: { id: "websockets-json", label: "Bifurc JSON", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: wsJsonExp, importer: wsJsonImp });

// Webhooks
registerFormat("webhooks", { definition: { id: "webhooks-json", label: "Bifurc JSON", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: hooksJsonExp, importer: hooksJsonImp });
