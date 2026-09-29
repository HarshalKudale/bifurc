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
  const { setSettingsPathOverride } = await import("@bifurc/engine/workspace/appSettings");
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

  const { initWorkspaceDir } = await import("@bifurc/engine/workspace/workspaceFs");
  initWorkspaceDir(WS_ID, "Test Workspace");
  const { initWorkspaceRepo } = await import("@bifurc/engine/store/gitStore");
  await initWorkspaceRepo(WS_ID);
}

async function teardown() {
  const { setDataDirOverride } = await import("@bifurc/engine/store/gitStore");
  setDataDirOverride("");
  const { setSettingsPathOverride } = await import("@bifurc/engine/workspace/appSettings");
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
  const { commandRegistry } = await import("@bifurc/engine/kernel/registry");
  const { registerEntityCrudCommands } = await import("@bifurc/engine/workspace/entities/entityCrudCommands");
  const { registerCrudKinds } = await import("@bifurc/engine/workspace/entities/kinds/crudKinds");
  const { registerCoreKinds } = await import("@bifurc/engine/workspace/entities/kinds/coreKinds");
  const { registerSyncCommands } = await import("@bifurc/engine/sync/syncCommands");
  const { bus } = await import("@bifurc/engine/kernel/eventBus");

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
    const { loadConfig } = await import("@bifurc/engine/workspace/config");

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

  /**
   * A path that names nothing is a **failure**, not a no-op.
   *
   * This case used to pin `{ok: true, commit: null}` — the engine's old contract, which folded "git
   * refused to stage this" into "there was nothing to stage". That fold is what let a broken publish
   * present itself to the user as *"this entity is already committed"* while the card above it said
   * "Not published". The distinction is now enforced: nothing staged **and** a requested path that git
   * does not hold is an error, and the error names the path.
   */
  it("reports a failure, with the path, when git stages nothing for a path it does not hold", async () => {
    const result = await registry.invoke(
      "entity.publish",
      { workspaceId: WS_ID, paths: ["mappings/does-not-exist.json"] },
      ctx,
    );

    expect(result.ok).toBe(false);
    expect(result.error).toContain("mappings/does-not-exist.json");
    expect(result.error).toContain("does not exist");
    expect(await subjects()).not.toContain("create mapping undefined");
  });

  /**
   * The other side of that rule, and the one that keeps the distinction honest: an **unchanged, tracked**
   * file stays a silent success, because there the two agree — git holds the file and there is genuinely
   * nothing to do. It is also where the commit **id** is first asserted: a real publish must name its
   * commit, since the hash is the proof and `ok` is only a claim.
   */
  it("names the commit it made, and reports no commit for an unchanged file", async () => {
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

  /**
   * **The report, reproduced.** A stale `.git/index.lock`.
   *
   * A killed git process leaves the lock behind, and from then on every `git add` fails with
   * `Unable to create '…/.git/index.lock': File exists.` while `git status` keeps reading happily — so
   * the entity correctly looks untracked, the Publish button is correctly offered, and the publish
   * fails for a reason nothing surfaced. `publishEntities` swallowed it in a bare `catch {}` and
   * reported `commit: null`, which the UI rendered as *"this entity is already committed"*.
   *
   * The lock is written by hand here because that is exactly what the crash leaves: a file nobody will
   * ever release.
   */
  it("publishes anyway when a stale index.lock was left behind by a killed process", async () => {
    const { getWorkspaceSyncStatus } = await import("@bifurc/engine/sync/statusTracker");
    const created = await createMapping("locked.localhost");
    const relPath = `mappings/${created.id}.json`;
    const lock = path.join(tmpDir, WS_ID, ".git", "index.lock");
    fs.writeFileSync(lock, "");

    const result = await registry.invoke(
      "entity.publish",
      { workspaceId: WS_ID, paths: [relPath] },
      ctx,
    );

    expect(result.ok).toBe(true);
    expect(result.commit).toMatch(/^[0-9a-f]{40}$/);
    // Removed, not merely retried past — otherwise the next publish fails the same way, forever.
    expect(fs.existsSync(lock)).toBe(false);
    expect(await tracked()).toContain(relPath);
    expect((await getWorkspaceSyncStatus(WS_ID))[relPath]).toBe("clean");
  });
});
