import type {
  CaptureClearParams, CaptureGetParams, CaptureListParams, CaptureRemoveParams, CaptureSetPausedParams,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../kernel/registry";
import { toLogEntryEvent } from "../utils/logEntryWire";
import { clearCapture, getCapture, isCapturePaused, listCapture, removeCapture, setCapturePaused } from "./store";

/** `capture.list` / `capture.get` / `capture.setPaused` / `capture.clear` / `capture.remove` — see `capture/store.ts`. */
export function registerCaptureCommands(registry: CommandRegistry): void {
  registry.register("capture.list", (p: CaptureListParams) => {
    const entries = listCapture(p.workspaceId);
    return { entries: p.summary ? entries.map(toLogEntryEvent) : entries, paused: isCapturePaused(p.workspaceId) };
  });
  registry.register("capture.setPaused", (p: CaptureSetPausedParams) => {
    setCapturePaused(p.workspaceId, p.paused);
    return { paused: isCapturePaused(p.workspaceId) };
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
