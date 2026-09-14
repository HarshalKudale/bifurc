import { ipcMain } from "electron";
import { registerEntityCrudHandlers } from "@/ipc/handlers/entityCrudFactory";
import { loadConfig } from "@/store/config";
import { generateId } from "@/store/config";
import {
  writeEntity, deleteEntityFile, readAllEntities,
} from "@/store/workspaceFs";

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

  ipcMain.handle("soap:addWsdl", async (_e, wsdl: Omit<SavedWsdl, "id" | "createdAt">) => {
    const cfg = loadConfig();
    const wsId = (wsdl as any).workspaceId ?? cfg.activeWorkspaceId;
    const newWsdl: SavedWsdl = { ...wsdl, id: generateId(), createdAt: Date.now(), workspaceId: wsId };
    writeEntity(wsId, "wsdls", newWsdl.id, newWsdl, null);
    return newWsdl;
  });

  ipcMain.handle("soap:deleteWsdl", async (_e, id: string) => {
    const cfg = loadConfig();
    const wsId = cfg.activeWorkspaceId;
    deleteEntityFile(wsId, "wsdls", id);
    return { ok: true };
  });

  ipcMain.handle("soap:listWsdls", async () => {
    const cfg = loadConfig();
    const wsId = cfg.activeWorkspaceId;
    return readAllEntities<SavedWsdl>(wsId, "wsdls");
  });

  ipcMain.handle("soap:fetchWsdl", async (_e, url: string) => {
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

  ipcMain.handle("soap:execute", async (_e, { endpointUrl, soapAction, headers, body }: { endpointUrl: string; soapAction: string; headers: Record<string, string>; body: string }) => {
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
}
