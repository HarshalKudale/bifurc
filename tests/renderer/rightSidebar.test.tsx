// @vitest-environment jsdom
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom";
import { AppConfig, Environment, Workspace } from "@/types";
import { Panel } from "@/lib/panelRegistry";
import RightSidebar from "@/components/sidebar/RightSidebar";
import VariablesSidebar from "@/components/sidebar/VariablesSidebar";
import TitleBar from "@/components/layout/TitleBar";

/**
 * The right sidebar grew a second face (2026-09-28): entity history, and the variables the active
 * environment can resolve. Three things are worth pinning, and none of them is a class name.
 *
 * 1. **The mode is the open state.** `null` must render *no* face and zero width; `"variables"` must
 *    render only the variables face. That is what makes "press the open face's control to close" and
 *    "press the other one to swap" work without a second flag that could disagree with the first.
 * 2. **The variables face groups by resolver order, not as one merged list.** `mergeEnvVars` overlays
 *    the environment on `__global__`, so a key in both resolves to the environment's value — and the
 *    only way to see that is to keep the global row and mark it.
 * 3. **The toggle sits beside the environment selector**, because what it opens is the expansion of
 *    that selector's current value.
 *
 * The history face is mocked. It reaches the bridge (`window.api.listHistory`) and its own loading
 * behaviour is not what this file is about.
 */
vi.mock("@/components/sidebar/HistorySidebar", () => ({
  default: () => <div data-testid="history-face" />,
}));

function makeEnv(id: string, name: string, vars: Array<[string, string]>): Environment {
  return {
    id,
    name,
    workspaceId: "ws-1",
    createdAt: 0,
    variables: vars.map(([key, value], i) => ({ id: `${id}-${i}`, key, value })),
  };
}

/**
 * Document order, which is the strongest ordering claim jsdom can support — it has no layout engine,
 * so "above" can only mean "earlier in the DOM".
 */
