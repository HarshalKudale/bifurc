/**
 * The e2e fixture ↔ engine contract: `names.json` holds **objects**, not strings.
 *
 * ## The failure this pins, measured 2026-09-21
 *
 * Four of the eleven e2e specs were failing. One of them —
 * `proxy-rules.spec.ts` › "renders the panel with its add control and seeded rules" — failed like this:
 *
 * ```
 * Error: expect(locator).toContainText(expected) failed
 *   Expected substring: "Block Analytics"
 *   Received string:    "… Add Rule Name Match Pattern On — regex Publish Revert — regex … 2 rules"
 * ```
 *
 * The panel had rendered, the count was right (**2 rules**), and every **name and pattern was empty** —
 * the `—` placeholder from `ProxyRulesTable.tsx`'s `{r.name || "—"}`. The cause was not the panel, the
 * store or the IPC seam: **the fixture wrote `rules/names.json` as `{ "rule-001": "Block Analytics" }` — a
 * string per id — while the engine reads `EntityNameEntry` objects** (`fsNamesIndex.ts:5`), via
 * `store/config.ts`'s `ruleNames[s.id]?.name` and `?.url`. A string value yields `undefined` for both, so
 * the entity loads with `name: ""`. The same bug was in **all four** names indexes the fixture writes:
 * `rules`, `requests`, `sockets` (read as `wsConnections`) and `webhooks`.
 *
 * ## Why this test exists rather than only the fixture fix
 *
 * The fixture is the *only* thing that produces seeded e2e state, and nothing else in the suite reads it.
 * **~2,600 unit and integration tests were green throughout** — they build their own fixtures from
 * `src/`, so the fixture's on-disk shape was exercised by nothing but a real Electron run, which is the
 * most expensive and least frequent thing here. This test closes that gap at the cheapest level that can
 * still catch it: run the fixture, read it back through the engine's own `loadConfig()`, assert the values
 * survive.
 *
 * It deliberately asserts on **field values, not shapes** — `name`, `pattern`, `method`, `url`,
 * `urlSuffix` — because the shape mismatch is only harmful through those reads. And it looks each entity
 * up **by id**, not index: the engine's order comes from `index.json`, so an index-based assertion would
 * fail on reordering rather than on the bug.
 *
 * ## The `%LOCALAPPDATA%` trap, which the first version of this probe fell into
 *
 * On win32 `store/workspaceFs.ts#dataRoot()` and `store/appSettings.ts#settingsPath()` both short-circuit
 * on `%LOCALAPPDATA%` **before** consulting the data root, so `setDataRoot()` alone is silently ignored and
 * the engine reads the *developer's real profile* — the divergence `TRAPS.md` records. The first probe run
 * therefore found 0 entities of every kind and "failed" for a reason unrelated to the bug. Pointing the env
 * at the temp root is how the e2e fixture isolates itself too, and it is restored in `afterAll` because a
 * leaked `%LOCALAPPDATA%` would redirect every later integration test in this worker.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { loadConfig } from "@bifurc/engine/workspace/config";
import { writeSampleWorkspace } from "../../e2e/fixtures/sampleData";

describe("the e2e fixture's seeded workspace survives the engine's own read path", () => {
  let root: string;
  let previousLocalAppData: string | undefined;
  let cfg: Record<string, any>;

  beforeAll(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "e2e-fixture-contract-"));
    const appDir = path.join(root, "Bifurc");
    const dataDir = path.join(appDir, "data");
    fs.mkdirSync(dataDir, { recursive: true });

    // Before `loadConfig()`, and before anything resolves a path — see the header.
    previousLocalAppData = process.env.LOCALAPPDATA;
    process.env.LOCALAPPDATA = root;

    // The fixture writes `app.json` to the parent of its data argument, which is where `settingsPath()`
    // looks once `%LOCALAPPDATA%` points here.
    writeSampleWorkspace(dataDir, null);
    cfg = loadConfig() as Record<string, any>;
  });

  afterAll(() => {
    if (previousLocalAppData === undefined) delete process.env.LOCALAPPDATA;
    else process.env.LOCALAPPDATA = previousLocalAppData;
    fs.rmSync(root, { recursive: true, force: true });
  });

  /** Look up by id — the engine orders by `index.json`, so an index assertion would test the wrong thing. */
  const byId = (list: unknown, id: string) =>
    (Array.isArray(list) ? list : []).find((x: any) => x.id === id) as any;

  it("reads every seeded proxy rule's name and pattern (the failure that started this)", () => {
    // The exact assertion `proxy-rules.spec.ts` makes against the DOM, one layer down.
    const rule = byId(cfg.proxyRules, "rule-001");
    expect(rule?.name).toBe("Block Analytics");
    // `pattern` comes from the names index's `url` field for this kind — a different key, same entry.
    expect(rule?.pattern).toBe(".*\\/analytics\\/.*");
    expect(cfg.proxyRules).toHaveLength(2);
  });

  it("reads every seeded request's name, method and url", () => {
    const req = byId(cfg.requests, "req-001");
    expect(req?.name).toBe("Get All Users");
    expect(req?.method).toBe("GET");
    expect(req?.url).toBe("{{baseUrl}}/api/users");
    expect(cfg.requests).toHaveLength(3);
  });

  it("reads every seeded websocket connection — the kind whose config field is `wsConnections`", () => {
    // Named `wsConnections` in `AppConfig`, `sockets` on disk. Getting that wrong is how the first probe
    // reported `undefined` for a whole kind.
    const ws = byId(cfg.wsConnections, "ws-001");
    expect(ws?.name).toBe("Live Chat Socket");
    expect(ws?.url).toBe("ws://localhost:3000/chat");
    expect(cfg.wsConnections).toHaveLength(2);
  });

  it("has no empty name anywhere a names index backs the kind", () => {
    /**
     * The generalisation of the bug, stated as a property rather than as four spot checks: **an empty name
     * is the symptom**, whatever the field is called. This is what would have failed on the old fixture for
     * every kind at once, and it is the assertion that catches a *fifth* kind being added with the wrong
     * shape.
     *
     * ## What this property had to be narrowed twice to become true, and both narrowings are the lesson
     *
     * 1. **Not every kind's display field is `name`.** The first version reported `mappings/map-001`, because
     *    `LocalMapping` carries **`label?` and no `name` at all** (`store/types.ts:3-10`). That was the test
     *    being wrong, not the fixture.
     * 2. **And `mappings` does not belong here even with `label`.** `map-003` has `label: ""` *deliberately*
     *    and `MappingsPanel.tsx:180` renders `m.label || "—"` — an optional field with a designed empty
     *    state. Asserting it is non-empty would be asserting that the fixture may not exercise that state.
     *
     * So the property is scoped to the kinds that are **backed by a names index**, where an empty name is
     * always a bug and never a state: `rules`, `requests`, `sockets`, `webhooks`. Mappings are read as full
     * entities, which is why the string-vs-object bug could never touch them — and `mocks` is included
     * because it is the one full-entity kind whose `name` is required.
     *
     * **The general form:** a "no blank field" property has to be scoped to fields that are *required*, or it
     * reports designed empty states as defects.
     */
    const kinds: Array<[string, unknown]> = [
      ["proxyRules", cfg.proxyRules],
      ["requests", cfg.requests],
      ["wsConnections", cfg.wsConnections],
      ["mocks", cfg.mocks],
    ];
    const blank: string[] = [];
    for (const [kind, list] of kinds) {
      for (const item of Array.isArray(list) ? list : []) {
        if (!(item as any).name) blank.push(`${kind}/${(item as any).id}.name`);
      }
    }
    expect(blank).toEqual([]);
  });
});
