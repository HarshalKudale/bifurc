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
 * correct, and its own logic **used to be** covered by `collectionRunner.test.ts` — that file went with the
 * renderer's run loop on 2026-09-30 (`plan/19` WI 4), and the loop's behaviour is now
 * `tests/runner/runRegistry.test.ts`'s subject. Every prop the fix restored
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
    // The run's own commands and events (`plan/19` WI 4). The hook asks the engine what it knows on mount
    // (`runnerStatus`) and then subscribes; it no longer loops, which is why there is no `loadEntity` /
    // `replayRequest` stub here any more — the engine reads the entities and sends the requests.
    runnerStatus: vi.fn(async () => null),
    runnerStart: vi.fn(async () => ({ ok: true, total: 0 })),
    runnerCancel: vi.fn(async () => ({ ok: true })),
    exportRunnerReport: vi.fn(async () => undefined),
    onRunnerProgress: vi.fn(() => () => {}),
    onRunnerFinished: vi.fn(() => () => {}),
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

  it("starts the run in the engine and renders the report the engine emits", async () => {
    panel();
    openRunner("Payments");

    fireEvent.click(screen.getByRole("button", { name: strings.collectionRunner.run }));

    /**
     * **The run is the engine's now** (`plan/19` WI 4, D29/D30). This case used to assert that the renderer
     * hydrated each request through `loadEntity`, replayed it, and saved the report — that loop is gone, so
     * what it asserts instead is the contract that replaced it: the UI *asks* the engine to start, and
     * renders what the engine emits.
     */
    await waitFor(() => expect(api.runnerStart).toHaveBeenCalledWith("ws-1", "f1"));

    // The hook re-reads the engine's state after a successful start, because the engine's answer is the
    // authority — it does not construct a snapshot of its own.
    await waitFor(() => expect(api.runnerStatus).toHaveBeenCalledWith("ws-1", "f1"));

    const report = {
      folderId: "f1",
      folderName: "Payments",
      startedAt: 1,
      completedAt: 2,
      totalRequests: 2,
      totalTests: 0,
      passedTests: 0,
      failedTests: 0,
      results: [
        { requestId: "r1", requestName: "Charge", method: "POST", url: "http://api.test/charge", status: 200, responseTime: 3, tests: [], testLogs: [] },
        { requestId: "r2", requestName: "Refund", method: "POST", url: "http://api.test/refund", status: 200, responseTime: 4, tests: [], testLogs: [] },
      ],
    };
    const onFinished = api.onRunnerFinished.mock.calls[0][0] as (p: unknown) => void;
    await act(async () => { onFinished({ workspaceId: "ws-1", folderId: "f1", report }); });

    // `onSaveReport` is the only path that persists a run; without it the report dies with the tab.
    await waitFor(() => expect(api.saveRunnerReport).toHaveBeenCalledWith("ws-1", report));

    // The UI agrees: a summary bar, a result card per request, and the export affordance. The cards
    // are located by the resolved URL, which only they render — the list rows carry name and method.
    expect(screen.getByText(strings.collectionRunner.allPassed)).toBeInTheDocument();
    expect(screen.getByText("http://api.test/charge")).toBeInTheDocument();
    expect(screen.getByText("http://api.test/refund")).toBeInTheDocument();
    expect(screen.getByTitle(strings.collectionRunner.exportTitle)).toBeInTheDocument();
  });
});
