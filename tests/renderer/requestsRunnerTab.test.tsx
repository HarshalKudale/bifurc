// @vitest-environment jsdom
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom";
import RequestsPanel from "@/panels/RequestsPanel";
import { strings } from "@/lib/strings";
import type { AppConfig, Folder, SavedRequest } from "@/types";

/**
 * The collection runner tab's **call site** in `RequestsPanel` (2026-09-28).
 *
 * `b594c51` ("Very major refactor") rewrote the runner-tab branch and dropped four of
 * `CollectionRunner`'s props, passing `folders={folders}` instead. `useCollectionRunner` seeds
 * `useState(requests)`, so `requests` arriving as `undefined` made `orderedRequests` undefined — and
 * `moveDown`'s `useCallback` **dependency array** reads `orderedRequests.length` *during render*.
 * Opening "Run Collection" on any folder therefore threw
 * `TypeError: Cannot read properties of undefined (reading 'length')` and took the whole requests
 * panel down with it.
 *
 * That crash is why this file renders the panel and not the component: `CollectionRunner` was always
 * correct, and its own logic is covered by `collectionRunner.test.ts`. Every prop the fix restored
 * gets a case here, because they fail differently — a missing `requests` crashes loudly, but a
 * missing `wsId` or `onSaveReport` fails *silently*, and a missing `folderName` renders "Collection"
 * for every folder in the app.
 *
 * **Every assertion is scoped to the runner**, because the sidebar tree keeps all of its rows
 * mounted: a folder collapses by CSS (`grid-template-rows: 0fr`), not by unmounting, so every
 * request name in the workspace is already in the DOM behind the panel. An unscoped
 * `getByText("Charge")` finds the sidebar's copy too — which is also why the negative cases below
 * are only meaningful once scoped.
 *
 * The typecheck already reported this — `RequestsPanel.tsx(144,23): error TS2322 … Property 'folders'
 * does not exist on type 'IntrinsicAttributes & CollectionRunnerProps'` — inside the 33 pre-existing
 * errors of `npm run typecheck --workspace @bifurc/gui`. `packages/gui` is not in `npm run
 * typecheck`, so nothing was ever red on it. This file is the gate that does not depend on anyone
 * reading that list.
 */

vi.mock("@/panels/requests/RequestTabContent", () => ({
  default: () => <div data-testid="tab-content" />,
}));

const folder = (id: string, name: string, parentId: string | null = null): Folder => ({
  id, name, parentId, createdAt: 0, workspaceId: "ws-1",
});

const request = (over: Partial<SavedRequest> & { id: string }): SavedRequest => ({
  name: over.id,
  method: "POST",
  url: "http://api.test/x",
  headers: {},
  body: "",
  createdAt: 0,
  workspaceId: "ws-1",
  folderId: null,
  ...over,
});

/**
 * Two folders, four requests — `f1` owns two, `f2` one, and one belongs to no folder. The counts are
 * the point: **2** means the folder filter ran, **4** means it did not, **3** means the
 * `folderId: null` case slipped in, and **0** means `requests` never arrived.
 */
const config = (): AppConfig => ({
  activeWorkspaceId: "ws-1",
  workspaces: [{ id: "ws-1", name: "Default" }],
  requestFolders: [folder("f1", "Payments"), folder("f2", "Search")],
  requests: [
    request({ id: "r1", name: "Charge", folderId: "f1", url: "http://api.test/charge" }),
    request({ id: "r2", name: "Refund", folderId: "f1", url: "http://api.test/refund" }),
    request({ id: "r3", name: "Lookup", folderId: "f2", url: "http://api.test/lookup" }),
    request({ id: "r4", name: "Loose", folderId: null, url: "http://api.test/loose" }),
  ],
} as unknown as AppConfig);

type ApiStub = Record<string, ReturnType<typeof vi.fn>>;
let api: ApiStub;

