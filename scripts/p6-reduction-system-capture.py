#!/usr/bin/env python3
"""P6 reduction, group 5 — part 2: the capture suite's re-point and the client's ordering pin.

`capture:shareJson` was the one channel in the group that owned a `dialog.showSaveDialog`. Its
production successor is the client's `shareCaptureJson` -> `local.writeArtifact`, so the two things
this script does are (a) re-point the integration suite that drove the channel and (b) pin the
client's ordering — artifact-first, deliberately — so the deletion is safe against a future
"harmonisation".

Count-asserting, CRLF-aware, idempotent (guard tests `new` only).
"""
import io
import sys

EDITS = [
    # ── capture suite: the @/main note + the invoke table ─────────────────────
    ("tests/integration/capture.integration.test.ts", 1,
     """// `systemHandlers` imports `getMainWindow` from `@/main`, which must not be loaded for real.
vi.mock("@/main", () => ({ getMainWindow: () => null, updateTrayMenu: () => { } }));

import { registerSystemHandlers } from "@/ipc/handlers/systemHandlers";

async function invoke<T = any>(channel: string, ...args: any[]): Promise<T> {
    const fn = handlers.get(channel);
    if (!fn) throw new Error(`No handler registered for "${channel}"`);
    return await fn({}, ...args);
}""",
     """/**
 * `@/main` (the tray) must not be loaded for real. Nothing this suite imports pulls it in any more —
 * `registerSystemHandlers` used to, and this suite no longer registers it — but the mock stays as the
 * guard for a transitive import that would otherwise reach Electron's window machinery.
 */
vi.mock("@/main", () => ({ getMainWindow: () => null, updateTrayMenu: () => { } }));

import { commandRegistry } from "@bifurc/engine/commands/registry";
import { bus } from "@bifurc/engine/eventBus";
import type { CaptureShareJsonResult } from "@bifurc/protocol";
import { call, writeArtifact } from "@/ipc/fileOpsClient";

/**
 * `capture:shareJson`, reproduced — the P6 reduction's **fifth** group deleted it on 2026-09-21.
 *
 * Reproduced rather than dropped, because the two cases below are about things the channel did *not*
 * own: the engine's `capture.shareJson` serialising a real captured entry, and the cancel shape. What
 * the channel did own — the save dialog and the write — is the one thing a shell channel may
 * legitimately own, and its **production successor is the client's** `shareCaptureJson`, which reaches
 * the same two steps through `local.writeArtifact`.
 *
 * The ordering differs and that is deliberate, not an oversight this fixture papers over: the shell
 * body asked first, the client renders first (`packages/client/src/local.ts`'s egress note argues the
 * artifact is a `JSON.stringify` the renderer already holds, so the command is cheap and asking first
 * would discard the engine's own suggested filename). `packages/client/tests/client.test.ts` pins the
 * client's order. This table keeps the *shell's* order because it is reproducing the deleted body, and
 * neither case below depends on the order — one sets a real path, the other cancels.
 */
const DELETED_CAPTURE_CHANNELS: Record<string, (...args: any[]) => unknown> = {
    "capture:shareJson": async (_e: any, entries: unknown[], suggestedName?: string) => {
        // Dialog FIRST — `File_Ops_Protocol.md` §3.2, as the deleted body had it. The mocked
        // `dialog.showSaveDialog` returns `dialogState.result`, so reading it directly is the same
        // value without an import of the mock.
        const { filePath, canceled } = dialogState.result;
        if (canceled || !filePath) return { ok: false, canceled: true };

        const artifact = await call<CaptureShareJsonResult>("capture.shareJson", { entries, suggestedName });
        if (!artifact.ok) return { ok: false, error: artifact.error };

        writeArtifact(artifact, filePath);
        return { ok: true, filePath };
    },
};

async function invoke<T = any>(channel: string, ...args: any[]): Promise<T> {
    const deleted = DELETED_CAPTURE_CHANNELS[channel];
    if (deleted) return (await deleted(...args)) as T;
    const fn = handlers.get(channel);
    if (!fn) throw new Error(`No handler registered for "${channel}"`);
    return await fn({}, ...args);
}"""),

    # ── capture suite: beforeAll ─────────────────────────────────────────────
    ("tests/integration/capture.integration.test.ts", 1,
     """    // P3 made `capture:shareJson` a thin client: it shows the dialog, calls the engine's
    // `capture.shareJson` and writes the artifact that comes back. The renderer-facing contract is
    // unchanged, so every assertion below still holds — but the engine half has to be registered,
    // exactly as `src/ipc/handlers.ts` registers it in production.
    //
    // `setDataRoot()` is supplied as well as the fixture's own override, because `blobRoot()`
    // resolves through `dataDir()` while the workspace store resolves through `setDataRootOverride`.
    // `createWorkspace()` only sets the latter, so without this a capture large enough to cross
    // `BLOB_INLINE_THRESHOLD_BYTES` would fail with `DataRootNotInitialisedError` instead of
    // staging a blob.
    setDataRoot(ws.root);
    registerBlobCommands(commandRegistry);
    registerFileOpsCommands(commandRegistry);
    registerSystemHandlers();
});""",
     """    // `setDataRoot()` is supplied as well as the fixture's own override, because `blobRoot()`
    // resolves through `dataDir()` while the workspace store resolves through `setDataRootOverride`.
    // `createWorkspace()` only sets the latter, so without this a capture large enough to cross
    // `BLOB_INLINE_THRESHOLD_BYTES` would fail with `DataRootNotInitialisedError` instead of
    // staging a blob.
    setDataRoot(ws.root);
    registerBlobCommands(commandRegistry);
    registerFileOpsCommands(commandRegistry);
    // `registerSystemHandlers()` used to be called here, for `capture:shareJson`. The channel is gone
    // (P6 reduction, fifth group, 2026-09-21) and the one channel that function still registers —
    // `app:checkUpdate` — is not driven by this suite, so the call went with it. The two shareJson
    // cases below drive the command through `DELETED_CAPTURE_CHANNELS` instead, which is also why the
    // `handlers` map is now empty: `invoke`'s fallthrough is a safety net that makes a future
    // un-translated channel fail loudly rather than silently resolve `undefined`.
});"""),

    # ── capture suite: the describe title ───────────────────────────────────
    ("tests/integration/capture.integration.test.ts", 1,
     """describe("capture:shareJson — exporting selected captures", () => {""",
     """describe("capture.shareJson — exporting selected captures (the deleted `capture:shareJson` channel)", () => {"""),

    # ── the client's ordering pin ───────────────────────────────────────────
    ("packages/client/tests/client.test.ts", 1,
     """  it("exportAudit reports a cancel and never runs the command", async () => {
    const { local } = fsLocal({ savePath: null });
    const { t, calls } = respondingTransport(() => ({ ok: true, inline: "eA==" }));
    const client = createClient(t, { local });

    await expect(client.exportAudit("json")).resolves.toEqual({ ok: false });
    // Fail fast: nothing was rendered, so the whole-log query never ran.
    expect(calls.some((c) => c.cmd === "audit.export")).toBe(false);
  });""",
     """  it("exportAudit reports a cancel and never runs the command", async () => {
    const { local } = fsLocal({ savePath: null });
    const { t, calls } = respondingTransport(() => ({ ok: true, inline: "eA==" }));
    const client = createClient(t, { local });

    await expect(client.exportAudit("json")).resolves.toEqual({ ok: false });
    // Fail fast: nothing was rendered, so the whole-log query never ran.
    expect(calls.some((c) => c.cmd === "audit.export")).toBe(false);
  });

  it("shareCaptureJson renders BEFORE it asks — the deliberate exception to dialog-first", async () => {
    /**
     * The **counterpart** to the two `exportAudit` cases above, and the confirmation that made
     * deleting the shell's `capture:shareJson` channel safe (P6 reduction, fifth group, 2026-09-21).
     *
     * The shell channel was dialog-first — save dialog, then `capture.shareJson`, then write. The
     * client is deliberately the other way round, and `local.ts`'s egress note gives the reason: this
     * artifact is a `JSON.stringify` of entries the renderer already holds, so rendering it before the
     * user commits costs nothing, whereas asking first would throw away the engine's own
     * `suggestedName` as the dialog's default. `audit.export` is the opposite case — `queryLog({
     * limit: 0 })`, the whole `git log` — which is why that one *had* to become dialog-first and this
     * one must not.
     *
     * Two facts are pinned, and neither is about values: that no `pickSavePath` happens (the hook is
     * called with no `destPath`, so it shows the dialog itself, after the render), and that the hook is
     * handed the **artifact's** name rather than the caller's fallback. Without this the difference
     * lives only in a docblock — and `exportAudit` is the worked example of what happens when a
     * docblock and a call disagree: the docblock was edited to sanction the bug.
     */
    const order: string[] = [];
    const { local } = fsLocal();
    local.writeArtifact = async (_b: string, suggestedName: string, _m: string, destPath?: string) => {
      order.push(`writeArtifact:${destPath ?? suggestedName}`);
      return { ok: true, filePath: destPath ?? "/tmp/written" };
    };
    const { t } = respondingTransport((cmd) => {
      order.push(cmd);
      return cmd === "capture.shareJson"
        ? { ok: true, inline: "eA==", suggestedName: "captured-requests.json", mimeType: "application/json" }
        : { ok: true };
    });
    const client = createClient(t, { local });

    const res = await client.shareCaptureJson([{ id: "x" }], "capture-GET-1.json");

    // The command first, then the hook — and no `pickSavePath` anywhere in between.
    expect(order).toEqual(["capture.shareJson", "writeArtifact:captured-requests.json"]);
    expect(res).toEqual({ ok: true, filePath: "/tmp/written" });
  });"""),
]


def main() -> int:
    failures, skipped = [], 0
    for path, n, old, new in EDITS:
        with io.open(path, "r", encoding="utf-8", newline="") as fh:
            text = fh.read()
        crlf = "\r\n" in text
        old_f = old.replace("\n", "\r\n") if crlf else old
        new_f = new.replace("\n", "\r\n") if crlf else new
        found_old, found_new = text.count(old_f), text.count(new_f)

        if found_new == n:
            skipped += 1
            print("skip %-56s x%d" % (path, n))
            continue
        if found_old != n:
            failures.append("%s: expected %d, found %d\n    ---\n%s\n    ---"
                            % (path, n, found_old, old[:300]))
            continue
        with io.open(path, "w", encoding="utf-8", newline="") as fh:
            fh.write(text.replace(old_f, new_f))
        print("ok   %-56s x%d" % (path, n))

    if failures:
        print("\nABORTED — pairs below did not match:")
        for f in failures:
            print(f)
        return 1
    print("\nskipped: %d" % skipped)
    return 0


if __name__ == "__main__":
    sys.exit(main())
