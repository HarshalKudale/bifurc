// @vitest-environment jsdom
import React from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom";
import { UrlBar, UrlBarVerb } from "@/components/editor/RequestTab";
import { TabToolbar, TabToolbarRow } from "@/components/editor/TabPage";

/**
 * The address row's two structural invariants, pinned after a real regression (2026-09-28).
 *
 * ## What went wrong, because the shape of the bug is the reason this file exists
 *
 * Unifying the four protocol tab pages moved `UrlBar` from "a block-level child of a column" to "a
 * flex item inside a `TabToolbarRow`". `bare` was added to drop the bar's own padding and border, so
 * the toolbar could own the band's frame — but `bare` dropped the **width** with them. As a flex item
 * with no `flex-1` the bar collapsed to its content width, so the URL input was short and the row
 * kept the remainder as dead space to the right of Send. Every protocol was affected; the user
 * spotted it immediately on the running app.
 *
 * A class assertion is a weak instrument in general, and this file leans on one deliberately:
 * **jsdom has no layout engine**, so there is no computed width to assert against. What *is* testable
 * is the mechanism — that the bar is a growing flex item — and that is precisely what was wrong.
 * Stating that plainly beats a test that pretends to measure.
 *
 * ## ⚠ Why this renders `UrlBar` directly instead of a real protocol tab
 *
 * Rendering the real `GraphQLTab` / `SoapTab` / `GrpcTab` in jsdom **hangs the vitest worker** — the
 * protocol editors mount CodeMirror, and the symptom is `Worker exited unexpectedly` after a ~115 s
 * hang, or a bare `SIGTERM` with no test output at all. `UrlBar` and the band components are pure
 * presentational React with no editor in them, so they render in milliseconds. Keep it that way: if
 * you need to cover a whole tab page, go through `tests/renderer/requestsRunnerTab.test.tsx`'s
 * pattern (render the panel, stub `RequestTabContent`).
 */

const noop = () => {};

function renderBar(extra: Partial<React.ComponentProps<typeof UrlBar>> = {}) {
  return render(
    <TabToolbar>
      <TabToolbarRow>
        <UrlBar
          bare
          url="https://example.test/orders"
          onUrlChange={noop}
          urlPlaceholder="https://example.test/orders"
          actionLabel="Send"
          actionLoadingLabel="Sending"
          actionLoading={false}
          actionDisabled={false}
          onAction={noop}
          activeEnv={null}
          {...extra}
        />
      </TabToolbarRow>
    </TabToolbar>,
  );
}

/**
 * The bar's own root, found structurally rather than by class name soup.
 *
 * `input` → `div.flex.items-stretch` (the bordered address group) → the `UrlBar` root. Using
 * `closest` on the group rather than on a bare `div` keeps this honest if the group's classes change.
 */
function urlBarRoot(): HTMLElement {
  const input = screen.getByPlaceholderText("https://example.test/orders");
  const group = input.closest("div.flex.items-stretch");
  expect(group).not.toBeNull();
  const root = group!.parentElement;
  expect(root).not.toBeNull();
  return root as HTMLElement;
}

describe("UrlBar inside a TabToolbarRow", () => {
  it("stretches to fill the row — dropping the padding must not drop the width", () => {
    renderBar();
    const root = urlBarRoot();

    // The regression: `bare` was `flex items-center gap-2`, with no grow. The bar was then only as
    // wide as its contents and the row kept the rest as dead space.
    expect(root.className).toContain("flex-1");
    // `min-w-0` matters too: without it a flex item refuses to shrink below its content width, so
    // the input would push the Send button out of the row on a narrow window instead of the input
    // shrinking.
    expect(root.className).toContain("min-w-0");

    // And it really is the row's direct child, which is the premise of `flex-1` growing width
    // rather than height.
    expect(root.parentElement!.className).toContain("items-center");
  });

  it("drops its own frame, so the toolbar's band is the only one", () => {
    renderBar();
    const root = urlBarRoot();

    // The other half of `bare`: no second `px-4 py-2.5` and no second bottom border, which is what
    // made the SOAP tab look like it had a stray divider under its address row.
    expect(root.className).not.toContain("border-b");
    expect(root.className).not.toContain("px-4");
  });

  it("renders `beforeAction` between the address and Send, never after it", () => {
    renderBar({ beforeAction: <input placeholder="ServiceName" readOnly /> });

    const children = Array.from(urlBarRoot().children);
    const address = children.findIndex((c) => c.classList.contains("items-stretch"));
    const service = children.findIndex((c) => c.getAttribute("placeholder") === "ServiceName");
    const send = children.findIndex((c) => c.tagName === "BUTTON");

    expect(address).toBeGreaterThanOrEqual(0);
    expect(service).toBeGreaterThan(address);
    // The whole point of the slot: gRPC's service/method must land on the near side of Send.
    // Anything after the button strands the primary action in the middle of the row.
    expect(send).toBeGreaterThan(service);
  });

  it("`leading` replaces the method dropdown rather than sitting beside it", () => {
    renderBar({ leading: <UrlBarVerb verb="gRPC" /> });

    // A protocol whose verb is not a choice must not render a one-option `<select>` — that reads as
    // a broken dropdown, not as a fixed verb.
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(screen.getByText("gRPC")).toBeInTheDocument();
  });

  it("keeps the method dropdown when `leading` is absent", () => {
    renderBar({ method: "GET", methods: ["GET", "POST"], onMethodChange: noop });

    expect(screen.getByRole("combobox")).toBeInTheDocument();
    expect(screen.getByRole("combobox")).toHaveValue("GET");
  });
});
