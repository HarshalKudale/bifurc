// @vitest-environment jsdom
/**
 * The card's status **after** a publish — the contradiction that was reported.
 *
 * > *"I click publish and get 'Nothing to publish — this entity is already committed'. So (1) if it is
 * > published why am I seeing Publish instead of Sync, (2) why does the card say not published, and
 * > (3) why is the button not disabled?"*
 *
 * All three are one fact: **the engine and the card disagree.** Git refuses to stage anything (so the
 * entity is committed and unchanged), while `entitySyncStatus` still says `new` — and `new` is exactly
 * what renders "Not published" and an enabled Publish.
 *
 * The two halves were each pinned separately and neither covered the join:
 * `mappingsPublish.test.tsx` faked the git actions, and `entityGitActions.test.tsx` asserted that
 * `refreshEntitySyncStatus` is *called* but never that the card re-reads its status. This mounts the
 * **real** hook and the **real** panel over one shared piece of state, which is what the app does.
 */
import React, { useCallback, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { AppConfig, EntitySyncStatus, LocalMapping, WorkspaceGitState } from "@/types";
import MappingsPanel from "@/panels/MappingsPanel";
import { useEntityGitActions } from "@/hooks/useEntityGitActions";

const WS = "ws-1";
const REL = "mappings/m1.json";
const PUSHED: WorkspaceGitState = { hasRemote: true, branch: "main", ahead: false, behind: false };

function makeMapping(): LocalMapping {
  return {
    id: "m1", workspaceId: WS, domain: "app.localhost",
    target: "127.0.0.1:3000", label: "My App", enabled: true,
  } as LocalMapping;
}

function makeConfig(): AppConfig {
  return {
    activeWorkspaceId: WS,
    workspaces: [{ id: WS, name: "Default", createdAt: 0, activeEnvironmentId: null }],
    mappings: [makeMapping()],
    proxyRules: [], ruleFolders: [], mocks: [], requests: [], mockFolders: [], requestFolders: [],
    wsConnections: [], wsFolders: [], webhooks: [], webhookFolders: [],
    graphqlRequests: [], graphqlSchemas: [], graphqlRequestFolders: [],
    grpcRequests: [], protoFiles: [], grpcRequestFolders: [],
    soapRequests: [], savedWsdls: [], soapRequestFolders: [], environments: [],
  } as unknown as AppConfig;
}

let publishEntity: ReturnType<typeof vi.fn>;
let getEntitySyncStatus: ReturnType<typeof vi.fn>;
/** What the engine's status map currently says — mutated by a successful publish, like a real repo. */
let files: Record<string, EntitySyncStatus>;

beforeEach(() => {
  files = { [REL]: "new" };
  publishEntity = vi.fn(async () => {
    // A real publish commits, and the next `git status` no longer lists the file.
    files = { [REL]: "clean" };
    return { ok: true, commit: "4f2a1c9e8b7d6a5f4e3d2c1b0a9f8e7d6c5b4a39" };
  });
  getEntitySyncStatus = vi.fn(async () => ({ files, git: PUSHED }));
  Object.defineProperty(window, "api", {
    value: {
      publishEntity,
      publishFolder: vi.fn(async () => ({ ok: true, commit: null })),
      gitDiscard: vi.fn(async () => ({ ok: true })),
      getConfig: vi.fn(async () => makeConfig()),
      getEntitySyncStatus,
    },
    configurable: true, writable: true,
  });
});

/**
 * The panel and the hook sharing one `entitySyncStatus`, wired the way `App.tsx` wires them.
 *
 * The sharing is the point: `App` owns the status state, hands it to the panel as a prop and to the
 * hook as `refreshEntitySyncStatus`. A test that gives each half its own copy cannot see whether a
 * write reaches the card.
 */
function Harness() {
  const [config, setConfig] = useState<AppConfig>(makeConfig);
  const [entitySyncStatus, setEntitySyncStatus] = useState<Record<string, EntitySyncStatus>>(files);
  const [workspaceGitState, setWorkspaceGitState] = useState<WorkspaceGitState | null>(PUSHED);

  const refreshConfig = useCallback(async () => { setConfig(await window.api.getConfig()); }, []);
  const refreshEntitySyncStatus = useCallback((wsId: string) => {
    void window.api.getEntitySyncStatus(wsId).then((payload: any) => {
      setEntitySyncStatus(payload.files ?? {});
      if (payload.git) setWorkspaceGitState(payload.git);
    });
  }, []);

  const git = useEntityGitActions({ wsConfig: config, wsId: WS, refreshConfig, refreshEntitySyncStatus, panel: "mappings" });

  return (
    <MappingsPanel
      config={config}
      onConfigChange={vi.fn().mockResolvedValue(undefined)}
      onRefreshServices={vi.fn()}
      entitySyncStatus={entitySyncStatus}
      workspaceGitState={workspaceGitState}
      onPublish={(id) => git.publish({ type: "entity", kind: "mappings", id })}
      onRevert={(id) => git.revert({ type: "entity", kind: "mappings", id })}
      isPublishing={() => false}
    />
  );
}

describe("MappingsPanel — the card and git must agree after a publish", () => {
  it("starts on 'Not published' with Publish, because the file is untracked", () => {
    render(<Harness />);

    expect(screen.getByText("Not published")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Publish" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sync" })).not.toBeInTheDocument();
  });

  /**
   * The reported contradiction, as three assertions. After one click the entity **is** committed, so:
   * the label must read Synced, the button must stop saying Publish, and it must not be offerable.
   */
  it("flips to Synced, and stops offering Publish, once the publish lands", async () => {
    render(<Harness />);

    fireEvent.click(screen.getByRole("button", { name: "Publish" }));

    await waitFor(() => expect(screen.getByText("Synced")).toBeInTheDocument());
    expect(screen.queryByText("Not published")).not.toBeInTheDocument();
    // (1) and (3): nothing left to publish, so no primary action is offered at all.
    expect(screen.queryByRole("button", { name: "Publish" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sync" })).not.toBeInTheDocument();
  });

  it("re-reads the status from the engine rather than trusting its own write", async () => {
    render(<Harness />);
    expect(getEntitySyncStatus).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Publish" }));

    // The status is derived from disk, so the only correct way to learn it is to ask again.
    await waitFor(() => expect(getEntitySyncStatus).toHaveBeenCalledWith(WS));
  });

  it("shows the commit id it was given, as proof the click did something", async () => {
    render(<Harness />);

    fireEvent.click(screen.getByRole("button", { name: "Publish" }));

    const notice = await screen.findByRole("status");
    expect(notice).toHaveTextContent("Published · 4f2a1c9");
    expect(notice).toHaveAttribute("title", "4f2a1c9e8b7d6a5f4e3d2c1b0a9f8e7d6c5b4a39");
  });

  /**
   * And the converse: if git refuses to stage a file the status calls `new`, that is **not** "already
   * committed" — the two disagree, and the message must not claim a state the UI itself contradicts.
   */
  it("does not claim 'already committed' when git staged nothing for an untracked file", async () => {
    // The engine reports success-with-no-commit, which is what the user saw…
    publishEntity.mockResolvedValue({ ok: true, commit: null });
    // …while the status still says the file is untracked. `files` is deliberately left as `new`.
    render(<Harness />);

    fireEvent.click(screen.getByRole("button", { name: "Publish" }));

    const notice = await screen.findByRole("status");
    // The status is re-read (it is still `new`), so the card must still show Publish — and the notice
    // must say that the engine and the status disagree rather than asserting the entity is committed.
    expect(notice).not.toHaveTextContent("already committed");
    expect(notice).toHaveTextContent(/disagree|still/i);
  });
});
