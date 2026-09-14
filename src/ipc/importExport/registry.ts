import type { EntityKind, FormatDefinition, FormatsMap, ExportResult, PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

export interface ExporterFn {
  run(wsId: string, filePath: string): Promise<ExportResult>;
}

export interface ImporterFn {
  preflight(wsId: string, filePath: string): PreflightResult;
  run(wsId: string, filePath: string, strategy: CollisionStrategy): Promise<ImportResult>;
}

export interface FormatEntry {
  definition: FormatDefinition;
  exporter?: ExporterFn;
  importer?: ImporterFn;
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

export function getExporter(kind: EntityKind, formatId: string): ExporterFn | undefined {
  return registry.get(kind)?.find((e) => e.definition.id === formatId)?.exporter;
}

export function getImporter(kind: EntityKind, formatId: string): ImporterFn | undefined {
  return registry.get(kind)?.find((e) => e.definition.id === formatId)?.importer;
}

// ── Register all built-in formats ─────────────────────────────────────────

import * as workspaceJsonExp from "@/ipc/importExport/exporters/workspace-json";
import * as workspaceZipExp  from "@/ipc/importExport/exporters/workspace-zip";
import * as workspaceJsonImp from "@/ipc/importExport/importers/workspace-json";
import * as workspaceZipImp  from "@/ipc/importExport/importers/workspace-zip";

import * as reqsPostmanExp    from "@/ipc/importExport/exporters/requests-postman";
import * as reqsOpenApiExp    from "@/ipc/importExport/exporters/requests-openapi";
import * as reqsCurlExp       from "@/ipc/importExport/exporters/requests-curl";
import * as reqsHarExp        from "@/ipc/importExport/exporters/requests-har";
import * as reqsInsomniaExp   from "@/ipc/importExport/exporters/requests-insomnia";
import * as reqsPostmanImp    from "@/ipc/importExport/importers/requests-postman";
import * as reqsOpenApiImp    from "@/ipc/importExport/importers/requests-openapi";
import * as reqsCurlImp       from "@/ipc/importExport/importers/requests-curl";
import * as reqsHarImp        from "@/ipc/importExport/importers/requests-har";
import * as reqsInsomniaImp   from "@/ipc/importExport/importers/requests-insomnia";

import * as mocksPostmanExp   from "@/ipc/importExport/exporters/mocks-postman";
import * as mocksJsonExp      from "@/ipc/importExport/exporters/mocks-json";
import * as mocksWireMockExp  from "@/ipc/importExport/exporters/mocks-wiremock";
import * as mocksPostmanImp   from "@/ipc/importExport/importers/mocks-postman";
import * as mocksJsonImp      from "@/ipc/importExport/importers/mocks-json";
import * as mocksWireMockImp  from "@/ipc/importExport/importers/mocks-wiremock";

import * as envsJsonExp       from "@/ipc/importExport/exporters/environments-json";
import * as envsPostmanExp    from "@/ipc/importExport/exporters/environments-postman";
import * as envsDotenvExp     from "@/ipc/importExport/exporters/environments-dotenv";
import * as envsJsonImp       from "@/ipc/importExport/importers/environments-json";
import * as envsPostmanImp    from "@/ipc/importExport/importers/environments-postman";
import * as envsDotenvImp     from "@/ipc/importExport/importers/environments-dotenv";

import * as mappingsJsonExp   from "@/ipc/importExport/exporters/mappings-json";
import * as mappingsJsonImp   from "@/ipc/importExport/importers/mappings-json";

import * as proxyJsonExp      from "@/ipc/importExport/exporters/proxyrules-json";
import * as proxyJsonImp      from "@/ipc/importExport/importers/proxyrules-json";

import * as wsJsonExp         from "@/ipc/importExport/exporters/websockets-json";
import * as wsJsonImp         from "@/ipc/importExport/importers/websockets-json";

import * as hooksJsonExp      from "@/ipc/importExport/exporters/webhooks-json";
import * as hooksJsonImp      from "@/ipc/importExport/importers/webhooks-json";

// Workspace
registerFormat("workspace", { definition: { id: "workspace-json", label: "Bifurc JSON", extensions: ["json"], supportsExport: true, supportsImport: true }, exporter: workspaceJsonExp, importer: workspaceJsonImp });
registerFormat("workspace", { definition: { id: "workspace-zip", label: "ZIP Archive", extensions: ["zip"], supportsExport: true, supportsImport: true }, exporter: workspaceZipExp, importer: workspaceZipImp });

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
