/**
 * The second axis of "is this entity synced?", and the deletion half of the publish model.
 *
 * Two things are pinned here, both of them measured against a **real** git repository in a temp
 * workspace rather than a mock — the behaviour is `git`'s, so a mock would only assert my own
 * assumptions back at me.
 *
 * ## 1. `getWorkspaceGitState` — the dimension the file map cannot see
 *
 * `getWorkspaceSyncStatus()` answers "does this file differ from HEAD?". A file committed locally but
 * never pushed does not differ from HEAD, so it reads `clean` — and the UI offered it **no button at
 * all**, which is why an entity could be committed and be impossible to push from its own row. That
 * is the whole of the "Publish/Sync is inconsistent" report, and `ahead` is what fixes it.
 *
 * The `ahead` cases are built with `git update-ref refs/remotes/origin/<branch>` rather than a real
 * remote: the measurement is against the **last known** remote state, deliberately without a fetch
 * (see `sync/gitState.ts` for why), so moving the ref is exactly what a fetch would have done.
 *
 * ## 2. A deleted *flat* kind must leave a recoverable stub
 *
 * `mappings` and `environments` are flat kinds and `deleteEntityCore` used to skip the git half of
 * the deletion entirely for them — so deleting a **published** mapping removed the file, removed it
 * from the config, and recorded nothing. The deletion was real on disk and invisible everywhere
 * else: no stub, no status, no way to recover it and no way to commit it from the UI. Folder kinds
 * never had that hole, which made it an inconsistency rather than a variant.
 *
 * Runs in the unit project. Each case builds its own temp workspace.
 */
import { describe, it, expect, beforeEach, afterEach, beforeAll } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import simpleGit from "simple-git";

const WS_ID = "ws-gitstate-test";
let tmpDir: string;

async function setup() {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lp-gitstate-test-"));

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
  // Best-effort. On Windows the safe-delete shim holds handles on the temp tree and `rmSync` throws
  // `EBUSY`, which vitest reports as a *test* failure even though the assertions all passed. The
  // directory is under `os.tmpdir()` and is the OS's to reap; asserting on it would only measure the
  // shim. (Same class of environment failure as the documented `gitStore.test.ts` teardown timeouts.)
  try { fs.rmSync(tmpDir, { recursive: true, force: true }); } catch { /* see above */ }
}

const git = () => simpleGit(path.join(tmpDir, WS_ID));
const branch = async () => (await git().raw(["rev-parse", "--abbrev-ref", "HEAD"])).trim();
const head = async () => (await git().revparse(["HEAD"])).trim();

async function addOrigin() {
  await git().raw(["remote", "add", "origin", "https://example.invalid/repo.git"]);
  const listed = (await git().raw(["remote"]))
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter(Boolean);
  // The setup asserts its own precondition. A silent failure here would surface as "no remote
  // configured" in the assertion under test, which is a different claim entirely.
  expect(listed).toContain("origin");
}

async function setRemoteRef(sha: string) {
  await git().raw(["update-ref", `refs/remotes/origin/${await branch()}`, sha]);
}

