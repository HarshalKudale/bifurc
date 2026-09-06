import { useState, useEffect } from "react";

export type ColorMode = "dark" | "light";

const DEFAULT_MODE: ColorMode = "dark";

function applyMode(mode: ColorMode) {
  const html = document.documentElement;
  html.classList.remove("dark", "light");
  if (mode === "light") html.classList.add("light");

  // Sync Electron titlebar overlay
  const bg = mode === "dark" ? "#1c1e27" : "#f2f3f8";
  const fg = mode === "dark" ? "#47e8a0" : "#3a3c47";
  window.api?.setTitleBarOverlay?.(bg, fg);
}

/**
 * Dark/light mode preference persisted via Electron's main-process
 * settings store (app.json) rather than localStorage.
 */
export function useColorMode(): [ColorMode, (m: ColorMode) => void] {
  const [mode, setMode] = useState<ColorMode>(DEFAULT_MODE);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const stored = await window.api?.getTheme?.();
        if (!cancelled && (stored === "dark" || stored === "light")) {
          setMode(stored);
        }
      } finally {
        if (!cancelled) setLoaded(true);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    applyMode(mode);
    if (loaded) {
      window.api?.setTheme?.(mode);
    }
  }, [mode, loaded]);

  return [mode, setMode];
}
