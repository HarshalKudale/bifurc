import { ipcMain, dialog } from "electron";
import { getAllFormats, getExporter, getImporter } from "@/ipc/importExport/registry";
import type { EntityKind, ExportRequest, PreflightRequest, ImportRequest } from "@/ipc/importExport/types";
import { commandRegistry } from "@/commands/registry";
import { bus } from "@bifurc/engine/eventBus";

// Import registry side effects (registers all formats)
import "./registry";

// P2 work item 7 — only `export.formats` converts here. The other three
// (`importExport:export/preflight/import`) were checked and are genuinely NOT safe to convert
// as a registration-only move — two separate real gaps, not one:
//   - `export.create`/`import.preflight` are SPLIT (engine should render/read content or a
//     blobId; client owns the save/open dialog — see this file's own comment: "Full blob-layer
//     rewiring is P3's job"), but today's handlers still call `dialog.showSaveDialog`/
//     `showOpenDialog` themselves and pass the resulting path straight to the exporter/importer.
//   - `import.commit`'s `collisionStrategy` enum is `["skip", "overwrite", "rename"]` in the
//     frozen protocol schema, but the actual local `CollisionStrategy` type (and every real
//     renderer call site) uses `["keep", "override", "new"]` — different string values, not
//     just missing fields. Converting this one would reject every real import outright.
// Both are P3 (`File_Ops_Protocol.md` blob layer) territory, not this pass's.
const ctx = { bus };

commandRegistry.register("export.formats", () => getAllFormats());

export function registerImportExportHandlers(): void {
  ipcMain.handle("importExport:formats", () => commandRegistry.invoke("export.formats", {}, ctx));

  ipcMain.handle("importExport:export", async (_e, req: ExportRequest) => {
    const { kind, format, wsId } = req;
    const exporter = getExporter(kind as EntityKind, format);
    if (!exporter) return { ok: false, error: `No exporter for ${kind}/${format}` };

    const formats = getFormatsForDialog(kind as EntityKind, format);
    const safeKind = kind.replace(/([A-Z])/g, "-$1").toLowerCase();
    const ext = formats.extensions[0] ?? "json";

    const { filePath, canceled } = await dialog.showSaveDialog({
      title: `Export ${kindLabel(kind as EntityKind)}`,
      defaultPath: `${safeKind}-export.${ext}`,
      filters: [
        { name: formats.label, extensions: formats.extensions },
        { name: "All Files", extensions: ["*"] },
      ],
    });

    if (canceled || !filePath) return { ok: false, canceled: true };
    return exporter.run(wsId, filePath);
  });

  ipcMain.handle("importExport:preflight", async (_e, req: PreflightRequest) => {
    const { kind, format, wsId } = req;
    const importer = getImporter(kind as EntityKind, format);
    if (!importer) return { ok: false, error: `No importer for ${kind}/${format}` };

    const formats = getFormatsForDialog(kind as EntityKind, format);

    const { filePaths, canceled } = await dialog.showOpenDialog({
      title: `Import ${kindLabel(kind as EntityKind)}`,
      filters: [
        { name: formats.label, extensions: formats.extensions },
        { name: "All Files", extensions: ["*"] },
      ],
      properties: ["openFile"],
    });

    if (canceled || filePaths.length === 0) return { ok: false, canceled: true };
    return importer.preflight(wsId, filePaths[0]);
  });

  ipcMain.handle("importExport:import", async (_e, req: ImportRequest) => {
    const { kind, format, wsId, filePath, collisionStrategy } = req;
    const importer = getImporter(kind as EntityKind, format);
    if (!importer) return { ok: false, error: `No importer for ${kind}/${format}` };
    return importer.run(wsId, filePath, collisionStrategy);
  });
}

function getFormatsForDialog(kind: EntityKind, formatId: string): { label: string; extensions: string[] } {
  const { getFormats } = require("./registry") as typeof import("@/ipc/importExport/registry");
  const fmt = getFormats(kind).find((f) => f.id === formatId);
  return fmt ?? { label: "Files", extensions: ["json"] };
}

function kindLabel(kind: EntityKind): string {
  const labels: Record<EntityKind, string> = {
    workspace: "Workspace",
    requests: "Requests",
    mocks: "Mocks",
    environments: "Environments",
    mappings: "Mappings",
    proxyRules: "Proxy Rules",
    websockets: "WebSockets",
    webhooks: "Webhooks",
  };
  return labels[kind] ?? kind;
}