beforeEach(() => {
  localStorage.clear();
  api = {
    loadRunnerConfig: vi.fn(async () => null),
    saveRunnerConfig: vi.fn(async () => undefined),
    // `handleRun` re-reads each request in full before sending it, keyed by `wsId`.
    loadEntity: vi.fn(async (_ws: string, _kind: string, id: string) => ({
      ok: true,
      entity: config().requests.find((r) => r.id === id),
    })),
    replayRequest: vi.fn(async () => ({ status: 200, headers: {}, body: "" })),
    saveRunnerReport: vi.fn(async () => ({ ok: true })),
  };
  (window as any).api = api;
  // Same three jsdom gaps as `requestsEmptyState.test.tsx`, for the same reasons.
  if (!window.ResizeObserver) {
    window.ResizeObserver = class {
      observe()    {}
      unobserve()  {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  }
  if (!Element.prototype.scrollIntoView) Element.prototype.scrollIntoView = () => {};
});

const panel = () => render(<RequestsPanel config={config()} onConfigChange={vi.fn(async () => {})} />);

/** Open the runner the way a user does: right-click the folder → "Run Collection". */
const openRunner = (folderName: string) => {
  fireEvent.contextMenu(screen.getByText(folderName));
  fireEvent.click(screen.getByRole("button", { name: strings.folderTree.runCollection }));
};

/**
 * The runner's left-hand request list, located by its "drag to reorder" hint — the only string in the
 * app that appears on exactly that element. Scoping through it is what makes the negatives below
 * assertions about the *collection* rather than about the workspace.
 */
const collectionList = () =>
  within(screen.getByText(strings.collectionRunner.dragToReorder).parentElement!.parentElement!);

describe("RequestsPanel — the collection runner tab", () => {
  it("scopes the run to the folder it was opened on", async () => {
    panel();
    openRunner("Payments");
    // The runner loads its per-folder config in an effect; let that promise settle so its state
    // update lands inside `act` rather than warning about it.
    await act(async () => {});

    // The header count is the sharp instrument for the filter (see the `config()` note above); the
    // folder name rides on the same element, so this pins `folderName` without matching the sidebar's
    // identically-named row.
    const header = screen.getByText(/2 requests/).parentElement!;
    expect(header.textContent).toContain("Payments");

    expect(collectionList().getByText("Charge")).toBeInTheDocument();
    expect(collectionList().getByText("Refund")).toBeInTheDocument();
    // The other folder's request, and the one in no folder — both of which the sidebar *is* showing,
    // so these fail if the filter is dropped rather than if the tree happens to be collapsed.
    expect(collectionList().queryByText("Lookup")).not.toBeInTheDocument();
    expect(collectionList().queryByText("Loose")).not.toBeInTheDocument();
  });

  it("tells the runner which workspace and folder it is running", async () => {
    panel();
    openRunner("Payments");

    // `wsId` is invisible in the UI — it only ever reaches the runner config, so this is the one
    // place a dropped `wsId` shows up at all.
    await waitFor(() => expect(api.loadRunnerConfig).toHaveBeenCalledWith("ws-1", "f1"));
  });

  it("runs the folder's requests and hands the report to the panel", async () => {
    panel();
    openRunner("Payments");

    fireEvent.click(screen.getByRole("button", { name: strings.collectionRunner.run }));

    await waitFor(() => expect(api.saveRunnerReport).toHaveBeenCalled());

    // Both of the folder's requests were hydrated (through `wsId`) and sent — and only those two.
    expect(api.loadEntity).toHaveBeenCalledWith("ws-1", "requests", "r1");
    expect(api.loadEntity).toHaveBeenCalledWith("ws-1", "requests", "r2");
    expect(api.replayRequest).toHaveBeenCalledTimes(2);
    expect(api.replayRequest.mock.calls[0][1]).toBe("http://api.test/charge");

    // `onSaveReport` is the only path that persists a run; without it the report dies with the tab.
    const [wsId, report] = api.saveRunnerReport.mock.calls[0] as [string, any];
    expect(wsId).toBe("ws-1");
    expect(report.folderId).toBe("f1");
    expect(report.totalRequests).toBe(2);
    expect(report.results.map((r: any) => r.requestName)).toEqual(["Charge", "Refund"]);

    // The UI agrees: a summary bar, a result card per request, and the export affordance. The cards
    // are located by the resolved URL, which only they render — the list rows carry name and method.
    expect(screen.getByText(strings.collectionRunner.allPassed)).toBeInTheDocument();
    expect(screen.getByText("http://api.test/charge")).toBeInTheDocument();
    expect(screen.getByText("http://api.test/refund")).toBeInTheDocument();
    expect(screen.getByTitle(strings.collectionRunner.exportTitle)).toBeInTheDocument();
  });
});
