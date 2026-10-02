// @vitest-environment jsdom
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { AppConfig } from "@/types";
import MappingsPanel from "@/panels/MappingsPanel";
import { EntityPathsProvider } from "@/lib/entityPaths";
import { strings } from "@/lib/strings";

// The panel no longer validates a mapping itself: it sends it, and the engine's verdict is what the user
// sees. And it never writes a config back — after a change it adopts the one the engine returns.

const WS = "ws-1";

const config = (mappings: unknown[] = []) =>
  ({
    workspaces: [{ id: WS, name: "Default", createdAt: 0, activeEnvironmentId: null }],
    activeWorkspaceId: WS,
    mappings,
    proxyRules: [],
    environments: [],
  }) as unknown as AppConfig;

let addMapping: ReturnType<typeof vi.fn>;
let getConfig: ReturnType<typeof vi.fn>;
let onConfigChange: ReturnType<typeof vi.fn>;

beforeEach(() => {
  addMapping = vi.fn();
  getConfig = vi.fn();
  onConfigChange = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(window, "api", { value: { addMapping, getConfig }, configurable: true, writable: true });
});

function fillAndCreate(subdomain: string, target: string) {
  render(
    <EntityPathsProvider paths={undefined}>
      <MappingsPanel config={config()} onConfigChange={onConfigChange} onRefreshServices={vi.fn()} />
    </EntityPathsProvider>,
  );
  fireEvent.click(screen.getAllByText(strings.mappings.addMapping)[0]);
  fireEvent.change(screen.getByPlaceholderText(strings.mappings.subdomainPlaceholder), { target: { value: subdomain } });
  fireEvent.change(screen.getByPlaceholderText(strings.mappings.targetPlaceholder), { target: { value: target } });
  fireEvent.click(screen.getByText(strings.common.create));
}

describe("MappingsPanel — create", () => {
  it("shows the engine's refusal on the field it names, and adopts nothing", async () => {
    addMapping.mockRejectedValue(
      Object.assign(new Error("api.localhost is already mapped."), { code: "BAD_REQUEST", details: { field: "domain" } }),
    );
    fillAndCreate("api", "127.0.0.1:4000");

    expect(await screen.findByText("api.localhost is already mapped.")).toBeInTheDocument();
    expect(addMapping).toHaveBeenCalledWith(expect.objectContaining({ domain: "api.localhost", target: "127.0.0.1:4000" }));
    expect(onConfigChange).not.toHaveBeenCalled();
  });

  it("adopts the engine's config after a create instead of editing its own copy", async () => {
    const fresh = config([{ id: "m9", workspaceId: WS, domain: "api.localhost", target: "127.0.0.1:4000", enabled: true }]);
    addMapping.mockResolvedValue({ id: "m9" });
    getConfig.mockResolvedValue(fresh);
    fillAndCreate("api", "127.0.0.1:4000");

    await waitFor(() => expect(onConfigChange).toHaveBeenCalledWith(fresh));
  });
});
