/**
 * The deleted sync/git/audit/history channels, reproduced for the two suites that drove them by name.
 *
 * `src/ipc/handlers/syncHandlers.ts` held **19 channels** — 17 of them one-line
 * `commandRegistry.invoke()` delegates — and was deleted on 2026-09-21 as the P6 reduction's second
 * handler group (`plan/07` §5b-terdecies). `gitRemote.integration.test.ts` and
 * `gitSync.integration.test.ts` drove those channels *by name* through the mocked `ipcMain`, so the call
 * shape has to resolve somewhere: this module is where.
 *
 * ## Why a table, and why `:` → `.` is not a guess
 *
 * There are **62 call sites** across the two suites and the mapping is **not** uniform —
 * `git:history` spreads a trailing `opts`, `history:list` passes its whole argument through — so inlining
 * it would be 62 chances to build the wrong object in suites whose subject is real git behaviour rather
 * than routing. The mapping lives here once.
 *
 * `command = channel.replace(":", ".")` is **exact for all sixteen channels below**, checked against the
 * deleted bodies rather than assumed, so only the part that varies is written down.
 *
 * ## This is not a second contract
 *
 * The production mapping is the **client's** (`packages/client/src/index.ts`), and it is asserted there —
 * `packages/client/tests/client.test.ts`'s "maps a positional window.api call onto the command's object
 * payload". This file exists so that deleting the shell's channels does not require rewriting 64
 * assertions about git.
 */
import type { CommandAction } from "@bifurc/protocol";
import { commandRegistry } from "@bifurc/engine/kernel/registry";
import { bus } from "@bifurc/engine/kernel/eventBus";

type Builder = (args: any[]) => unknown;

/** Positional arguments → the object payload, exactly as each deleted channel body built it. */
const PARAMS: Record<string, Builder> = {
  // ── sync ────────────────────────────────────────────────────────────────────────────────────────
  "sync:setRemote": ([workspaceId, remote, branch]) => ({ workspaceId, remote, branch }),
  "sync:disconnect": ([workspaceId]) => ({ workspaceId }),
  "sync:push": ([workspaceId]) => ({ workspaceId }),
  "sync:pull": ([workspaceId]) => ({ workspaceId }),
  "sync:getState": ([workspaceId]) => ({ workspaceId }),
  "sync:setAutoSync": ([workspaceId, enabled]) => ({ workspaceId, enabled }),
  "sync:getEntityStatus": ([workspaceId]) => ({ workspaceId }),
  // ── generic git ─────────────────────────────────────────────────────────────────────────────────
  "git:diff": ([workspaceId, relPath]) => ({ workspaceId, relPath }),
  "git:discard": ([workspaceId, relPath]) => ({ workspaceId, relPath }),
  "git:sync": ([workspaceId, paths, message]) => ({ workspaceId, paths, message }),
  // The one spread: the channel did `{ workspaceId, relPath, ...opts }`.
  "git:history": ([workspaceId, relPath, opts]) => ({ workspaceId, relPath, ...(opts ?? {}) }),
  // ── entity / folder ─────────────────────────────────────────────────────────────────────────────
  "entity:publish": ([workspaceId, paths, message]) => ({ workspaceId, paths, message }),
  "folder:publish": ([workspaceId, kind, folderName]) => ({ workspaceId, kind, folderName }),
  "entity:restore": ([workspaceId, relPath]) => ({ workspaceId, relPath }),
  // ── audit / history ─────────────────────────────────────────────────────────────────────────────
  // The other passthrough: `history:list` forwarded its single argument object whole.
  "history:list": ([opts]) => opts ?? {},
  "history:diff": ([commitHash, filePath, workspaceId]) => ({ commitHash, filePath, workspaceId }),
};

/**
 * `T` defaults to `any` deliberately: the helper this replaces did too, and the alternative is touching
 * all 64 call sites to add type arguments in tests whose assertions are about git, not about typing.
 */
export async function invokeDeletedChannel<T = any>(channel: string, ...args: any[]): Promise<T> {
  const build = PARAMS[channel];
  if (!build) {
    throw new Error(
      `deletedChannelHarness: no mapping for "${channel}". ` +
        `If this channel still exists, it belongs to a group the reduction has not reached yet.`,
    );
  }
  const command = channel.replace(":", ".") as CommandAction;
  return (await commandRegistry.invoke(command, build(args), { bus })) as T;
}
