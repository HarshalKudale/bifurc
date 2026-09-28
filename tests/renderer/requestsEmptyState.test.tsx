// @vitest-environment jsdom
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom";
import RequestsPanel from "@/panels/RequestsPanel";
import ProtocolSelectorTab from "@/components/editor/ProtocolSelectorTab";
import { strings } from "@/lib/strings";
import type { AppConfig } from "@/types";

/**
 * The requests panel's empty state **is** the new-tab page (2026-09-28).
 *
 * It used to be `No requests open` + a hint, with the four protocol cards reachable only through
 * `+` / `Ctrl+T`. Now "no tab open" renders those cards directly, and picking one opens a tab that
 * already carries that protocol.
 *
 * What is under test is the **panel's wiring**, not the editors — the four protocol editors need a
 * real browser (CodeMirror, layout) and hang jsdom, so `RequestTabContent` is replaced by a probe
 * that reports the protocol it was handed. The reducers behind those editors are covered by
 * `restTabReducer` / `protocolTabReducers`.
 *
 * Each case is written so it can actually fail:
 *   - the cards by *name*, not by count — a renamed or added card is the failure mode
 *   - the deleted copy asserted **absent by literal**, so a revert is caught rather than tolerated
 *   - the `+` path, which is where widening `openNewTabInFolder(protocol?)` would have broken:
 *     `TabBar` handed the handler straight to `onClick`, so a `MouseEvent` would land in
 *     `draftProtocols[tabId]` as a truthy non-protocol — no branch matches it, the cards disappear
 *     and the tab renders blank. The guard is one line at that call site, so it needs a case.
 *   - `mode="mock"`, because `ProtocolSelectorTab` is **shared** with the Mocks panel and this change
 *     deleted its banner for both modes. The request half alone would not notice mock mode breaking.
 */
vi.mock("@/panels/requests/RequestTabContent", () => ({
  default: ({ tabId, activeProtocol }: { tabId: string; activeProtocol: string }) => (
    <div data-testid="tab-content" data-protocol={activeProtocol} data-for={tabId} />
  ),
}));

/**
 * The panel reads only the workspace id and the (empty) entity lists, so a partial cast is bounded by
 * what it touches; written in full it would be ~25 fields no assertion here depends on.
 */
const cfg = (over: Partial<AppConfig> = {}): AppConfig =>
  ({ activeWorkspaceId: "ws-1", workspaces: [{ id: "ws-1", name: "Default" }], ...over } as AppConfig);

const panel = () => render(<RequestsPanel config={cfg()} onConfigChange={vi.fn(async () => {})} />);

const tabStrip = (): HTMLElement[] =>
  Array.from(document.querySelectorAll<HTMLElement>("[data-tab-id]"));

const renderedProtocol = (): string | undefined =>
  screen.getByTestId("tab-content").dataset.protocol;

/** Card label → the protocol the panel must hand the tab's editor. */
const CARDS = [
  { card: /REST Request/i, badge: "REST", protocol: "rest" },
  { card: /GraphQL Operation/i, badge: "GQL", protocol: "graphql" },
  { card: /gRPC Call/i, badge: "gRPC", protocol: "grpc" },
  { card: /SOAP Request/i, badge: "SOAP", protocol: "soap" },
];

