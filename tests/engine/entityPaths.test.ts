import { describe, it, expect, vi, beforeAll } from "vitest";

/**
 * `loadEntityPathMaps()` — `plan/19` WI 5, item 3.
 *
 * ## Why this mocks `loadConfig` — and why that is not the usual "mocking the store tests nothing"
 *
 * The first version of this file ran against a real temp data root, real settings and real `saveConfig`,
 * and it **failed**: `loadConfig()` does not read entities from the config JSON at all. It rebuilds them by
 * reading the entity **files** (`readAllEntities` / `readEntityStubs` + the `names.json` index), so writing
 * arrays into the config proved nothing about what the function would see.
 *
 * That is the signal to move the seam. What this module *adds* is **composition** — which kind is flat,
 * which folder a folderId names, how the path is spelled — and that is the part worth pinning. The storage
 * layer underneath it is already covered by the suites that own it; re-driving it here would make this test
 * a second, worse copy of theirs, and it would fail for their reasons rather than this module's.
 *
 * The **kind registry is real** (`registerCrudKinds()`), which is the other half of the seam: the whole
 * reason this module walks the registry instead of keeping a table is that a new kind must appear for free,
 * and a mocked registry would hide exactly that.
 */

const h = vi.hoisted(() => {
  const FIXTURE = {
    activeWorkspaceId: "ws1",
    // `My: Folder?` is deliberate: `:` and `?` are illegal on Windows and the storage sanitiser strips
    // them, so the expected path below is only right if this module uses the *same* sanitiser.
    requestFolders: [{ id: "f1", name: "My: Folder?", parentId: null, createdAt: 1, workspaceId: "ws1" }],
    requests: [
      { id: "r1", folderId: "f1" },
      { id: "r2", folderId: null },
    ],
    mockFolders: [{ id: "mf1", name: "Before", parentId: null, createdAt: 1, workspaceId: "ws1" }],
    mocks: [{ id: "mk1", folderId: "mf1" }],
    // A flat kind: `mappings` is written as `kind/id.json` whatever its `folderId` says.
    mappings: [{ id: "m1", folderId: "f1" }],
  };
  return { FIXTURE };
});

vi.mock("@bifurc/engine/workspace/config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@bifurc/engine/workspace/config")>()),
  loadConfig: () => h.FIXTURE,
}));

import { loadEntityPathMaps } from "@bifurc/engine/workspace/entityPaths";
import { registerCrudKinds } from "@bifurc/engine/workspace/entities/kinds/crudKinds";

describe("loadEntityPathMaps", () => {
  beforeAll(() => {
    // The registry is process-global and module-private; this is the only way in, and it is the real one.
    registerCrudKinds();
  });

  it("uses the folder's sanitised NAME, and gives a flat kind no folder segment", () => {
    const maps = loadEntityPathMaps();

    expect(maps.entities["r1"]).toBe("requests/My Folder/r1.json");
    expect(maps.entities["r2"]).toBe("requests/r2.json");
    // Flat: no folder segment even though the entity carries a `folderId`.
    expect(maps.entities["m1"]).toBe("mappings/m1.json");
  });

  it("emits the folder prefix as a directory, trailing slash included", () => {
    // `FolderTree`'s git rollup matches status keys with `startsWith(prefix)`, so a missing slash would
    // also match a sibling folder whose name merely starts with this one.
    expect(loadEntityPathMaps().folders["f1"]).toBe("requests/My Folder/");
  });

  it("is a reading of the config, so a rename moves the path and the prefix together", () => {
    expect(loadEntityPathMaps().entities["mk1"]).toBe("mocks/Before/mk1.json");

    h.FIXTURE.mockFolders[0].name = "After";

    const maps = loadEntityPathMaps();
    expect(maps.entities["mk1"]).toBe("mocks/After/mk1.json");
    expect(maps.folders["mf1"]).toBe("mocks/After/");
  });

  it("covers every kind the registry knows, so a new kind appears for free", () => {
    // The property that makes walking the registry better than a second table: a kind registered here is
    // in the map. `rules` is not in the fixture, so this asserts the *walk* rather than the fixture — a
    // hand-kept table that forgot `rules` would still pass the three cases above.
    const cfg = h.FIXTURE as Record<string, unknown>;
    cfg.proxyRules = [{ id: "rule1", folderId: null }];
    expect(loadEntityPathMaps().entities["rule1"]).toBe("rules/rule1.json");
    delete cfg.proxyRules;
  });
});
