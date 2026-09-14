import { useEffect, useRef } from "react";
import { readStorageRaw, removeStorage, writeStorage } from "@/lib/storage";

// NOTE (Phase 2.8):
// This hook and usePersistedState cannot be reasonably consolidated due to significantly different APIs and use cases.
// - usePersistedState acts as a useState drop-in replacement that synchronously syncs React state to localStorage.
// - useDraftPersist is a specialized debouncing hook that pulls data via a getter (avoiding per-keystroke React re-renders) 
//   and manages complex lifecycle requirements (like discarded tabs and clearing on save).

const PREFIX = "lp:draft:";

// Tab IDs explicitly discarded by closeTab - unmount must not re-save them.
const discarded = new Set<string>();

export function saveDraft(tabId: string, data: unknown): void {
  if (discarded.has(tabId)) return;
  writeStorage(PREFIX + tabId, data);
}

export function loadDraft<T>(tabId: string): T | null {
  try {
    const raw = readStorageRaw(PREFIX + tabId);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch { return null; }
}

export function clearDraft(tabId: string): void {
  discarded.add(tabId);
  removeStorage(PREFIX + tabId);
}

/** Return all draft tab IDs currently stored (for a given id prefix). */
export function getDraftIds(idPrefix: string): string[] {
  const ids: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const k = localStorage.key(i);
    if (k && k.startsWith(PREFIX + idPrefix)) {
      ids.push(k.slice(PREFIX.length));
    }
  }
  return ids;
}

/**
 * Call inside an editor to auto-save `getData()` to localStorage.
 * Pass `tabId=null` for saved (non-draft) tabs - hook becomes a no-op.
 * Pass `isEmpty` to suppress saving while all fields are blank.
 * Call `markSaved()` when the user officially saves; the draft is then
 * cleared on unmount instead of being flushed.
 */
export function useDraftPersist(
  tabId: string | null,
  getData: () => unknown,
  isEmpty?: () => boolean,
): { markSaved: () => void } {
  const savedRef = useRef(false);
  const dataRef = useRef(getData);
  const isEmptyRef = useRef(isEmpty);
  dataRef.current = getData;
  isEmptyRef.current = isEmpty;

  const markSaved = () => { savedRef.current = true; };

  const shouldSkip = () => !!(isEmptyRef.current && isEmptyRef.current());

  // Debounced auto-save on every render (data changes trigger re-render)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null); useEffect(() => {
    if (!tabId) return;
    if (savedRef.current) return;
    if (discarded.has(tabId)) return;
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      if (!savedRef.current && !discarded.has(tabId) && !shouldSkip()) {
        saveDraft(tabId, dataRef.current());
      }
    }, 400);
  });

  // Save immediately on mount to ensure new drafts are persisted
  useEffect(() => {
    if (!tabId) return;
    if (discarded.has(tabId)) return;
    // Save the draft immediately so it persists even if user switches panels without entering data
    saveDraft(tabId, dataRef.current());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabId]);

  // On unmount: flush immediately if not yet saved; clear if saved
  useEffect(() => {
    return () => {
      if (!tabId) return;
      if (timerRef.current) clearTimeout(timerRef.current);
      if (savedRef.current) {
        // User saved it - clear the draft
        clearDraft(tabId);
      } else if (!discarded.has(tabId)) {
        // Always save on unmount, even if empty (preserves new tabs when switching panels)
        saveDraft(tabId, dataRef.current());
      }
      // Clean up the discard entry once the component is gone
      discarded.delete(tabId);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabId]);

  return { markSaved };
}
