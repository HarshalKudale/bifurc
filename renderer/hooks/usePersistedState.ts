import { useState, useEffect, useRef } from "react";
import { readStorage, writeStorage } from "@/lib/storage";

// NOTE (Phase 2.8):
// This hook and useDraftPersist cannot be reasonably consolidated due to significantly different APIs and use cases.
// - usePersistedState acts as a useState drop-in replacement that synchronously syncs React state to localStorage.
// - useDraftPersist is a specialized debouncing hook that pulls data via a getter (avoiding per-keystroke React re-renders) 
//   and manages complex lifecycle requirements (like discarded tabs and clearing on save).

export function usePersistedState<T>(
  key: string,
  defaultValue: T,
  validate?: (v: T) => T,
): [T, React.Dispatch<React.SetStateAction<T>>] {
  const [state, setState] = useState<T>(() => {
    return readStorage(key, defaultValue, validate);
  });

  // Skip first render write so we don't overwrite a freshly read value
  const mounted = useRef(false);
  useEffect(() => {
    if (!mounted.current) { mounted.current = true; return; }
    writeStorage(key, state);
  }, [key, state]);

  return [state, setState];
}
