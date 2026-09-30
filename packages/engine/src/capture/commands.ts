import type { CaptureClearParams, CaptureListParams, CaptureRemoveParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../kernel/registry";
import { clearCapture, listCapture, removeCapture } from "./store";

/** `capture.list` / `capture.clear` / `capture.remove` — see `capture/store.ts`. */
export function registerCaptureCommands(registry: CommandRegistry): void {
  registry.register("capture.list", (p: CaptureListParams) => ({ entries: listCapture(p.workspaceId) }));
  registry.register("capture.clear", (p: CaptureClearParams) => {
    clearCapture(p.workspaceId);
    return { ok: true };
  });
  registry.register("capture.remove", (p: CaptureRemoveParams) => ({
    ok: true,
    removed: removeCapture(p.workspaceId, p.ids),
  }));
}
