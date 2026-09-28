// @vitest-environment jsdom
/**
 * The mapping card's git actions, clicked — the exact path the report was about.
 *
 * ## Why this file exists
 *
 * Reported: *"clicking on publish in mappings does nothing — that mapping should be committed, it still
 * says not published."* Every link in that chain was verified by reading and none of them was obviously
 * wrong, which is exactly when a test is cheaper than another read: the panel's `publish()` →
 * `panelFactory`'s binder → `useEntityGitActions.publish()` → `entityPath()` → `window.api.publishEntity`.
 *
 * So this mounts the **real** `MappingsPanel` with a real `useEntityGitActions`-shaped `onPublish` and
 * asserts the call that reaches the bridge — the payload the engine will actually receive. A wrong path
 * here (`mappings/<id>.json` is the only correct value for a flat kind) would make the engine stage
 * nothing and return `{ok: true}` for a no-op commit, which is indistinguishable from "nothing happened"
 * at the UI.
 */
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { AppConfig, LocalMapping, WorkspaceGitState } from "@/types";
import MappingsPanel from "@/panels/MappingsPanel";

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

function makeConfig(mappings: LocalMapping[]): AppConfig {
  return {
    port: 80,
    webhookPort: 9101,
    forwardProxyPort: 8080,
    companionPort: 4747,
    minimizeToTray: true,
    tlsEnabled: false,
    tlsCaCertPath: null,
    tlsCaKeyPath: null,
    workspaces: [{ id: WS, name: "Default", createdAt: 0, activeEnvironmentId: null }],
    activeWorkspaceId: WS,
    activeEnvironmentId: null,
    mappings,
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
  } as unknown as AppConfig;
}

const PUSHED: WorkspaceGitState = { hasRemote: true, branch: "main", ahead: false, behind: false };

let publishEntity: ReturnType<typeof vi.fn>;
let getConfig: ReturnType<typeof vi.fn>;

beforeEach(() => {
  publishEntity = vi.fn().mockResolvedValue({ ok: true });
  getConfig = vi.fn().mockResolvedValue(makeConfig([makeMapping()]));
  Object.defineProperty(window, "api", {
    value: { publishEntity, getConfig, gitDiscard: vi.fn().mockResolvedValue({ ok: true }) },
    configurable: true,
    writable: true,
  });
});

/** The props `panelFactory` binds for the mappings panel, with the call recording kept visible. */
function renderPanel({
  status,
  git = PUSHED,
}: {
  status?: "clean" | "modified" | "new" | "deleted";
  git?: WorkspaceGitState | null;
}) {
  const mapping = makeMapping();
  const onPublish = vi.fn((id: string) => {
    // The real binder resolves the path through `useEntityGitActions.entityPath`, whose only correct
    // answer for a flat kind is `mappings/<id>.json`.
    const path = `mappings/${id}.json`;
    return window.api.publishEntity(WS, [path]).then((r) => r as { ok: boolean; error?: string });
  });

  const utils = render(
    <MappingsPanel
      config={makeConfig([mapping])}
      onConfigChange={vi.fn().mockResolvedValue(undefined)}
      onRefreshServices={vi.fn()}
      entitySyncStatus={status ? { "mappings/m1.json": status } : {}}
      workspaceGitState={git}
      onPublish={onPublish}
      onRevert={vi.fn().mockResolvedValue({ ok: true })}
      isPublishing={() => false}
    />,
  );
  return { ...utils, onPublish, mapping };
}

