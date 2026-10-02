// @vitest-environment jsdom
/**
 * `useEntityGitActions` — the middle link, which is the one that had never been exercised.
 *
 * The chain a Publish click travels is
 * `MappingsPanel.publish` → `panelFactory`'s binder → **this hook** → `window.api` → the engine.
 * `tests/renderer/mappingsPublish.test.tsx` pins the first two and `tests/sync/mappingPublish.test.ts`
 * pins the engine, but both of them *assume* the path this hook computes — the panel test fakes the
 * binder, and the engine test is handed the path directly. So a wrong path here is invisible to both,
 * and its symptom is precisely the report: the engine stages nothing, returns `{ok: true}` for a no-op,
 * and the card goes on saying "Not published".
 *
 * Hence these cases assert the **exact payload** that reaches the bridge, for every target shape.
 */
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { AppConfig, LocalMapping, ProxyRule } from "@/types";
import { useEntityGitActions } from "@/hooks/useEntityGitActions";

const WS = "ws-1";

function makeMapping(over: Partial<LocalMapping> = {}): LocalMapping {
  return {
    id: "m1",
    workspaceId: WS,
    domain: "app.localhost",
    target: "127.0.0.1:3000",
    label: "My App",
    enabled: true,
    ...over,
  } as LocalMapping;
}

function makeRule(over: Partial<ProxyRule> = {}): ProxyRule {
  return {
    id: "r1",
    workspaceId: WS,
    name: "Rule",
    pattern: "/api",
    useRegex: false,
    enabled: true,
    targetType: "mapping",
    targetMappingId: "m1",
    folderId: null,
    createdAt: 0,
    ...over,
  } as ProxyRule;
}

function makeConfig(over: Partial<AppConfig> = {}): AppConfig {
  return {
    activeWorkspaceId: WS,
    workspaces: [{ id: WS, name: "Default", createdAt: 0, activeEnvironmentId: null }],
    mappings: [],
    proxyRules: [],
    ruleFolders: [],
    mocks: [],
    requests: [],
    mockFolders: [],
    requestFolders: [],
    wsConnections: [],
    wsFolders: [],
    webhooks: [],
    webhookFolders: [],
    graphqlRequests: [],
    graphqlSchemas: [],
    graphqlRequestFolders: [],
    grpcRequests: [],
    protoFiles: [],
    grpcRequestFolders: [],
    soapRequests: [],
    savedWsdls: [],
    soapRequestFolders: [],
    environments: [],
    ...over,
  } as unknown as AppConfig;
}

let publishEntity: ReturnType<typeof vi.fn>;
let publishFolder: ReturnType<typeof vi.fn>;
let gitDiscard: ReturnType<typeof vi.fn>;
let getConfig: ReturnType<typeof vi.fn>;

beforeEach(() => {
  publishEntity = vi.fn().mockResolvedValue({ ok: true });
  publishFolder = vi.fn().mockResolvedValue({ ok: true });
  gitDiscard = vi.fn().mockResolvedValue({ ok: true });
  getConfig = vi.fn().mockResolvedValue(makeConfig({ mappings: [makeMapping()] }));
  Object.defineProperty(window, "api", {
    value: { publishEntity, publishFolder, gitDiscard, getConfig },
    configurable: true,
    writable: true,
  });
});

/**
 * What the engine's map would say for this config.
 *
 * `plan/19` WI 5 item 3 moved path computation into `workspace/entityPaths.ts`, so this suite no longer
 * relies on the hook to *derive* a path — it supplies the answer, as the engine would. The table below is
 * the same kind → config-array → folder-array mapping the engine's registry walk uses; the engine's own
 * copy is pinned by `tests/engine/entityPaths.test.ts`, and this one exists so these cases can stay about
 * **the payload that reaches the bridge** rather than about path arithmetic.
 */
const KIND_TABLES: Array<[string, keyof AppConfig, keyof AppConfig | null]> = [
  ["mappings", "mappings", null],
  ["environments", "environments", null],
  ["rules", "proxyRules", "ruleFolders"],
  ["requests", "requests", "requestFolders"],
  ["mocks", "mocks", "mockFolders"],
  ["sockets", "wsConnections", "wsFolders"],
  ["webhooks", "webhooks", "webhookFolders"],
  ["graphqlRequests", "graphqlRequests", "graphqlRequestFolders"],
  ["grpcRequests", "grpcRequests", "grpcRequestFolders"],
  ["soapRequests", "soapRequests", "soapRequestFolders"],
];

