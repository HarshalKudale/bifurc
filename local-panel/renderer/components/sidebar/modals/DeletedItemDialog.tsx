import React from "react";
import { FolderTreeItem } from "../FolderTree.types";
import { strings } from "@/lib/strings";

interface Props {
  item: FolderTreeItem;
  onRestore?: (id: string) => void;
  onPublish?: (id: string) => void;
  onClose: () => void;
}

export function DeletedItemDialog({ item, onRestore, onPublish, onClose }: Props) {
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
          {strings.folderTree.pendingDeletion}
        </div>
        <div style={{ fontSize: 12, color: "var(--c-muted-foreground)", lineHeight: 1.5 }}>
          <span style={{ color: "var(--c-foreground)", fontWeight: 500 }}>{item.name}</span>
          {" "}{strings.folderTree.pendingDeletionBody}
        </div>
        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          <button
            className="px-3 py-1.5 text-xs rounded border border-border hover:bg-surface-2 text-foreground cursor-pointer"
            onClick={onClose}
          >
            {strings.common.cancel}
          </button>
          {onRestore && (
            <button
              className="px-3 py-1.5 text-xs rounded border border-signal text-signal hover:bg-signal/10 cursor-pointer"
              onClick={() => { onRestore(item.id); onClose(); }}
            >
              {strings.folderTree.restore}
            </button>
          )}
          {onPublish && (
            <button
              className="px-3 py-1.5 text-xs rounded bg-destructive/80 hover:bg-destructive text-white cursor-pointer"
              onClick={() => { onPublish(item.id); onClose(); }}
            >
              {strings.folderTree.commitDelete}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
