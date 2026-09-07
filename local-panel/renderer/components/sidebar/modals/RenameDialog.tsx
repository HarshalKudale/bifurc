import React, { useState, useEffect, useRef } from "react";
import { strings } from "@/lib/strings";

export function RenameDialog({
  currentName,
  onSave,
  onCancel,
}: {
  currentName: string;
  onSave(name: string): void;
  onCancel(): void;
}) {
  const [val, setVal] = useState(currentName);
  const ref = useRef<HTMLInputElement>(null);
  
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);

  const handleSave = () => {
    const trimmed = val.trim();
    if (trimmed && trimmed !== currentName) onSave(trimmed);
    else onCancel();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.5)" }}
      onClick={onCancel}
    >
      <div
        className="bg-card border border-border rounded-lg shadow-2xl p-4 w-72"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            handleSave();
          }
          if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
      >
        <div className="text-xs font-semibold text-foreground mb-3">{strings.folderTree.renameFolder}</div>
        <input
          ref={ref}
          className="w-full bg-surface-2 border border-border focus:border-signal rounded px-2 py-1.5 text-xs text-foreground outline-none mb-4"
          value={val}
          onChange={(e) => setVal(e.target.value)}
        />
        <div className="flex justify-end gap-2">
          <button
            className="px-3 py-1.5 text-xs rounded border border-border hover:bg-surface-2 text-muted-foreground cursor-pointer"
            onClick={onCancel}
          >
            {strings.common.cancel}
          </button>
          <button
            className="px-3 py-1.5 text-xs rounded font-semibold cursor-pointer"
            style={{ background: "var(--c-signal)", color: "#fff", opacity: val.trim() ? 1 : 0.5 }}
            disabled={!val.trim()}
            onClick={handleSave}
          >
            {strings.common.save}
          </button>
        </div>
      </div>
    </div>
  );
}