function enginePaths(cfg: AppConfig): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [kind, listKey, folderKey] of KIND_TABLES) {
    const list = ((cfg[listKey] as unknown as any[]) ?? []);
    const folders = folderKey ? ((cfg[folderKey] as unknown as any[]) ?? []) : [];
    for (const e of list) {
      const folder = e.folderId ? folders.find((f: any) => f.id === e.folderId) : null;
      out[e.id] = folder ? `${kind}/${folder.name}/${e.id}.json` : `${kind}/${e.id}.json`;
    }
  }
  return out;
}

function mount(config: AppConfig, panel = "mappings") {
  const refreshConfig = vi.fn().mockResolvedValue(undefined);
  const refreshEntitySyncStatus = vi.fn();
  const onAfterWrite = vi.fn();
  const paths = enginePaths(config);
  const hook = renderHook(() =>
    useEntityGitActions({
      wsConfig: config,
      wsId: WS,
      refreshConfig,
      refreshEntitySyncStatus,
      relPathOf: (id: string) => paths[id] ?? "",
      panel,
      onAfterWrite,
    }),
  );
  return { ...hook, refreshConfig, refreshEntitySyncStatus, onAfterWrite };
}

describe("useEntityGitActions — the path a Publish click actually sends", () => {
  it("sends a flat kind's path with no folder level", async () => {
    const { result } = mount(makeConfig({ mappings: [makeMapping()] }));

    await act(async () => {
      await result.current.publish({ type: "entity", kind: "mappings", id: "m1" });
    });

    // `mappings/<id>.json` is the only correct answer. `mappings/m1.json/…`, a folder-prefixed form,
    // or a `.localhost`-derived name would all stage nothing and return `{ok: true}`.
    expect(publishEntity).toHaveBeenCalledWith(WS, ["mappings/m1.json"], undefined);
  });

  it("resolves a foldered entity's path against ITS OWN kind's folder list", async () => {
    // The Requests panel is multiplexed across four storage kinds; resolving a GraphQL request
    // against `requestFolders` produced `graphqlRequests/<id>.json` with no folder — a path that does
    // not exist — which is the same silent no-op one kind over.
    const rule = makeRule({ folderId: "f1" });
    const { result } = mount(
      makeConfig({ proxyRules: [rule], ruleFolders: [{ id: "f1", name: "Folder One", parentId: null, workspaceId: WS, createdAt: 0 }] }),
      "rules",
    );

    await act(async () => {
      await result.current.publish({ type: "entity", kind: "rules", id: "r1" });
    });

    expect(publishEntity).toHaveBeenCalledWith(WS, ["rules/Folder One/r1.json"], undefined);
  });

  it("recovers from a caller that named the wrong kind, using the id to find the right one", async () => {
    const { result } = mount(makeConfig({ mappings: [makeMapping()] }));

    await act(async () => {
      // "requests" is what a multiplexed panel would pass for something that is really a mapping.
      await result.current.publish({ type: "entity", kind: "requests", id: "m1" });
    });

    expect(publishEntity).toHaveBeenCalledWith(WS, ["mappings/m1.json"], undefined);
  });

  it("passes a bare path straight through, for a surface that already knows it", async () => {
    const { result } = mount(makeConfig({ mappings: [makeMapping()] }));

    await act(async () => {
      await result.current.publish({ type: "path", path: "mappings/m1.json" });
    });

    expect(publishEntity).toHaveBeenCalledWith(WS, ["mappings/m1.json"], undefined);
  });

  it("publishes a whole selection in ONE commit", async () => {
    const { result } = mount(makeConfig({ mappings: [makeMapping()] }));

    await act(async () => {
      await result.current.publish({ type: "paths", paths: ["mappings/a.json", "mappings/b.json"] });
    });

    expect(publishEntity).toHaveBeenCalledTimes(1);
    expect(publishEntity).toHaveBeenCalledWith(WS, ["mappings/a.json", "mappings/b.json"], undefined);
  });

  it("routes a folder through folder.publish, which owns the folder-name sanitiser", async () => {
    const { result } = mount(makeConfig({ mappings: [makeMapping()] }));

    await act(async () => {
      await result.current.publish({ type: "folder", kind: "mocks", folderName: "My Folder" });
    });

    // The raw name, not a pre-sanitised one: the engine's `folder.publish` must use the *storage*
    // sanitiser, and pre-sanitising here would reintroduce the second, divergent one.
    expect(publishFolder).toHaveBeenCalledWith(WS, "mocks", "My Folder");
  });

  it("refuses a path the workspace no longer holds, instead of publishing nothing", async () => {
    const { result } = mount(makeConfig({ mappings: [] }));

    let outcome: { ok: boolean; error?: string } | undefined;
    await act(async () => {
      outcome = await result.current.publish({ type: "entity", kind: "mappings", id: "gone" });
    });

    expect(outcome?.ok).toBe(false);
    expect(publishEntity).not.toHaveBeenCalled();
  });

  it("reverts the same path it would publish", async () => {
    const { result } = mount(makeConfig({ mappings: [makeMapping()] }));

    await act(async () => {
      await result.current.revert({ type: "entity", kind: "mappings", id: "m1" });
    });

    expect(gitDiscard).toHaveBeenCalledWith(WS, "mappings/m1.json");
  });

  it("publishes the whole panel's kind, and the singleton file for the health bar", async () => {
    const a = mount(makeConfig({ mappings: [makeMapping()] }), "mappings");
    await act(async () => { await a.result.current.publishPanel(); });
    expect(publishFolder).toHaveBeenCalledWith(WS, "mappings", null);

    const b = mount(makeConfig(), "healthbar");
    await act(async () => { await b.result.current.publishPanel(); });
    expect(publishEntity).toHaveBeenCalledWith(WS, ["healthbar/services.json"]);
  });

  it("refreshes the config, the status AND the History sidebar after a write", async () => {
    const { result, refreshConfig, refreshEntitySyncStatus, onAfterWrite } = mount(makeConfig({ mappings: [makeMapping()] }));

    await act(async () => {
      await result.current.publish({ type: "entity", kind: "mappings", id: "m1" });
    });

    // All three, in this order. `onAfterWrite` is what refreshes an open History panel, and the four
    // panels that never passed `onAfterSave` are why it lives here rather than at each call site.
    expect(refreshConfig).toHaveBeenCalledTimes(1);
    expect(refreshEntitySyncStatus).toHaveBeenCalledWith(WS);
    expect(onAfterWrite).toHaveBeenCalledTimes(1);
  });

  it("passes the commit id straight through to the caller", async () => {
    // The id is the proof a git operation happened, so the hook must not swallow or reshape it —
    // `write()` returns whatever the writer returned.
    const sha = "4f2a1c9e8b7d6a5f4e3d2c1b0a9f8e7d6c5b4a39";
    publishEntity.mockResolvedValue({ ok: true, commit: sha });
    const { result } = mount(makeConfig({ mappings: [makeMapping()] }));

    let outcome: { ok: boolean; commit?: string | null } | undefined;
    await act(async () => {
      outcome = await result.current.publish({ type: "entity", kind: "mappings", id: "m1" });
    });

    expect(outcome?.commit).toBe(sha);
  });

  it("reports a failure and still refreshes, rather than leaving the UI claiming success", async () => {
    publishEntity.mockResolvedValue({ ok: false, error: "no remote configured" });
    const { result, refreshEntitySyncStatus } = mount(makeConfig({ mappings: [makeMapping()] }));

    let outcome: { ok: boolean; error?: string } | undefined;
    await act(async () => {
      outcome = await result.current.publish({ type: "entity", kind: "mappings", id: "m1" });
    });

    expect(outcome?.ok).toBe(false);
    expect(result.current.lastError).toBe("no remote configured");
    // The refresh still runs: a partial failure can leave the tree out of step with disk.
    expect(refreshEntitySyncStatus).toHaveBeenCalledWith(WS);
  });
});
