import React from "react";
import { strings } from "@/lib/strings";

/**
 * The dialog for a **deleted-but-uncommitted** entity — the same one everywhere.
 *
 * ## Why it is generic
 *
 * It used to take a `FolderTreeItem`, which was fine while the folder tree was the only surface that
 * could show this state. It is not any more: the mappings panel and the environments panel render
 * entities straight from the config, and since `mappings`/`environments` gained a pending-deletion
 * record (2026-09-28) they can show the same state. A dialog that needs a tree item cannot be used
 * there, and the alternative — a second dialog with the same two buttons — is how the four
 * vocabularies for publish/revert happened in the first place.
 *
 * ## The two choices are the two git outcomes, and there are only two
 *
 * - **Recover** is `revert` — `git checkout HEAD -- <path>` restores the file and clears the pending
 *   deletion. It is not a separate operation, which is why this dialog does not call a third handler.
 * - **Commit Delete** is `publish` — the deletion is staged, committed and pushed like any other
 *   change. It is not a separate operation either.
 *
 * So a caller passes the same two callbacks it uses for Publish and Revert, and the *labels* here are
 * the only thing that is specific to the deleted state.
 */
interface Props {
  /** The entity's display name. */
  name: string;
  /** Restore the last synced version. */
  onRecover: () => void;
  /** Commit the deletion. */
  onCommitDelete: () => void;
  onClose: () => void;
}

export function DeletedItemDialog({ name, onRecover, onCommitDelete, onClose }: Props) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
    >
      <div
        className="bg-card border border-border rounded-lg shadow-2xl p-5 flex flex-col gap-4"
        style={{ minWidth: 300, maxWidth: 380 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ fontSize: 13, fontWeight: 600, color: "var(--c-amber)" }}>
          {strings.entityGit.pendingDeletion}
        </div>
        <div style={{ fontSize: 12, color: "var(--c-muted-foreground)", lineHeight: 1.5 }}>
          <span style={{ color: "var(--c-foreground)", fontWeight: 500 }}>{name}</span>
          {" "}{strings.entityGit.pendingDeletionBody}
        </div>
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          <button
            className="px-3 py-1.5 text-xs rounded border border-border hover:bg-surface-2 text-foreground cursor-pointer"
            onClick={onClose}
          >
            {strings.common.cancel}
          </button>
          <button
            className="px-3 py-1.5 text-xs rounded border border-signal text-signal hover:bg-signal/10 cursor-pointer"
            title={strings.entityGit.recoverTooltip}
            onClick={onRecover}
          >
            {strings.entityGit.recover}
          </button>
          <button
            className="px-3 py-1.5 text-xs rounded bg-destructive/80 hover:bg-destructive text-white cursor-pointer"
            title={strings.entityGit.syncDeleteTooltip}
            onClick={onCommitDelete}
          >
            {strings.entityGit.syncDelete}
          </button>
        </div>
      </div>
    </div>
  );
}
