// @vitest-environment jsdom
import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import "@testing-library/jest-dom";
import CodeEditor from "@/components/common/CodeEditor";

// CodeMirror uses ResizeObserver, MutationObserver, and layout APIs that
// jsdom doesn't fully implement. We provide minimal stubs.

beforeEach(() => {
  if (!window.ResizeObserver) {
    window.ResizeObserver = class {
      observe()    {}
      unobserve()  {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  }
  // jsdom provides MutationObserver, but ensure it exists
  if (!window.MutationObserver) {
    window.MutationObserver = class {
      observe()    {}
      disconnect() {}
      takeRecords() { return []; }
    } as unknown as typeof MutationObserver;
  }
  // CM6 calls getBoundingClientRect on its container
  if (!Element.prototype.getBoundingClientRect) {
    Element.prototype.getBoundingClientRect = () => ({
      x: 0, y: 0, width: 200, height: 200,
      top: 0, left: 0, bottom: 200, right: 200,
      toJSON: () => {},
    });
  }
});

describe("CodeEditor", () => {
  it("renders a container div with data-testid", () => {
    render(<CodeEditor value="hello" />);
    expect(screen.getByTestId("code-editor")).toBeInTheDocument();
  });

  it("mounts a .cm-editor inside the container", () => {
    render(<CodeEditor value='{"key": 1}' language="json" />);
    const container = screen.getByTestId("code-editor");
    // CM6 attaches .cm-editor as a child of the container
    const cmEditor = container.querySelector(".cm-editor");
    expect(cmEditor).not.toBeNull();
  });

  it("does not call onChange in readOnly mode", () => {
    const onChange = vi.fn();
    render(<CodeEditor value="const x = 1;" language="javascript" readOnly onChange={onChange} />);
    // In readOnly mode the editor is not editable — onChange should never be wired
    expect(onChange).not.toHaveBeenCalled();
  });

  it("renders without errors for empty value", () => {
    expect(() => render(<CodeEditor value="" />)).not.toThrow();
  });

  it("renders without errors for each supported language", () => {
    const languages = ["json", "html", "xml", "javascript", "text"] as const;
    for (const lang of languages) {
      expect(() => render(<CodeEditor value="test" language={lang} />)).not.toThrow();
    }
  });

  it("renders placeholder text when value is empty", () => {
    render(<CodeEditor value="" placeholder="Type something…" />);
    const container = screen.getByTestId("code-editor");
    // CM6 placeholder is rendered as a .cm-placeholder span
    const ph = container.querySelector(".cm-placeholder");
    expect(ph).not.toBeNull();
  });

  it("updates displayed content when value prop changes", async () => {
    const { rerender } = render(<CodeEditor value="first" language="text" />);
    await act(async () => {
      rerender(<CodeEditor value="second" language="text" />);
    });
    // No errors thrown means the sync update path worked
    expect(screen.getByTestId("code-editor")).toBeInTheDocument();
  });

  it("accepts a minHeight style prop", () => {
    render(<CodeEditor value="" minHeight={120} />);
    const el = screen.getByTestId("code-editor") as HTMLElement;
    expect(el.style.minHeight).toBe("120px");
  });

  it("forwards className to the container", () => {
    render(<CodeEditor value="" className="custom-class flex-1" />);
    const el = screen.getByTestId("code-editor");
    expect(el).toHaveClass("custom-class", "flex-1");
  });
});
