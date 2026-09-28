import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Default autosave debounce.
 *
 * The timer measures **silence, not the first keystroke**: every snapshot change restarts it,
 * so a burst of typing produces exactly one save, one second after the user stops. A shorter
 * delay would fire mid-word; a longer one makes the "Saved" indicator feel broken.
 */
export const AUTOSAVE_DELAY_MS = 1000;

export interface DebouncedAutosaveOptions {
  /**
   * Serialized snapshot of the editable state. Compared by string, so the caller must
   * serialise deterministically (the tab reducers already do — `JSON.stringify` of the
   * draft projection).
   */
  snapshot: string;
  /**
   * Autosave runs only while true. A draft has no server row to update — it is created
   * explicitly by the user — so callers pass `false` for drafts and `true` once the entity
   * exists.
   */
  enabled: boolean;
  /** The write itself. Identity may change every render; it is held in a ref. */
  onSave: () => Promise<unknown> | unknown;
  delay?: number;
}

export interface DebouncedAutosave {
  /** A save is in flight. */
  saving: boolean;
  /** Timestamp of the last successful save, or `null` if none has happened yet. */
  savedAt: number | null;
  /** Message from the last failed save, cleared by the next success. */
  error: string | null;
  /**
   * Save now, cancelling the pending debounce, and return whatever `onSave` returned.
   *
   * This is what Ctrl+S and Sync route to, so the shortcut means "don't wait" rather than
   * "save" — the save was going to happen anyway. Returning the value lets Sync recover the
   * id from the create/update response.
   */
  flush: () => Promise<unknown>;
}

/**
 * Save `snapshot` one second after it stops changing.
 *
 * Three properties are load-bearing:
 *
 * 1. **Arming is silent.** `baselineRef` starts as `null`; the first snapshot seen while
 *    `enabled` becomes the baseline *without* saving. That is what makes `enabled` flipping
 *    false→true safe — creating an entity (draft → saved id) must not immediately write it
 *    back. Without this, the create path would fire a redundant update on every tab open, and
 *    a panel whose data arrives asynchronously would write its own initial load straight back.
 * 2. **The baseline only moves on success.** A failed save leaves the snapshot dirty, so a later
 *    `flush` — or the next edit — retries rather than the change being silently swallowed.
 * 3. **Unmount flushes.** Closing a tab inside the debounce window would otherwise drop the last
 *    keystroke. The tab strip keeps inactive tabs mounted (`display: none`), so this is about
 *    *closing*, not switching.
 */
export function useDebouncedAutosave({
  snapshot,
  enabled,
  onSave,
  delay = AUTOSAVE_DELAY_MS,
}: DebouncedAutosaveOptions): DebouncedAutosave {
  const [saving, setSaving] = useState(false);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const onSaveRef = useRef(onSave);
  onSaveRef.current = onSave;

  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;

  const snapshotRef = useRef(snapshot);
  snapshotRef.current = snapshot;

  /**
   * The snapshot last written to the server — or the one that was current when autosave was
   * armed. `null` means "not armed yet".
   */
  const baselineRef = useRef<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inFlightRef = useRef(0);

  const clearTimer = useCallback(() => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const runSave = useCallback(async (target: string) => {
    inFlightRef.current += 1;
    setSaving(true);
    try {
      const result = await onSaveRef.current();
      baselineRef.current = target;
      setSavedAt(Date.now());
      setError(null);
      return result;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return undefined;
    } finally {
      inFlightRef.current -= 1;
      if (inFlightRef.current === 0) setSaving(false);
    }
  }, []);

  useEffect(() => {
    if (!enabled) {
      // Disarmed: forget the baseline so re-arming cannot compare against a stale snapshot, and
      // drop any timer that was already counting down.
      baselineRef.current = null;
      clearTimer();
      return;
    }
    if (baselineRef.current === null) {
      baselineRef.current = snapshot;
      return;
    }
    if (snapshot === baselineRef.current) return;

    // Deliberately no `clearTimer()` on this path: returning it as the cleanup below means React
    // cancels the previous timer before this effect re-runs. That *is* the reset, and it is the one
    // mechanism — a second explicit call would make each look redundant and hide a regression.
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      void runSave(snapshot);
    }, delay);
    return clearTimer;
  }, [snapshot, enabled, delay, runSave, clearTimer]);

  const flush = useCallback(async () => {
    if (!enabledRef.current) return undefined;
    clearTimer();
    if (baselineRef.current === null || snapshotRef.current === baselineRef.current) return undefined;
    return runSave(snapshotRef.current);
  }, [clearTimer, runSave]);

  // A tab closed mid-debounce must not lose its last edit.
  useEffect(
    () => () => {
      clearTimer();
      if (!enabledRef.current) return;
      if (baselineRef.current === null || snapshotRef.current === baselineRef.current) return;
      void runSave(snapshotRef.current);
    },
    [clearTimer, runSave],
  );

  return { saving, savedAt, error, flush };
}
