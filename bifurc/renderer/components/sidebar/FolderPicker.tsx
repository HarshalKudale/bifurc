import React, { useState } from "react";
import { Folder as FolderType } from "@/types";
import { Folder, ChevronDown } from "@/lib/icons";
import { strings } from "@/lib/strings";

interface Props {
  folders: FolderType[];
  value: string | null;
  onChange(id: string | null): void;
}

export default function FolderPicker({ folders, value, onChange }: Props) {
  const [open, setOpen] = useState(false);

  const selectedName = value ? (folders.find((f) => f.id === value)?.name ?? "—") : strings.folderTree.root;

  function buildOptions(): { folder: FolderType; depth: number }[] {
    const result: { folder: FolderType; depth: number }[] = [];
    function walk(parentId: string | null, depth: number) {
      for (const f of folders) {
        if ((f.parentId ?? null) === parentId) {
          result.push({ folder: f, depth });
          walk(f.id, depth + 1);
        }
      }
    }
    walk(null, 0);
    return result;
  }

  const options = buildOptions();

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 px-2.5 py-1.5 rounded border border-border bg-card hover:bg-surface-2 text-xs text-foreground transition-colors cursor-pointer whitespace-nowrap"
      >
        <Folder size={11} />
        <span className="max-w-[120px] truncate">{selectedName}</span>
        <ChevronDown size={10} className="text-muted-foreground ml-0.5" />
      </button>

      {open && (
        <>
          <div className="fixed inset-0 z-40" onClick={() => setOpen(false)} />
          <div className="absolute bottom-full mb-1 left-0 z-50 bg-card border border-border rounded-md shadow-2xl py-1 min-w-[180px] max-h-60 overflow-y-auto animate-scale-in">
            <button
              onClick={() => { onChange(null); setOpen(false); }}
              className={`w-full text-left px-3 py-1.5 text-xs cursor-pointer hover:bg-surface-2 flex items-center gap-1.5 ${value === null ? "text-signal font-semibold" : "text-foreground"}`}
            >
              <Folder size={11} /> {strings.folderTree.root}
            </button>
            {options.map(({ folder, depth }) => (
              <button
                key={folder.id}
                onClick={() => { onChange(folder.id); setOpen(false); }}
                className={`w-full text-left py-1.5 text-xs cursor-pointer hover:bg-surface-2 flex items-center gap-1.5 ${value === folder.id ? "text-signal font-semibold" : "text-foreground"}`}
                style={{ paddingLeft: 12 + depth * 14 }}
              >
                <Folder size={11} /> {folder.name}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
