import { ipcMain } from "electron";
import type { GraphqlIntrospectParams, GraphqlExecuteParams } from "@bifurc/protocol";
import { registerEntityCrudHandlers } from "@/ipc/handlers/entityCrudFactory";
import { loadConfig, generateId } from "@/store/config";
import { writeEntity, deleteEntityFile, readAllEntities } from "@/store/workspaceFs";
import { commandRegistry } from "@/commands/registry";
import { bus } from "@/eventBus";

interface SavedGraphQLRequest {
  id: string; name: string; endpointUrl: string; headers: Record<string, string>;
  query: string; variables: string; operationName: string;
  preScript?: string; postScript?: string; schemaId?: string | null;
  createdAt: number; folderId?: string | null; workspaceId: string;
}

interface SavedGraphQLMock {
  id: string; name: string; enabled: boolean; endpointPattern: string; useRegex: boolean;
  operationType: "query" | "mutation" | "subscription" | "any"; operationName: string;
  responseStatus: number; responseHeaders: Record<string, string>; responseBody: string;
  responseDelay?: number; schemaId?: string | null;
  createdAt: number; folderId?: string | null; workspaceId: string;
}

interface SavedGraphQLSchema {
  id: string; name: string; content: string; endpointUrl?: string;
  introspectedAt?: number; createdAt: number; workspaceId: string;
}

// P2 work item 7 — only graphql.introspect/execute convert here. `graphql:addRequest/
// updateRequest/deleteRequest` and `graphql:addMock/updateMock/deleteMock` go through the
// entityCrudFactory (out of scope until the CRUD collapse, per work item 7's status note), and
// `graphql:addSchema/deleteSchema/listSchemas` have no protocol command at all yet — the
// protocol's own comment on `graphql.ts` says GraphQL schema CRUD is meant to collapse into
// `entity.*` too, so they wait for the same collapsing pass.
const ctx = { bus };

commandRegistry.register("graphql.introspect", async ({ url, headers }: GraphqlIntrospectParams) => {
  const introspectionQuery = `
      query IntrospectionQuery {
        __schema {
          queryType { name }
          mutationType { name }
          subscriptionType { name }
          types { ...FullType }
          directives { name description locations args { ...InputValue } }
        }
      }
      fragment FullType on __Type {
        kind name description
        fields(includeDeprecated: true) { name description args { ...InputValue } type { ...TypeRef } isDeprecated deprecationReason }
        inputFields { ...InputValue }
        interfaces { ...TypeRef }
        enumValues(includeDeprecated: true) { name description isDeprecated deprecationReason }
        possibleTypes { ...TypeRef }
      }
      fragment InputValue on __InputValue { name description type { ...TypeRef } defaultValue }
      fragment TypeRef on __Type { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } } } }
    `;
  try {
    const http = url.startsWith("https") ? require("https") : require("http");
    const body = JSON.stringify({ query: introspectionQuery });
    const parsed = new URL(url);
    const reqHeaders: Record<string, string> = { "Content-Type": "application/json", "Content-Length": String(Buffer.byteLength(body)), ...headers };
    const result = await new Promise<string>((resolve, reject) => {
      const req = http.request({ hostname: parsed.hostname, port: parsed.port, path: parsed.pathname + parsed.search, method: "POST", headers: reqHeaders, rejectUnauthorized: false }, (res: any) => {
        let data = "";
        res.on("data", (chunk: string) => { data += chunk; });
        res.on("end", () => resolve(data));
      });
      req.on("error", reject);
      req.write(body);
      req.end();
    });
    return { ok: true, sdl: result };
  } catch (err: any) {
    return { ok: false, error: err.message ?? String(err) };
  }
});

commandRegistry.register("graphql.execute", async ({ url, headers, query, variables, operationName }: GraphqlExecuteParams) => {
  const http = url.startsWith("https") ? require("https") : require("http");
  let parsedVars: unknown = undefined;
  try { if (variables && variables.trim()) parsedVars = JSON.parse(variables); } catch { /* leave undefined */ }
  const body = JSON.stringify({ query, variables: parsedVars, operationName: operationName || undefined });
  const parsed = new URL(url);
  const reqHeaders: Record<string, string> = { "Content-Type": "application/json", "Content-Length": String(Buffer.byteLength(body)), ...headers };
  const start = Date.now();
  const { status, resHeaders, resBody } = await new Promise<{ status: number; resHeaders: Record<string, string>; resBody: string }>((resolve, reject) => {
    const req = http.request({ hostname: parsed.hostname, port: parsed.port, path: parsed.pathname + parsed.search, method: "POST", headers: reqHeaders, rejectUnauthorized: false }, (res: any) => {
      let data = "";
      res.on("data", (chunk: string) => { data += chunk; });
      res.on("end", () => {
        const h: Record<string, string> = {};
        for (const [k, v] of Object.entries(res.headers)) { h[k] = Array.isArray(v) ? v.join(", ") : String(v); }
        resolve({ status: res.statusCode ?? 0, resHeaders: h, resBody: data });
      });
    });
    req.on("error", reject);
    req.write(body);
    req.end();
  });
  return { status, headers: resHeaders, body: resBody, durationMs: Date.now() - start };
});

export function registerGraphqlHandlers() {
  registerEntityCrudHandlers<SavedGraphQLRequest>({
    ipcAdd: "graphql:addRequest", ipcUpdate: "graphql:updateRequest", ipcDelete: "graphql:deleteRequest",
    kind: "graphqlRequests",
    configKey: "graphqlRequests" as any,
    folderConfigKey: "graphqlRequestFolders" as any,
    getNameEntry: (req) => ({ name: req.name, endpointUrl: req.endpointUrl }),
  });

  registerEntityCrudHandlers<SavedGraphQLMock>({
    ipcAdd: "graphql:addMock", ipcUpdate: "graphql:updateMock", ipcDelete: "graphql:deleteMock",
    kind: "graphqlMocks",
    configKey: "graphqlMocks" as any,
    folderConfigKey: "graphqlMockFolders" as any,
    hasEnabledState: true,
    validate: (mock) => {
      if (!mock.endpointPattern || !mock.endpointPattern.trim()) throw new Error("endpointPattern is required for GraphQL mocks");
    },
    getNameEntry: (mock) => ({ name: mock.name, operationName: mock.operationName }),
  });

  ipcMain.handle("graphql:addSchema", async (_e, schema: Omit<SavedGraphQLSchema, "id" | "createdAt">) => {
    const cfg = loadConfig();
    const wsId = (schema as any).workspaceId ?? cfg.activeWorkspaceId;
    const newSchema: SavedGraphQLSchema = { ...schema, id: generateId(), createdAt: Date.now(), workspaceId: wsId };
    writeEntity(wsId, "graphqlSchemas", newSchema.id, newSchema, null);
    return newSchema;
  });

  ipcMain.handle("graphql:deleteSchema", async (_e, id: string) => {
    const cfg = loadConfig();
    const wsId = cfg.activeWorkspaceId;
    deleteEntityFile(wsId, "graphqlSchemas", id);
    return { ok: true };
  });

  ipcMain.handle("graphql:listSchemas", async () => {
    const cfg = loadConfig();
    const wsId = cfg.activeWorkspaceId;
    return readAllEntities<SavedGraphQLSchema>(wsId, "graphqlSchemas");
  });

  ipcMain.handle("graphql:introspect", (_e, params: GraphqlIntrospectParams) =>
    commandRegistry.invoke("graphql.introspect", params, ctx));

  ipcMain.handle("graphql:execute", (_e, params: GraphqlExecuteParams) =>
    commandRegistry.invoke("graphql.execute", params, ctx));
}