describe("MappingsPanel — the publish button actually publishes", () => {
  it("shows Publish for an untracked mapping and commits its file", async () => {
    const { onPublish } = renderPanel({ status: "new" });

    const button = screen.getByRole("button", { name: "Publish" });
    fireEvent.click(button);

    await waitFor(() => expect(onPublish).toHaveBeenCalledWith("m1"));
    // The payload the engine receives. A path that does not match the file on disk makes
    // `publishEntities` stage nothing and return `{ok: true}` — a silent no-op.
    await waitFor(() =>
      expect(publishEntity).toHaveBeenCalledWith(WS, ["mappings/m1.json"]),
    );
  });

  it("shows Sync — not Publish — once the mapping is tracked, and commits the same file", async () => {
    const { onPublish } = renderPanel({ status: "modified" });

    expect(screen.queryByRole("button", { name: "Publish" })).not.toBeInTheDocument();
    const button = screen.getByRole("button", { name: "Sync" });
    fireEvent.click(button);

    await waitFor(() => expect(onPublish).toHaveBeenCalledWith("m1"));
    await waitFor(() => expect(publishEntity).toHaveBeenCalledWith(WS, ["mappings/m1.json"]));
  });

  it("offers Sync for a clean mapping whose commits are not on the remote", async () => {
    // The state the old model could not express: `clean` vs HEAD, ahead of the remote.
    renderPanel({ status: "clean", git: { hasRemote: true, branch: "main", ahead: true, behind: false } });

    expect(screen.getByRole("button", { name: "Sync" })).toBeInTheDocument();
  });

  it("offers nothing for a fully synced mapping", () => {
    renderPanel({ status: "clean" });

    expect(screen.queryByRole("button", { name: "Publish" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sync" })).not.toBeInTheDocument();
  });

  it("surfaces an engine refusal instead of looking like a no-op", async () => {
    publishEntity.mockResolvedValue({ ok: false, error: "no remote configured" });
    const { onPublish } = renderPanel({ status: "new" });

    fireEvent.click(screen.getByRole("button", { name: "Publish" }));

    await waitFor(() => expect(onPublish).toHaveBeenCalled());
    // A failed publish must say so. Before the uniform layer the folder-tree handlers ignored
    // `{ok:false}` entirely, which is how "does nothing" becomes the user-visible symptom.
    expect(await screen.findByRole("alert")).toHaveTextContent("no remote configured");
  });

  /**
   * The reported symptom, at its root: a publish that reaches the engine and stages nothing is a
   * **success** (`ok: true`), because an unchanged file must not produce an empty commit. It used to
   * be silent, which made it indistinguishable from a button that is not wired up.
   */
  /**
   * The message is only honest when the card **agrees** that the entity is published. The button that
   * can reach it is Sync, from a clean entity whose commits are not on the remote: the file matches
   * HEAD, so there is nothing to stage — and the entity is tracked, so "already committed" is true.
   */
  it("says 'nothing to publish' when git and the status agree there is nothing to do", async () => {
    publishEntity.mockResolvedValue({ ok: true, commit: null });
    renderPanel({ status: "clean", git: { hasRemote: true, branch: "main", ahead: true, behind: false } });

    fireEvent.click(screen.getByRole("button", { name: "Sync" }));

    expect(await screen.findByRole("status")).toHaveTextContent("Nothing to publish");
  });

  /**
   * …and when they **disagree**, it must say so rather than assert a state the card contradicts. A
   * Publish button sitting above the words "this entity is already committed" is a bug report, not a
   * state — and it was the exact shape of the report.
   */
  it("reports the disagreement instead of claiming the entity is committed", async () => {
    publishEntity.mockResolvedValue({ ok: true, commit: null });
    renderPanel({ status: "new" });

    fireEvent.click(screen.getByRole("button", { name: "Publish" }));

    const notice = await screen.findByRole("status");
    expect(notice).not.toHaveTextContent("already committed");
    expect(notice).toHaveTextContent(/disagree/i);
  });

  it("reports the commit id, so success is provable rather than claimed", async () => {
    const sha = "4f2a1c9e8b7d6a5f4e3d2c1b0a9f8e7d6c5b4a39";
    publishEntity.mockResolvedValue({ ok: true, commit: sha });
    renderPanel({ status: "new" });

    fireEvent.click(screen.getByRole("button", { name: "Publish" }));

    const notice = await screen.findByRole("status");
    // The 7-character form in the text — what `git log --oneline` prints…
    expect(notice).toHaveTextContent("Published · 4f2a1c9");
    // …and the whole id on hover, because a truncated hash with no way to read the rest loses it.
    expect(notice).toHaveAttribute("title", sha);
  });
});
