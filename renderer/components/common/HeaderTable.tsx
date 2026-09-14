import React, { useRef, useEffect } from "react";
import { KVRow, mkRowId } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { X } from "@/lib/icons";

interface Props {
  rows: KVRow[];
  onChange: (rows: KVRow[]) => void;
  readOnly?: boolean;
  emptyMessage?: string;
  mockControls?: {
    allMocked: boolean;
    anyMocked: boolean;
    onToggleAll: (mocked: boolean) => void;
  };
}

export default function HeaderTable({ rows, onChange, readOnly = false, emptyMessage, mockControls }: Props) {
  const topKeyInputRef = useRef<HTMLInputElement>(null);

  // Ensure there is always an empty row at index 0 when not read-only
  useEffect(() => {
    if (readOnly) return;
    if (rows.length === 0) {
      onChange([{ id: mkRowId(), enabled: true, key: "", value: "" }]);
    } else if (rows[0].key.trim() !== "" || rows[0].value.trim() !== "") {
      onChange([{ id: mkRowId(), enabled: true, key: "", value: "" }, ...rows]);
    }
  }, [readOnly, rows.length === 0 ? 0 : rows[0]?.id]);

  const update = (id: string, patch: Partial<KVRow>) =>
    onChange(rows.map((r) => (r.id === id ? { ...r, ...patch } : r)));
  const remove = (id: string) => onChange(rows.filter((r) => r.id !== id));

  const handleRow0Blur = (e: React.FocusEvent<HTMLDivElement>) => {
    // If focus is still within the first row (e.g. moving between key and value inputs), don't prepend
    if (e.currentTarget.contains(e.relatedTarget as Node)) {
      return;
    }
    if (rows[0] && (rows[0].key.trim() || rows[0].value.trim())) {
      onChange([{ id: mkRowId(), enabled: true, key: "", value: "" }, ...rows]);
    }
  };

  const handleRow0KeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (rows[0] && (rows[0].key.trim() || rows[0].value.trim())) {
        onChange([{ id: mkRowId(), enabled: true, key: "", value: "" }, ...rows]);
        setTimeout(() => {
          topKeyInputRef.current?.focus();
        }, 10);
      }
    }
  };

  const defaultEmpty = readOnly ? strings.common.noHeaders : strings.common.noHeadersAddRow;

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center border-b border-border/60 bg-background/20 flex-shrink-0">
        {!readOnly && (
          <div className="w-9 flex-shrink-0 flex items-center justify-center border-r border-border/40">
            <input
              type="checkbox"
              checked={rows.length > 0 && rows.every((r) => r.enabled)}
              onChange={(e) => {
                if (rows.length === 0) return;
                onChange(rows.map((r) => ({ ...r, enabled: e.target.checked })));
              }}
              disabled={rows.length === 0}
              className="accent-signal cursor-pointer disabled:opacity-40"
              title="Toggle all"
            />
          </div>
        )}
        {mockControls && !readOnly && (
          <div className="w-20 flex-shrink-0 border-r border-border/40 px-2 py-1.5">
            <div className="flex flex-col gap-1">
              <label className="flex items-center gap-1 text-[10px] text-muted-foreground cursor-pointer">
                <input
                  type="checkbox"
                  checked={mockControls.allMocked}
                  onChange={(e) => mockControls.onToggleAll(e.target.checked)}
                  className="accent-signal cursor-pointer"
                />
                <span>Mock all</span>
              </label>
              <button
                type="button"
                onClick={() => mockControls.onToggleAll(false)}
                disabled={!mockControls.anyMocked}
                className="text-left text-[10px] text-muted-foreground hover:text-foreground disabled:opacity-40 cursor-pointer"
              >
                Unmock all
              </button>
            </div>
          </div>
        )}
        <div className="flex-1 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground border-r border-border/40">
          {strings.common.key}
        </div>
        <div className="flex-1 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          {strings.common.value}
        </div>
        {!readOnly && <div className="w-9 flex-shrink-0" />}
      </div>

      {readOnly && rows.length === 0 && (
        <p className="px-4 py-5 text-xs text-muted-foreground italic">{emptyMessage ?? defaultEmpty}</p>
      )}

      {rows.map((row, index) => {
        const isFirstRow = index === 0 && !readOnly;
        const isTopEmpty = isFirstRow && !row.key && !row.value;

        return (
          <div
            key={row.id}
            onBlur={isFirstRow ? handleRow0Blur : undefined}
            onKeyDown={isFirstRow ? handleRow0KeyDown : undefined}
            className={`flex items-stretch border-b border-border/25 last:border-0 group hover:bg-card/30 transition-colors ${
              !row.enabled && !readOnly ? "opacity-40" : ""
            }`}
          >
            {!readOnly && (
              <div className="w-9 flex-shrink-0 flex items-center justify-center border-r border-border/30">
                <input
                  type="checkbox"
                  checked={row.enabled}
                  onChange={(e) => update(row.id, { enabled: e.target.checked })}
                  className="accent-signal cursor-pointer"
                />
              </div>
            )}
            {mockControls && !readOnly && (
              <div className="w-20 flex-shrink-0 flex items-center justify-center border-r border-border/30">
                <label className="flex items-center gap-1.5 text-[10px] text-muted-foreground cursor-pointer">
                  <input
                    type="checkbox"
                    checked={!!row.mocked}
                    onChange={(e) => update(row.id, { mocked: e.target.checked })}
                    className="accent-signal cursor-pointer"
                  />
                  <span>Mock</span>
                </label>
              </div>
            )}
            <div className="flex-1 border-r border-border/25 min-w-0">
              <input
                ref={isFirstRow ? topKeyInputRef : undefined}
                className="w-full h-full bg-transparent font-mono text-xs px-3 py-2 outline-none focus:bg-card/60 min-w-0"
                style={{ color: "var(--c-signal)" }}
                placeholder={readOnly ? "—" : strings.editor.placeholderKey}
                value={row.key}
                onChange={(e) => update(row.id, { key: e.target.value })}
                readOnly={readOnly}
                tabIndex={readOnly ? -1 : undefined}
              />
            </div>
            <div className="flex-1 min-w-0">
              <input
                className="w-full h-full bg-transparent font-mono text-xs text-foreground px-3 py-2 outline-none focus:bg-card/60 min-w-0"
                placeholder={readOnly ? "—" : strings.editor.placeholderValue}
                value={row.value}
                onChange={(e) => update(row.id, { value: e.target.value })}
                readOnly={readOnly}
                tabIndex={readOnly ? -1 : undefined}
              />
            </div>
            {!readOnly && (
              <div className="w-9 flex-shrink-0 flex items-center justify-center">
                {!isTopEmpty && (
                  <button
                    type="button"
                    onClick={() => remove(row.id)}
                    className="w-full h-full flex items-center justify-center text-muted-foreground hover:text-destructive opacity-0 group-hover:opacity-100 transition-all cursor-pointer"
                    title={strings.common.delete}
                  >
                    <X size={13} />
                  </button>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

export { HeaderTable };
