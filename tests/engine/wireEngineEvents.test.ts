import { describe, it, expect, vi } from "vitest";
import { EngineEventBus } from "@bifurc/engine/kernel/eventBus";
import { emitLog } from "@bifurc/engine/kernel/telemetry";
import { emit as emitSyncStatus, setState } from "@bifurc/engine/gitops/syncState";
import { wireEngineEvents } from "@bifurc/engine/registerAll";

// Every composition (shell, createEngine, headless host) wires the engine's notifications through this one
// call. Before it existed only the shell did, so a headless host's clients got no captured traffic and no
// sync status — with every transport test green, because those test the bridge, not the traffic.

describe("wireEngineEvents()", () => {
  it("puts sync status and captured traffic on the bus it is given, until undone", () => {
    const bus = new EngineEventBus();
    const syncStatus = vi.fn();
    const logEntry = vi.fn();
    bus.onTyped("sync.status", syncStatus);
    bus.onTyped("log.entry", logEntry);

    const unwire = wireEngineEvents(bus);
    setState("ws-wire", { status: "syncing" });
    emitSyncStatus("ws-wire");
    emitLog({ id: "log-1" } as Parameters<typeof emitLog>[0]);

    expect(syncStatus).toHaveBeenCalledWith(expect.objectContaining({ wsId: "ws-wire", status: "syncing" }));
    expect(logEntry).toHaveBeenCalledWith(expect.objectContaining({ id: "log-1" }));

    unwire();
    emitSyncStatus("ws-wire");
    emitLog({ id: "log-2" } as Parameters<typeof emitLog>[0]);
    expect(syncStatus).toHaveBeenCalledTimes(1);
    expect(logEntry).toHaveBeenCalledTimes(1);
  });
});