describe("RequestsPanel — the empty state is the new-tab page", () => {
  beforeEach(() => {
    localStorage.clear();
    // Nothing on these paths calls the bridge — `useMultiplexedEntities` is pure and the tab-loading
    // effect skips drafts — but the module reaches for `window.api` lazily, so it must exist.
    (window as any).api = {};
    // `SidebarLayout` wraps the panel in `react-resizable-panels`, whose mount does `new
    // ResizeObserver(...)`; jsdom does not provide it. Same stub as `codeEditor.test.tsx`, for the
    // same reason — mocking the layout away would take the panel out of its real container.
    if (!window.ResizeObserver) {
      window.ResizeObserver = class {
        observe()    {}
        unobserve()  {}
        disconnect() {}
      } as unknown as typeof ResizeObserver;
    }
    // `TabBar` scrolls the active tab into view once a tab opens; jsdom has no layout, so it has no
    // `scrollIntoView`. Without this the *first* case that opens a tab dies inside an effect.
    if (!Element.prototype.scrollIntoView) {
      Element.prototype.scrollIntoView = () => {};
    }
  });

  it("shows the four protocol cards, and opens no tab", () => {
    panel();

    for (const { card } of CARDS) {
      expect(screen.getByRole("button", { name: card })).toBeInTheDocument();
    }
    expect(tabStrip()).toHaveLength(0);
    expect(screen.queryByTestId("tab-content")).not.toBeInTheDocument();
  });

  it("no longer renders the old empty-state copy, nor the chooser banner", () => {
    panel();

    // Literals, not `strings.*`: `a618982`'s empty-state keys were deleted along with the copy, so
    // reading them back would now be a compile error rather than an assertion.
    expect(screen.queryByText("No requests open")).not.toBeInTheDocument();
    expect(screen.queryByText(/Click a request in the tree/)).not.toBeInTheDocument();
    // The banner was deleted outright, so this text exists nowhere in the app any more.
    expect(screen.queryByText(/Select Request Protocol/i)).not.toBeInTheDocument();
  });

  it.each(CARDS)("opens one tab on $protocol when $card is picked", ({ card, badge, protocol }) => {
    panel();

    fireEvent.click(screen.getByRole("button", { name: card }));

    const tabs = tabStrip();
    expect(tabs).toHaveLength(1);
    expect(tabs[0].textContent).toContain(badge);
    expect(renderedProtocol()).toBe(protocol);
    // The cards and a tab are mutually exclusive — the empty state is replaced, not stacked on.
    expect(screen.queryByRole("button", { name: /REST Request/i })).not.toBeInTheDocument();
  });

  it("leaves the chooser inside a tab opened with +, and reuses that tab", () => {
    panel();

    // The tab bar's `+`. This is the call site that must not forward its `MouseEvent`.
    fireEvent.click(screen.getAllByTitle(strings.requests.newTab).at(-1)!);

    expect(tabStrip()).toHaveLength(1);
    // Still on the chooser: no editor yet. Had the event reached `draftProtocols`, the cards would be
    // gone and this tab would render whatever a `MouseEvent` resolves to — nothing.
    expect(screen.queryByTestId("tab-content")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /SOAP Request/i })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /SOAP Request/i }));

    // Picking here changes *that* tab's protocol; it must not open a second one.
    expect(tabStrip()).toHaveLength(1);
    expect(renderedProtocol()).toBe("soap");
  });
});

/**
 * The shared component, in the mode the panel test never reaches.
 *
 * `ProtocolSelectorTab` serves both panels; the cases above only exercise `mode="request"`. Mocks went
 * REST-only in `plan/17` work item 8, and that filter is the one thing standing between mock mode and
 * an `onSelect("graphql")` for a kind that no longer exists — so it gets its own case rather than
 * riding on the request half.
 */
describe("ProtocolSelectorTab — mock mode", () => {
  it("offers REST alone, and no protocol that is no longer a creatable mock", () => {
    render(<ProtocolSelectorTab mode="mock" onSelect={vi.fn()} />);

    expect(screen.getByRole("button", { name: /REST Mock/i })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /GraphQL Mock/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /gRPC Mock/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /SOAP Mock/i })).not.toBeInTheDocument();
    // One card, not four — the count is what a filter regression would move.
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });

  it("reports the picked protocol to the caller", () => {
    const onSelect = vi.fn();
    render(<ProtocolSelectorTab mode="mock" onSelect={onSelect} />);

    fireEvent.click(screen.getByRole("button", { name: /REST Mock/i }));

    expect(onSelect).toHaveBeenCalledWith("rest");
  });
});
