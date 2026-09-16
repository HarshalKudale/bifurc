import { ipcMain } from "electron";
import type { SoapFetchWsdlParams, SoapExecuteParams } from "@bifurc/protocol";
import { registerEntityCrudHandlers, registerSimpleEntityHandlers } from "@/ipc/handlers/entityCrudFactory";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { bus } from "@bifurc/engine/eventBus";

interface SavedSoapRequest {
  id: string; name: string; endpointUrl: string; soapAction: string;
  headers: Record<string, string>; body: string; wsdlId?: string | null;
  operationName?: string; preScript?: string; postScript?: string;
  createdAt: number; folderId?: string | null; workspaceId: string;
}

interface SavedSoapMock {
  id: string; name: string; enabled: boolean; endpointPattern: string; useRegex: boolean;
  soapActionPattern: string; operationName?: string;
  responseStatus: number; responseHeaders: Record<string, string>; responseBody: string;
  responseDelay?: number; wsdlId?: string | null;
  createdAt: number; folderId?: string | null; workspaceId: string;
}

interface SavedWsdl {
  id: string; name: string; content: string; sourceUrl?: string;
  importedAt: number; createdAt: number; workspaceId: string;
}

// P2 work item 7 — soap.fetchWsdl/execute convert here directly; soap:addRequest/updateRequest/
// deleteRequest and soap:addMock/updateMock/deleteMock already route through the CommandRegistry
// too, via the `registerEntityCrudHandlers()` calls below (the CRUD collapse — see
// `entityCrudFactory.ts` and `plan/03-phase-2-engine-extraction.md` work item 7's tenth batch).
// `soap:addWsdl/deleteWsdl/listWsdls` now route through the same registry too, via
// `registerSimpleEntityHandlers()` — they don't track a `configKey` array the way every
// `CrudFactoryOpts` kind does (WSDLs are written straight to disk, nothing mirrors them into
// `AppConfig`), and have no "update" concept, so they go through the smaller
// `entity.create`/`entity.delete`/`entity.list` fallback path instead.
const ctx = { bus };

commandRegistry.register("soap.fetchWsdl", async ({ url }: SoapFetchWsdlParams) => {
  const httpMod = url.startsWith("https") ? require("https") : require("http");
  try {
    const parsed = new URL(url);
    const content = await new Promise<string>((resolve, reject) => {
      const req = httpMod.request(
        { hostname: parsed.hostname, port: parsed.port, path: parsed.pathname + parsed.search, method: "GET", rejectUnauthorized: false },
        (res: any) => {
          let data = "";
          res.on("data", (chunk: string) => { data += chunk; });
          res.on("end", () => resolve(data));
        },
      );
      req.on("error", reject);
      req.end();
    });
    return { ok: true, content };
  } catch (err: any) {
    return { ok: false, error: err?.message ?? "Failed to fetch WSDL" };
  }
});

commandRegistry.register("soap.execute", async ({ endpointUrl, soapAction, headers, body }: SoapExecuteParams) => {
  const httpMod = endpointUrl.startsWith("https") ? require("https") : require("http");
  const parsed = new URL(endpointUrl);
  const xmlBody = Buffer.from(body, "utf-8");
  const reqHeaders: Record<string, string> = {
    "Content-Type": "text/xml; charset=utf-8",
    "Content-Length": String(xmlBody.length),
    ...headers,
  };
  if (soapAction) reqHeaders["SOAPAction"] = soapAction;
  const start = Date.now();
  const { status, resHeaders, resBody } = await new Promise<{ status: number; resHeaders: Record<string, string>; resBody: string }>((resolve, reject) => {
    const req = httpMod.request(
      { hostname: parsed.hostname, port: parsed.port, path: parsed.pathname + parsed.search, method: "POST", headers: reqHeaders, rejectUnauthorized: false },
      (res: any) => {
        let data = "";
        res.on("data", (chunk: string) => { data += chunk; });
        res.on("end", () => {
          const h: Record<string, string> = {};
          for (const [k, v] of Object.entries(res.headers)) { h[k] = Array.isArray(v) ? v.join(", ") : String(v); }
          resolve({ status: res.statusCode ?? 0, resHeaders: h, resBody: data });
        });
      },
    );
    req.on("error", reject);
    req.write(xmlBody);
    req.end();
  });
  return { status, headers: resHeaders, body: resBody, durationMs: Date.now() - start };
});

export function registerSoapHandlers() {
  registerEntityCrudHandlers<SavedSoapRequest>({
    ipcAdd: "soap:addRequest", ipcUpdate: "soap:updateRequest", ipcDelete: "soap:deleteRequest",
    kind: "soapRequests",
    configKey: "soapRequests" as any,
    folderConfigKey: "soapRequestFolders" as any,
    getNameEntry: (req) => ({ name: req.name, endpointUrl: req.endpointUrl, soapAction: req.soapAction }),
  });

  registerEntityCrudHandlers<SavedSoapMock>({
    ipcAdd: "soap:addMock", ipcUpdate: "soap:updateMock", ipcDelete: "soap:deleteMock",
    kind: "soapMocks",
    configKey: "soapMocks" as any,
    folderConfigKey: "soapMockFolders" as any,
    hasEnabledState: true,
    validate: (mock) => {
      if (!mock.endpointPattern || !mock.endpointPattern.trim()) throw new Error("endpointPattern is required for SOAP mocks");
    },
    getNameEntry: (mock) => ({ name: mock.name, soapActionPattern: mock.soapActionPattern }),
  });

  registerSimpleEntityHandlers<SavedWsdl>({
    kind: "wsdls",
    ipcAdd: "soap:addWsdl", ipcDelete: "soap:deleteWsdl", ipcList: "soap:listWsdls",
  });

  ipcMain.handle("soap:fetchWsdl", (_e, url: string) =>
    commandRegistry.invoke("soap.fetchWsdl", { url }, ctx));

  ipcMain.handle("soap:execute", (_e, params: SoapExecuteParams) =>
    commandRegistry.invoke("soap.execute", params, ctx));
}
