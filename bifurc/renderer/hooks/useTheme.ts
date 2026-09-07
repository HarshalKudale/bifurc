import { useState, useEffect } from "react";

export type ColorMode = "dark" | "light";

const DEFAULT_MODE: ColorMode = "dark";
const TITLEBAR_THEME = {
  dark: { bg: "#090e12", fg: "#eef2f7" },
  light: { bg: "#eff2f6", fg: "#151b21" },
} as const;

function applyMode(mode: ColorMode) {
  const html = document.documentElement;
  html.classList.remove("dark", "light");
  if (mode === "light") html.classList.add("light");

  // Sync Electron titlebar overlay
  const { bg, fg } = TITLEBAR_THEME[mode];
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