describe("getWorkspaceGitState — where the workspace stands relative to its remote", () => {
  beforeEach(setup);
  afterEach(teardown);

  it("reports no remote, and never claims to be ahead of one", async () => {
    const { getWorkspaceGitState } = await import("@bifurc/engine/sync/gitState");

    const state = await getWorkspaceGitState(WS_ID);

    expect(state.hasRemote).toBe(false);
    // A local-only workspace is "synced" by definition — there is nowhere to push, so the UI must not
    // grow a Sync button out of the absence of a remote.
    expect(state.ahead).toBe(false);
    expect(state.behind).toBe(false);
  });

  it("treats a configured-but-never-pushed branch as ahead", async () => {
    const { getWorkspaceGitState } = await import("@bifurc/engine/sync/gitState");
    const { getGit } = await import("@bifurc/engine/store/gitStore");
    await addOrigin();

    // The engine's own git handle must see the same remote the setup just added — if these disagree,
    // the failure is in `getGit`'s per-workspace cache, not in the state derivation.
    const engineRemotes = (await getGit(WS_ID).raw(["remote"])).trim();

    const state = await getWorkspaceGitState(WS_ID);

    expect(engineRemotes).toContain("origin");
    // One object comparison rather than four asserts, so a failure prints the whole state.
    expect({
      hasRemote: state.hasRemote,
      branch: state.branch,
      ahead: state.ahead,
      behind: state.behind,
    }).toEqual({ hasRemote: true, branch: await branch(), ahead: true, behind: false });
  });

  it("reports in-sync when HEAD matches the remote-tracking ref", async () => {
    const { getWorkspaceGitState } = await import("@bifurc/engine/sync/gitState");
    await addOrigin();
    await setRemoteRef(await head());

    const state = await getWorkspaceGitState(WS_ID);

    expect(state.ahead).toBe(false);
    expect(state.behind).toBe(false);
  });

  it("reports ahead when there is a local commit the remote ref lacks", async () => {
    const { getWorkspaceGitState } = await import("@bifurc/engine/sync/gitState");
    await addOrigin();
    await setRemoteRef(await head());

    fs.writeFileSync(path.join(tmpDir, WS_ID, "mocks", "x.json"), "{}", "utf-8");
    await git().raw(["add", "-A"]);
    await git().raw(["commit", "-m", "local work"]);

    const state = await getWorkspaceGitState(WS_ID);

    expect(state.ahead).toBe(true);
    expect(state.behind).toBe(false);
  });

  it("reports behind when the remote ref has a commit HEAD lacks", async () => {
    const { getWorkspaceGitState } = await import("@bifurc/engine/sync/gitState");
    await addOrigin();

    // A commit that exists only on the remote side: make it, remember it, then wind HEAD back.
    fs.writeFileSync(path.join(tmpDir, WS_ID, "mocks", "y.json"), "{}", "utf-8");
    await git().raw(["add", "-A"]);
    await git().raw(["commit", "-m", "their work"]);
    const theirCommit = await head();
    await git().raw(["reset", "--hard", "HEAD~1"]);
    await setRemoteRef(theirCommit);

    const state = await getWorkspaceGitState(WS_ID);

    expect(state.ahead).toBe(false);
    expect(state.behind).toBe(true);
  });
});

