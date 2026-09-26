// @vitest-environment jsdom
import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import "@testing-library/jest-dom";
import PortsSettingsSection from "@/panels/settings/PortsSettingsSection";
import SettingsPanel from "@/panels/SettingsPanel";
import { strings } from "@/lib/strings";
import type { AppConfig } from "@/types";

/**
 * `plan/17` work item 6 — the four host-level ports in one global section.
 *
 * Nothing rendered this section before it existed, so these cases are the whole guard on it. The load-bearing
 * ones are the **count** (`getAllByRole("spinbutton")` is 4 — a fifth input would be the web GUI's port, which
 * is derived and must never become a setting) and the **commit shape** (a port change carries every other
 * field through, because `config.save` writes the whole object back).
 */

/**
 * The section reads only the four ports, so a partial cast is bounded by what it touches. Written out in full
 * it would be ~25 fields of noise that no assertion here depends on.
 */
const cfg = (over: Partial<AppConfig> = {}): AppConfig =>
  ({
    port: 80,
    webhookPort: 9101,
    forwardProxyPort: 8080,
    companionPort: 4747,
    ...over,
  } as AppConfig);

const valueOf = (label: string): string =>
  (screen.getByLabelText(label) as HTMLInputElement).value;

describe("PortsSettingsSection", () => {
  it("renders one input per host-level port, seeded from the config", () => {
    render(<PortsSettingsSection config={cfg()} onConfigChange={vi.fn()} />);

    expect(valueOf(strings.settings.edgePort)).toBe("80");
    expect(valueOf(strings.settings.forwardProxyPort)).toBe("8080");
    expect(valueOf(strings.settings.webhookServerPort)).toBe("9101");
    expect(valueOf(strings.settings.companionPort)).toBe("4747");
  });

  it("offers exactly four ports — the web GUI's derived port is not a setting", () => {
    render(<PortsSettingsSection config={cfg()} onConfigChange={vi.fn()} />);

    const inputs = screen.getAllByRole("spinbutton");
    expect(inputs).toHaveLength(4);
    for (const input of inputs) {
      expect(input.getAttribute("aria-label") ?? "").not.toMatch(/gui/i);
    }
  });

  it("states that the web GUI's port is derived", () => {
    render(<PortsSettingsSection config={cfg()} onConfigChange={vi.fn()} />);

    expect(screen.getByText(strings.settings.portsDerivedNote)).toBeInTheDocument();
  });

  it("commits a new forward proxy port on blur, carrying every other field through", () => {
    const onConfigChange = vi.fn(async () => {});
    render(<PortsSettingsSection config={cfg()} onConfigChange={onConfigChange} />);

    const input = screen.getByLabelText(strings.settings.forwardProxyPort);
    fireEvent.change(input, { target: { value: "9090" } });
    fireEvent.blur(input);

    expect(onConfigChange).toHaveBeenCalledTimes(1);
    expect(onConfigChange).toHaveBeenCalledWith(
      expect.objectContaining({
        port: 80,
        forwardProxyPort: 9090,
        webhookPort: 9101,
        companionPort: 4747,
      })
    );
  });

  it("commits a new companion port on Enter", () => {
    const onConfigChange = vi.fn(async () => {});
    render(<PortsSettingsSection config={cfg()} onConfigChange={onConfigChange} />);

    const input = screen.getByLabelText(strings.settings.companionPort);
    fireEvent.change(input, { target: { value: "5000" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(onConfigChange).toHaveBeenCalledWith(expect.objectContaining({ companionPort: 5000 }));
  });

  it("marks the companion as needing a restart, and only the companion", () => {
    render(<PortsSettingsSection config={cfg()} onConfigChange={vi.fn()} />);

    // Exactly one badge: the edge, the forward proxy and the webhook are re-bound at runtime, so a badge on
    // any of them would be a false claim.
    expect(screen.getAllByText(strings.settings.restartRequired)).toHaveLength(1);
  });

  it("reverts a value that is not a port instead of committing it", () => {
    const onConfigChange = vi.fn(async () => {});
    render(<PortsSettingsSection config={cfg()} onConfigChange={onConfigChange} />);

    const input = screen.getByLabelText(strings.settings.edgePort);
    fireEvent.change(input, { target: { value: "99999" } });
    fireEvent.blur(input);

    expect(onConfigChange).not.toHaveBeenCalled();
    expect(valueOf(strings.settings.edgePort)).toBe("80");
  });

  it("does not commit when the value is unchanged", () => {
    const onConfigChange = vi.fn(async () => {});
    render(<PortsSettingsSection config={cfg()} onConfigChange={onConfigChange} />);

    const input = screen.getByLabelText(strings.settings.forwardProxyPort);
    fireEvent.blur(input);

    expect(onConfigChange).not.toHaveBeenCalled();
  });

  it("shows the configured webhook port rather than a hardcoded default", () => {
    // Discriminating on purpose: the engine deleted a fixed webhook default in favour of a random one, so a
    // `?? 9101` reappearing in this row's copy would be a second answer to "which port is the receiver on".
    render(<PortsSettingsSection config={cfg({ webhookPort: 4321 })} onConfigChange={vi.fn()} />);

    expect(valueOf(strings.settings.webhookServerPort)).toBe("4321");
    expect(screen.getByText(/localhost:4321\/bifurc\/webhooks\//)).toBeInTheDocument();
  });

  it("follows a config change from the outside", () => {
    const { rerender } = render(
      <PortsSettingsSection config={cfg()} onConfigChange={vi.fn()} />
    );

    rerender(
      <PortsSettingsSection config={cfg({ forwardProxyPort: 8888 })} onConfigChange={vi.fn()} />
    );

    expect(valueOf(strings.settings.forwardProxyPort)).toBe("8888");
  });
});

/**
 * The wiring, not the section.
 *
 * The cases above render `PortsSettingsSection` directly, so every one of them would still pass if
 * `SettingsPanel` never mounted it — the component would be correct and unreachable. That is the shape of gap
 * `verify:web` found four of in code that passed 471 tests, so it gets its own case.
 */
describe("SettingsPanel → Ports section", () => {
  it("mounts the Ports section", () => {
    // `__APP_VERSION__` is a Vite `define` global, substituted at build time, so it does not exist when a test
    // renders a component that reads it. Stubbed rather than mocked away: the real panel is the point here.
    vi.stubGlobal("__APP_VERSION__", "0.0.0-test");

    render(
      <SettingsPanel
        config={cfg({ tlsEnabled: false, tlsCaCertPath: null, tlsCaKeyPath: null })}
        serverRunning
        serverError={null}
        onConfigChange={vi.fn(async () => {})}
        onServerRestart={vi.fn(async () => {})}
        colorMode="dark"
        onColorModeChange={vi.fn()}
        sidebarVisibility={{}}
        onSidebarVisibilityChange={vi.fn()}
      />
    );

    expect(screen.getByText(strings.settings.sectionPorts)).toBeInTheDocument();
  });
});