function precedes(a: Element, b: Element): boolean {
  return (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
}

describe("RightSidebar — one shell, two faces", () => {
  const shellProps = {
    filePath: "requests/r1.json",
    workspaceId: "ws-1",
    onClose: vi.fn(),
    activeEnvironment: null,
    globalEnvironment: null,
  };

  it("renders no face and collapses to zero width when the mode is null", () => {
    const { container } = render(<RightSidebar {...shellProps} mode={null} />);
    const shell = container.querySelector(".right-sidebar") as HTMLElement;
    expect(shell.style.width).toBe("0px");
    expect(shell.style.opacity).toBe("0");
    expect(screen.queryByTestId("history-face")).not.toBeInTheDocument();
    expect(screen.queryByText("Variables")).not.toBeInTheDocument();
  });

  it("shows the history face, and only that, in history mode", () => {
    render(<RightSidebar {...shellProps} mode="history" />);
    expect(screen.getByTestId("history-face")).toBeInTheDocument();
    expect(screen.queryByText("Variables")).not.toBeInTheDocument();
  });

  it("shows the variables face, and only that, in variables mode", () => {
    const { container } = render(<RightSidebar {...shellProps} mode="variables" />);
    const shell = container.querySelector(".right-sidebar") as HTMLElement;
    expect(shell.style.width).toBe("280px");
    expect(shell.style.opacity).toBe("1");
    expect(screen.queryByTestId("history-face")).not.toBeInTheDocument();
    expect(screen.getByText("Variables")).toBeInTheDocument();
  });
});

describe("VariablesSidebar — the variables face", () => {
  const active = makeEnv("env-dev", "Dev", [["API_KEY", "dev-key"], ["SHARED", "from-dev"]]);
  const globalEnv = makeEnv("__global__", "Global", [["SHARED", "from-global"], ["TIMEOUT", "30"]]);

  let writeText: ReturnType<typeof vi.fn>;
  let onClose: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    onClose = vi.fn();
    writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
  });

  it("lists the active environment's variables above the globals", () => {
    render(<VariablesSidebar activeEnvironment={active} globalEnvironment={globalEnv} onClose={onClose} />);

    expect(screen.getByText("{{API_KEY}}")).toBeInTheDocument();
    expect(screen.getByText("{{TIMEOUT}}")).toBeInTheDocument();
    expect(screen.getByText("4 variables")).toBeInTheDocument();
    expect(precedes(screen.getByText("Environment"), screen.getByText("Global"))).toBe(true);
  });

  it("keeps a global row the environment shadows, and marks it overridden", () => {
    render(<VariablesSidebar activeEnvironment={active} globalEnvironment={globalEnv} onClose={onClose} />);

    // Both SHARED rows survive — that is the whole point. The merged view would show one.
    expect(screen.getAllByText("{{SHARED}}")).toHaveLength(2);
    expect(screen.getByText("from-dev")).toBeInTheDocument();
    expect(screen.getByText("from-global")).toBeInTheDocument();

    const badges = screen.getAllByText("overridden");
    expect(badges).toHaveLength(1);
    // …and the badge is on the *global* row, because the environment is what wins.
    expect(precedes(screen.getByText("Global"), badges[0])).toBe(true);
  });

  it("marks nothing overridden when the environment shadows no global", () => {
    const prod = makeEnv("env-prod", "Prod", [["API_KEY", "prod-key"]]);
    render(<VariablesSidebar activeEnvironment={prod} globalEnvironment={globalEnv} onClose={onClose} />);

    expect(screen.queryByText("overridden")).not.toBeInTheDocument();
    expect(screen.getByText("3 variables")).toBeInTheDocument();
  });

  it("asks for an environment when none is active, and still shows the globals", () => {
    render(<VariablesSidebar activeEnvironment={null} globalEnvironment={globalEnv} onClose={onClose} />);

    expect(screen.getByText("No environment selected")).toBeInTheDocument();
    expect(screen.getByText("{{TIMEOUT}}")).toBeInTheDocument();
    expect(screen.getByText("2 variables")).toBeInTheDocument();
  });

  it("copies the {{KEY}} token rather than the value", async () => {
    render(<VariablesSidebar activeEnvironment={active} globalEnvironment={globalEnv} onClose={onClose} />);

    const row = screen.getByText("{{API_KEY}}").closest(".group") as HTMLElement;
    fireEvent.click(within(row).getByLabelText("Copy variable reference"));

    await waitFor(() => expect(writeText).toHaveBeenCalledWith("{{API_KEY}}"));
  });

  it("closes from the header button", () => {
    render(<VariablesSidebar activeEnvironment={active} globalEnvironment={globalEnv} onClose={onClose} />);

    fireEvent.click(screen.getByLabelText("Close variables"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("TitleBar — the variables toggle", () => {
  const baseProps = {
    config: {
      activeWorkspaceId: "ws-1",
      activeEnvironmentId: null,
      port: 8080,
      environments: [],
    } as unknown as AppConfig,
    serverRunning: false,
    serverError: null,
    helpText: "help",
    onServerStart: vi.fn(),
    onServerStop: vi.fn(),
    onEnvChange: vi.fn(),
    onManageEnvs: vi.fn(),
    workspaces: [{ id: "ws-1", name: "Default" }] as Workspace[],
    activeWorkspaceId: "ws-1",
    onWorkspaceChange: vi.fn(),
    onWorkspaceCreate: vi.fn(),
    onWorkspaceRename: vi.fn(),
    onWorkspaceDelete: vi.fn(),
    activePanel: "services" as Panel,
    onOpenWorkspaceSettings: vi.fn(),
    onOpenSearch: vi.fn(),
  };

  it("sits beside the environment selector and reports its pressed state", () => {
    const onToggleVariables = vi.fn();
    render(<TitleBar {...baseProps} variablesOpen={false} onToggleVariables={onToggleVariables} />);

    const toggle = screen.getByRole("button", { name: "Environment variables" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(precedes(screen.getByTitle("Switch environment"), toggle)).toBe(true);

    fireEvent.click(toggle);
    expect(onToggleVariables).toHaveBeenCalledTimes(1);
  });

  it("shows as pressed while the variables face is open", () => {
    render(<TitleBar {...baseProps} variablesOpen={true} onToggleVariables={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Environment variables" })).toHaveAttribute("aria-pressed", "true");
  });
});