describe("deleting a flat kind — the deletion must survive until it is published", () => {
  let registry: any;
  let ctx: any;

  /**
   * `commandRegistry` is a **module-level singleton**, so registering per test throws
   * `Command "entity.create" is already registered` on the second case. Registered once here; the
   * kind containers (`entityCrudRegistry`, `simpleEntityKinds`) are singletons for the same reason.
   */
  beforeAll(async () => {
    const { commandRegistry } = await import("@bifurc/engine/kernel/registry");
    const { registerEntityCrudCommands } = await import("@bifurc/engine/store/entityCrudCommands");
    const { registerCrudKinds } = await import("@bifurc/engine/store/kinds/crudKinds");
    const { registerCoreKinds } = await import("@bifurc/engine/store/kinds/coreKinds");
    const { bus } = await import("@bifurc/engine/kernel/eventBus");

    registerCrudKinds();
    registerCoreKinds();
    registerEntityCrudCommands(commandRegistry);
    registry = commandRegistry;
    ctx = { bus };
  });

  async function createMapping(domain: string) {
    return await registry.invoke(
      "entity.create",
      { kind: "mappings", entity: { domain, target: "127.0.0.1:3000", label: domain }, workspaceId: WS_ID },
      ctx,
    ) as { id: string };
  }

  beforeEach(setup);
  afterEach(teardown);

  it("keeps a published mapping visible as a stub, with its deletion pending", async () => {
    const { syncChanges } = await import("@bifurc/engine/sync/gitOps");
    const { getPendingDeletions } = await import("@bifurc/engine/store/workspaceFs");
    const { loadConfig } = await import("@bifurc/engine/store/config");

    const created = await createMapping("tracked.localhost");
    // Make it tracked — this is what makes the deletion something that has to be *committed*.
    const published = await syncChanges(WS_ID, [`mappings/${created.id}.json`]);
    expect(published.ok).toBe(true);
    expect(fs.existsSync(path.join(tmpDir, WS_ID, "mappings", `${created.id}.json`))).toBe(true);

    await registry.invoke("entity.delete", { kind: "mappings", id: created.id }, ctx);

    // The file is gone…
    expect(fs.existsSync(path.join(tmpDir, WS_ID, "mappings", `${created.id}.json`))).toBe(false);
    // …the deletion is recorded…
    expect(getPendingDeletions(WS_ID, "mappings").map((p) => p.id)).toContain(created.id);
    // …and `loadConfig()` rebuilds a stub, so the entity stays visible and recoverable instead of
    // vanishing. `enabled: false` is what keeps the proxy from routing to a deleted mapping.
    const stub = (loadConfig().mappings ?? []).find((m: any) => m.id === created.id);
    expect(stub).toBeDefined();
    expect(stub!.enabled).toBe(false);
  });

  it("leaves no trace of an untracked mapping's deletion, because there is nothing to commit", async () => {
    const { getPendingDeletions } = await import("@bifurc/engine/store/workspaceFs");
    const { loadConfig } = await import("@bifurc/engine/store/config");

    const created = await createMapping("untracked.localhost");

    await registry.invoke("entity.delete", { kind: "mappings", id: created.id }, ctx);

    expect(fs.existsSync(path.join(tmpDir, WS_ID, "mappings", `${created.id}.json`))).toBe(false);
    // A pending deletion here would be a stub for an entity that was never published — a row the user
    // could "commit" into a commit that deletes a file git never had.
    expect(getPendingDeletions(WS_ID, "mappings").map((p) => p.id)).not.toContain(created.id);
    expect((loadConfig().mappings ?? []).find((m: any) => m.id === created.id)).toBeUndefined();
  });

  it("drops the stub once the deletion is published", async () => {
    const { syncChanges } = await import("@bifurc/engine/sync/gitOps");
    const { getPendingDeletions } = await import("@bifurc/engine/store/workspaceFs");
    const { loadConfig } = await import("@bifurc/engine/store/config");

    const created = await createMapping("commitme.localhost");
    await syncChanges(WS_ID, [`mappings/${created.id}.json`]);
    await registry.invoke("entity.delete", { kind: "mappings", id: created.id }, ctx);
    expect((loadConfig().mappings ?? []).some((m: any) => m.id === created.id)).toBe(true);

    const published = await syncChanges(WS_ID, [`mappings/${created.id}.json`]);
    expect(published.ok).toBe(true);

    // Publishing the deletion clears the record, and the stub goes with it — this is what finally
    // removes the entity from the UI, and it is why the record has to be written for flat kinds at
    // all: without it there is nothing to clear and the stub can never be retired.
    expect(getPendingDeletions(WS_ID, "mappings").map((p) => p.id)).not.toContain(created.id);
    expect((loadConfig().mappings ?? []).some((m: any) => m.id === created.id)).toBe(false);
  });

  it("never writes a names.json for a flat kind", async () => {
    const { syncChanges } = await import("@bifurc/engine/sync/gitOps");
    const { discardChanges } = await import("@bifurc/engine/sync/gitOps");

    const created = await createMapping("names.localhost");
    await syncChanges(WS_ID, [`mappings/${created.id}.json`]);

    fs.writeFileSync(
      path.join(tmpDir, WS_ID, "mappings", `${created.id}.json`),
      JSON.stringify({ id: created.id, domain: "names.localhost", target: "127.0.0.1:9999", label: "edited" }),
      "utf-8",
    );
    const reverted = await discardChanges(WS_ID, `mappings/${created.id}.json`);
    expect(reverted.ok).toBe(true);

    // Only foldered kinds have a names index. `upsertNameEntry` on a flat kind would create
    // `mappings/names.json` — untracked, invisible in the entity list, and then silently committed by
    // the next `git add -A -- mappings/`.
    expect(fs.existsSync(path.join(tmpDir, WS_ID, "mappings", "names.json"))).toBe(false);
  });
});
