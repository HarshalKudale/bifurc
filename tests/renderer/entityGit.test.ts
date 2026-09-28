/**
 * The uniform git-tracking vocabulary — the spec for what every surface offers.
 *
 * `lib/entityGit.ts` exists because seven surfaces each answered "what can the user do with this
 * entity?" independently and disagreed: four vocabularies for two actions, and four different gates
 * for History. These cases pin the answer so a new surface cannot invent a fifth.
 *
 * The load-bearing case is `clean + ahead`. That is the state the old model could not express at
 * all — a file that matches HEAD while HEAD is not on the remote reads `clean`, so the entity was
 * offered **no button** and could not be pushed from its own row. It is the whole of the
 * "Publish/Sync is inconsistent" report.
 */
import { describe, it, expect } from "vitest";
import {
  entityGitView,
  entityGitDotColor,
  folderGitRollup,
  panelEntityKind,
  panelSingletonPath,
  foldersForKind,
} from "@/lib/entityGit";
import type { EntitySyncStatus, WorkspaceGitState } from "@/types";

const NO_REMOTE: WorkspaceGitState = { hasRemote: false, branch: null, ahead: false, behind: false };
const PUSHED: WorkspaceGitState = { hasRemote: true, branch: "main", ahead: false, behind: false };
const AHEAD: WorkspaceGitState = { hasRemote: true, branch: "main", ahead: true, behind: false };

describe("entityGitView — one answer to 'what can I do with this entity?'", () => {
  it("offers Publish, and only Publish, for an untracked entity", () => {
    const v = entityGitView("new", PUSHED);
    expect(v.primary).toBe("publish");
    expect(v.primaryEnabled).toBe(true);
    // Revert means "reset to the last synced version" — an untracked entity has no such version, so
    // discarding it would be a *delete*, which is a different action with its own confirmation.
    expect(v.canRevert).toBe(false);
    // History of an entity with no commits is an empty panel.
    expect(v.canViewHistory).toBe(false);
    // The deletion cannot be undone, because there is nothing committed to restore from.
    expect(v.deleteIsRecoverable).toBe(false);
  });

  it("offers Sync and Revert for local work, and a recoverable delete", () => {
    for (const status of ["modified", "deleted"] as EntitySyncStatus[]) {
      const v = entityGitView(status, PUSHED);
      expect(v.primary).toBe("sync");
      expect(v.canRevert).toBe(true);
      expect(v.canViewHistory).toBe(true);
      expect(v.deleteIsRecoverable).toBe(true);
    }
  });

  it("offers nothing for a fully synced entity", () => {
    const v = entityGitView("clean", PUSHED);
    expect(v.primary).toBeNull();
    expect(v.primaryEnabled).toBe(false);
    expect(v.canRevert).toBe(false);
    expect(v.canViewHistory).toBe(true);
  });

  /**
   * The missing dimension. A committed-but-unpushed entity is `clean` as far as `git status` is
   * concerned, so the file-status axis alone cannot see it — only `WorkspaceGitState.ahead` can.
   */
  it("offers Sync for a clean entity whose commits are not on the remote", () => {
    const v = entityGitView("clean", AHEAD);
    expect(v.primary).toBe("sync");
    expect(v.primaryEnabled).toBe(true);
    // Nothing to revert: the file already matches HEAD. Only the *commit* is outstanding.
    expect(v.canRevert).toBe(false);
    expect(v.stateLabel).toBe("Unpushed commits");
  });

  it("never offers Sync-for-unpushed when there is no remote to push to", () => {
    expect(entityGitView("clean", NO_REMOTE).primary).toBeNull();
    // …and a missing remote must not suppress what a known file status already justifies.
    expect(entityGitView("modified", NO_REMOTE).primary).toBe("sync");
    expect(entityGitView("new", NO_REMOTE).primary).toBe("publish");
  });

  /**
   * Reads fail open, writes fail closed. An unknown status means "the status map has not loaded, or
   * this path is not in it" — not "clean". A write on that basis risks a commit the user did not ask
   * for; a *read* destroys nothing, and refusing it would make the History button flicker off on
   * every status refresh — including the refresh that opening History triggers.
   */
  it("refuses every write but allows History when the status is unknown", () => {
    for (const git of [null, undefined, PUSHED, AHEAD]) {
      const v = entityGitView(undefined, git);
      expect(v.known).toBe(false);
      expect(v.primary).toBeNull();
      expect(v.canRevert).toBe(false);
      expect(v.canViewHistory).toBe(true);
      expect(v.deleteIsRecoverable).toBe(true);
    }
  });

  it("labels the two writers with one vocabulary", () => {
    expect(entityGitView("new", PUSHED).primaryLabel).toBe("Publish");
    expect(entityGitView("modified", PUSHED).primaryLabel).toBe("Sync");
    expect(entityGitView("clean", AHEAD).primaryLabel).toBe("Sync");
    expect(entityGitView("clean", PUSHED).primaryLabel).toBe("");
  });
});

