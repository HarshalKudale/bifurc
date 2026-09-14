import React, { useEffect, useRef } from "react";
import { Folder as FolderType } from "@/types";
import { strings } from "@/lib/strings";

export function MoveDialog({
  folders,
  onMove,
  onCancel,
}: {
  folders: FolderType[];
  onMove(folderId: string | null): void;
  onCancel(): void;
}) {
  const firstRef = useRef<HTMLButtonElement>(null);
  
  useEffect(() => {
    firstRef.current?.focus();
  }, []);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.45)" }}
      onClick={onCancel}
    >
      <div
        className="bg-card border border-border rounded-lg shadow-2xl w-56"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
      >
        <div className="px-3 py-2 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground border-b border-border">
          {strings.folderTree.moveToFolder}
        </div>
        <div className="max-h-64 overflow-y-auto py-1">
          <button
            ref={firstRef}
            className="w-full text-left px-3 py-1.5 text-xs hover:bg-surface-2 text-muted-foreground cursor-pointer"
            onClick={() => onMove(null)}
          >
            {strings.folderTree.root}
          </button>
          {folders.map((f) => (
            <button
              key={f.id}
              className="w-full text-left px-3 py-1.5 text-xs hover:bg-surface-2 text-foreground cursor-pointer"
              onClick={() => onMove(f.id)}
            >
              {f.name}
            </button>
          ))}
        </div>
        <div className="p-2 border-t border-border">
          <button
            className="w-full text-center text-xs text-muted-foreground hover:text-foreground cursor-pointer py-0.5"
            onClick={onCancel}
          >
            {strings.common.cancel}
          </button>
        </div>
      </div>
    </div>
  );
}
