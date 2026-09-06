import React from "react";
import { Environment } from "@/types";
import { strings } from "@/lib/strings";
import EnvVarHint from "@/components/editor/EnvVarHint";
import RandomizerHint from "@/components/editor/RandomizerHint";
import { KVRow, mkRowId } from "@/lib/utils";

export function TokenToolbar({ env, onInsert }: { env: Environment | null; onInsert: (token: string) => void }) {
  return (
    <div className="flex items-center gap-1.5 px-3 py-1.5 border-b border-border/40 bg-background/10 flex-shrink-0 justify-end">
      <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider mr-1">{strings.editor.insert}</span>
      <EnvVarHint env={env} onInsert={onInsert} />
      <RandomizerHint onInsert={onInsert} />
    </div>
  );
}

export function appendTokenToFocusedRow(rows: KVRow[], token: string): KVRow[] {
  if (rows.length === 0) {
    return [{ id: mkRowId(), enabled: true, key: "", value: token }];
  }
  for (let i = rows.length - 1; i >= 0; i--) {
    if (rows[i].enabled) {
      return rows.map((r, idx) => idx === i ? { ...r, value: r.value + token } : r);
    }
  }
  return rows.map((r, idx) => idx === rows.length - 1 ? { ...r, value: r.value + token } : r);
}

export function insertAtActiveInput(token: string): boolean {
  const el = document.activeElement as HTMLInputElement | null;
  if (!el || el.tagName !== "INPUT" || el.readOnly) return false;
  const start = el.selectionStart ?? el.value.length;
  const end = el.selectionEnd ?? el.value.length;
  const newValue = el.value.slice(0, start) + token + el.value.slice(end);
  const nativeSetter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")?.set;
  if (!nativeSetter) return false;
  nativeSetter.call(el, newValue);
  el.dispatchEvent(new Event("input", { bubbles: true }));
  const cursorPos = start + token.length;
  setTimeout(() => el.setSelectionRange(cursorPos, cursorPos), 0);
  return true;
}