describe("entityGitDotColor", () => {
  it("marks a create or a delete red and an edit amber", () => {
    expect(entityGitDotColor("new")).toBe("red");
    expect(entityGitDotColor("deleted")).toBe("red");
    expect(entityGitDotColor("modified")).toBe("yellow");
    expect(entityGitDotColor("clean")).toBe("green");
    expect(entityGitDotColor(undefined)).toBe("green");
  });
});

describe("folderGitRollup — a group's status, from the same map the rows use", () => {
  const files: Record<string, EntitySyncStatus> = {
    "mocks/A/new.json": "new",
    "mocks/A/mod.json": "modified",
    "mocks/A/Sub/deep.json": "new",
    "mocks/B/clean.json": "clean",
    "requests/r.json": "new",
  };

  it("counts a folder's descendants, not just its own files", () => {
    const r = folderGitRollup(["mocks/A/"], files, PUSHED);
    // `mocks/A/Sub/deep.json` is included: "Publish All" on a parent has to mean the subtree.
    expect(r.changed).toBe(3);
  });

  it("says Publish only when every outstanding file is unpublished", () => {
    expect(folderGitRollup(["mocks/A/"], { "mocks/A/a.json": "new" }, PUSHED).primary).toBe("publish");
    // One edit among the new files makes the group's verb the general one.
    expect(folderGitRollup(["mocks/A/"], { "mocks/A/a.json": "new", "mocks/A/b.json": "modified" }, PUSHED).primary).toBe("sync");
  });

  it("offers nothing for a clean folder, and Sync for an unpushed one", () => {
    expect(folderGitRollup(["mocks/B/"], files, PUSHED).primary).toBeNull();
    expect(folderGitRollup(["mocks/B/"], files, AHEAD).primary).toBe("sync");
  });

  it("does not count another kind's files under the same prefix shape", () => {
    const r = folderGitRollup(["mocks/"], files, PUSHED);
    expect(r.changed).toBe(3);
    expect(folderGitRollup(["requests/"], files, PUSHED).changed).toBe(1);
  });
});

describe("the panel → entity-kind map", () => {
  it("names every panel that publishes something, including the ones the old maps missed", () => {
    // `webhooks` was in the publish handler and in neither footer map, so the webhooks panel showed
    // "up to date" with unpublishable work on screen and no button to publish it with.
    expect(panelEntityKind("webhooks")?.kind).toBe("webhooks");
    // The requests panel is multiplexed across four storage kinds; a single prefix missed three.
    expect(panelEntityKind("requests")?.prefixes).toContain("graphqlRequests/");
    expect(panelEntityKind("requests")?.prefixes).toContain("soapRequests/");
    expect(panelEntityKind("requests")?.prefixes).toContain("grpcRequests/");
    expect(panelEntityKind("settings")).toBeNull();
  });

  it("treats healthbar as a singleton file rather than a folder", () => {
    expect(panelSingletonPath("healthbar")).toBe("healthbar/services.json");
    expect(panelSingletonPath("mocks")).toBeNull();
  });

  it("reads a kind's folder list from the right config key, and none for a flat kind", () => {
    const cfg = {
      requestFolders: [{ id: "f1", name: "F" }],
      mockFolders: [{ id: "f2", name: "M" }],
    };
    expect(foldersForKind(cfg, "requests")).toEqual([{ id: "f1", name: "F" }]);
    expect(foldersForKind(cfg, "mocks")).toEqual([{ id: "f2", name: "M" }]);
    // `null` for a flat kind is what stops `entityRelPath()` producing a root-level path for an
    // entity that is really in a subfolder.
    expect(foldersForKind(cfg, "mappings")).toEqual([]);
    expect(foldersForKind(cfg, "environments")).toEqual([]);
  });
});
