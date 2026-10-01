import type { CaptureClearParams, CaptureGetParams, CaptureListParams, CaptureRemoveParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../kernel/registry";
import { toLogEntryEvent } from "../utils/logEntryWire";
import { clearCapture, getCapture, listCapture, removeCapture } from "./store";

/** `capture.list` / `capture.get` / `capture.clear` / `capture.remove` — see `capture/store.ts`. */
export function registerCaptureCommands(registry: CommandRegistry): void {
  registry.register("capture.list", (p: CaptureListParams) => {
    const entries = listCapture(p.workspaceId);
    return { entries: p.summary ? entries.map(toLogEntryEvent) : entries };
  });
  registry.register("capture.get", (p: CaptureGetParams) => ({ entry: getCapture(p.workspaceId, p.id) ?? null }));
  registry.register("capture.clear", (p: CaptureClearParams) => {
    clearCapture(p.workspaceId);
    return { ok: true };
  });
  registry.register("capture.remove", (p: CaptureRemoveParams) => ({
    ok: true,
    removed: removeCapture(p.workspaceId, p.ids),
  }));
}
