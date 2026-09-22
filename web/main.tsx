/**
 * The browser host's entry (D18).
 *
 * Compare it with `renderer/main.tsx`: both mount the **same** `<App />` from `packages/gui`, and the
 * only difference between them is what happens above the render. That is the whole point of the shared
 * UI — the divergence between the desktop and the browser is one line, not two codebases.
 *
 * Here it is an explicit install step, because a browser has no preload phase. The Electron entry has no
 * equivalent line, because `src/preload.ts` has already run by the time it executes.
 */
import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/space-grotesk/400.css";
import "@fontsource/space-grotesk/500.css";
import "@fontsource/space-grotesk/600.css";
import "@fontsource/jetbrains-mono/400.css";
import "@/styles.css";
import { App } from "@bifurc/gui";

import { installWindowApi } from "./api";

// Before the render, not after: `App` reads `window.api` during mount.
const { url, probe } = installWindowApi({
  onFatal: (error) => console.error(`Bifurc: lost the engine at ${url} —`, error.message),
});

/**
 * Prove the round trip, then hand over to the UI.
 *
 * `plan/08` work item 1's whole verification is "a decoupled client actually reaches an engine from a
 * page". With a real UI on top, a failure to connect now shows up as a broken-looking screen, which is
 * ambiguous — so the probe is kept and reported on the console, where it says unambiguously whether the
 * engine answered. Removing this is work item 2's job, when the UI renders connection state itself.
 */
void probe().then(
  () => console.info(`Bifurc: engine at ${url} answered.`),
  (error: unknown) =>
    console.error(
      `Bifurc: no engine at ${url} —`,
      error instanceof Error ? error.message : String(error),
    ),
);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
