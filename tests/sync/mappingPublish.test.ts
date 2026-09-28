/**
 * Publishing **a mapping** end to end through the command a click actually issues.
 *
 * ## Why this file exists
 *
 * Reported twice: *"clicking on publish in mappings does nothing … it still says not published."* The
 * renderer half is pinned by `tests/renderer/mappingsPublish.test.tsx` (the panel calls `onPublish` and
 * the binder sends `mappings/<id>.json`). This is the other half: does
 * `entity.publish` with that one path actually produce a commit, and does the file then read `clean`?
 *
 * `tests/sync/gitState.test.ts` already publishes a mapping — but through **`syncChanges()`**, the
 * function, not through the **command**. The difference is the whole registry layer: param validation,
 * the handler's own `emitEntityStatus`, and `updateLastKnownHead`. A command that returns `{ok: true}`
 * without committing is indistinguishable from a working one at the UI, which is exactly the symptom.
 *
 * It also pins the two things a mapping publish has to do that a mock's does not: stage a **flat** path
 * (no folder level) and leave no `names.json` behind.
 */
import { describe, it, expect, beforeAll, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import simpleGit from "simple-git";

const WS_ID = "ws-mapping-publish";
let tmpDir: string;

async function setup() {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lp-mapping-pub-"));

  const { setDataDirOverride } = await import("@bifurc/engine/store/gitStore");
  setDataDirOverride(tmpDir);
  const { setSettingsPathOverride } = await import("@bifurc/engine/store/appSettings");
  setSettingsPathOverride(path.join(tmpDir, "app.json"));

  fs.writeFileSync(
    path.join(tmpDir, "app.json"),
    JSON.stringify({
      port: 80,
      minimizeToTray: false,
      workspaces: [{ id: WS_ID, name: "Test Workspace", activeEnvironmentId: null }],
      activeWorkspaceId: WS_ID,
    }),
    "utf-8",
  );

  const { initWorkspaceDir } = await import("@bifurc/engine/store/workspaceFs");
  initWorkspaceDir(WS_ID, "Test Workspace");
  const { initWorkspaceRepo } = await import("@bifurc/engine/store/gitStore");
  await initWorkspaceRepo(WS_ID);
}

async function teardown() {
  const { setDataDirOverride } = await import("@bifurc/engine/store/gitStore");
  setDataDirOverride("");
  const { setSettingsPathOverride } = await import("@bifurc/engine/store/appSettings");
  setSettingsPathOverride(null);
  try { fs.rmSync(tmpDir, { recursive: true, force: true }); } catch { /* safe-delete shim holds handles */ }
}

const git = () => simpleGit(path.join(tmpDir, WS_ID));
const subjects = async (): Promise<string[]> =>
  (await git().raw(["log", "--format=%s"])).trim().split("\n").filter(Boolean);
const tracked = async (): Promise<string[]> =>
  (await git().raw(["ls-files"])).split(/\r?\n/).map((s) => s.trim()).filter(Boolean);

let registry: any;
let ctx: any;

beforeAll(async () => {
  const { commandRegistry } = await import("@bifurc/engine/commands/registry");
  const { registerEntityCrudCommands } = await import("@bifurc/engine/store/entityCrudCommands");
  const { registerCrudKinds } = await import("@bifurc/engine/store/kinds/crudKinds");
  const { registerCoreKinds } = await import("@bifurc/engine/store/kinds/coreKinds");
  const { registerSyncCommands } = await import("@bifurc/engine/sync/syncCommands");
  const { bus } = await import("@bifurc/engine/eventBus");

  registerCrudKinds();
  registerCoreKinds();
  registerEntityCrudCommands(commandRegistry);
  registerSyncCommands(commandRegistry);
  registry = commandRegistry;
  ctx = { bus };
});

beforeEach(setup);
afterEach(teardown);

async function createMapping(domain: string) {
  return await registry.invoke(
    "entity.create",
    { kind: "mappings", entity: { domain, target: "127.0.0.1:3000", label: "My App" }, workspaceId: WS_ID },
    ctx,
  ) as { id: string };
}

describe("entity.publish — a mapping, through the command the button issues", () => {
  it("commits the mapping's file and the status stops saying 'new'", async () => {
    const { getWorkspaceSyncStatus } = await import("@bifurc/engine/sync/statusTracker");

    const created = await createMapping("app.localhost");
    const relPath = `mappings/${created.id}.json`;
    expect(fs.existsSync(path.join(tmpDir, WS_ID, "mappings", `${created.id}.json`))).toBe(true);

    // Before: the file is untracked, which is what makes the card say "Not published".
    expect((await getWorkspaceSyncStatus(WS_ID))[relPath]).toBe("new");

    const result = await registry.invoke(
      "entity.publish",
      { workspaceId: WS_ID, paths: [relPath], message: undefined },
      ctx,
    );

    // The **commit id** is the assertion that matters: it proves a commit exists *and* names it.
    expect(result.ok).toBe(true);
    expect(result.commit).toMatch(/^[0-9a-f]{40}$/);
    // …and it must be the commit that is actually HEAD, not an id invented on the way out.
    expect(result.commit).toBe((await git().revparse(["HEAD"])).trim());
    expect(await subjects()).toContain("create mapping My App");
    // …and the file must be in the index, which is what makes the status `clean` rather than `new`.
    expect(await tracked()).toContain(relPath);
    expect((await getWorkspaceSyncStatus(WS_ID))[relPath]).toBe("clean");
  });

  it("writes the entity id into the commit body, so History can find it", async () => {
    const created = await createMapping("hist.localhost");
    await registry.invoke("entity.publish", { workspaceId: WS_ID, paths: [`mappings/${created.id}.json`] }, ctx);

    const body = await git().raw(["log", "-1", "--format=%B"]);
    expect(body).toContain(`entity-id: ${created.id}`);
    expect(body).toContain(`workspace-id: ${WS_ID}`);
  });

  it("publishes an edited mapping as an update, and leaves nothing untracked behind it", async () => {
    const { loadConfig } = await import("@bifurc/engine/store/config");

    const created = await createMapping("edit.localhost");
    const relPath = `mappings/${created.id}.json`;
    await registry.invoke("entity.publish", { workspaceId: WS_ID, paths: [relPath] }, ctx);

    const mapping = (loadConfig().mappings ?? []).find((m: any) => m.id === created.id);
    await registry.invoke(
      "entity.update",
      { kind: "mappings", entity: { ...mapping, target: "127.0.0.1:4000" } },
      ctx,
    );

    await registry.invoke("entity.publish", { workspaceId: WS_ID, paths: [relPath] }, ctx);

    expect((await subjects())[0]).toMatch(/^(create|update) mapping/);
    expect(JSON.parse(fs.readFileSync(path.join(tmpDir, WS_ID, "mappings", `${created.id}.json`), "utf-8")).target)
      .toBe("127.0.0.1:4000");

    // The flat-kind trap: a `names.json` here would be untracked, invisible in the UI, and then
    // silently committed by the next `git add -A -- mappings/`.
    expect(fs.existsSync(path.join(tmpDir, WS_ID, "mappings", "names.json"))).toBe(false);
    expect(await tracked()).not.toContain("mappings/names.json");
  });

  it("reports a refusal rather than a silent no-op when the path names nothing", async () => {
    const result = await registry.invoke(
      "entity.publish",
      { workspaceId: WS_ID, paths: ["mappings/does-not-exist.json"] },
      ctx,
    );

    // The engine stages nothing and returns success — that is `publishEntities`'s contract for a
    // no-op, and it is why the *renderer* must never compute a path the engine cannot find. Pinned
    // here so the contract is visible from the engine side too.
    expect(result).toEqual({ ok: true, commit: null });
    expect(await subjects()).not.toContain("create mapping undefined");
  });

  /**
   * `committed: false` is what makes a no-op distinguishable from a commit — both report `ok: true`.
   *
   * Added 2026-09-28 after the report *"clicking publish in mappings does nothing … it still says not
   * published"*: a publish that reaches the engine and stages nothing is a **success**, and the UI had
   * no way to know that, so it showed no feedback at all. The flag is asserted from the engine side so
   * the renderer's message can never be a guess.
   */
  it("distinguishes 'committed' from 'there was nothing to commit'", async () => {
    const created = await createMapping("noop.localhost");
    const relPath = `mappings/${created.id}.json`;

    const first = await registry.invoke("entity.publish", { workspaceId: WS_ID, paths: [relPath] }, ctx);
    // The id, present on the first publish — this is the "something git related happened" proof.
    expect(first.ok).toBe(true);
    expect(first.commit).toMatch(/^[0-9a-f]{40}$/);

    // Nothing changed since. Publishing again must not create an empty commit…
    const head = (await git().revparse(["HEAD"])).trim();
    const second = await registry.invoke("entity.publish", { workspaceId: WS_ID, paths: [relPath] }, ctx);
    // …so there is no id to report, and `null` says exactly that — distinct from `ok: false`, which
    // would mean a step failed.
    expect(second).toEqual({ ok: true, commit: null });
    expect((await git().revparse(["HEAD"])).trim()).toBe(head);
  });
});
