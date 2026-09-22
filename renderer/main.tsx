/**
 * The Electron host's entry (D18).
 *
 * It is only a mount point now. The UI itself — `<App />` and all 211 files under it — lives in
 * `packages/gui`, which the browser host in `web/` imports as well. The two entries differ in exactly two
 * ways: **which `window.api` exists by the time they run**, and where their bundle is emitted.
 *
 * There is no bootstrap step here, and there is one in `web/main.tsx`, because `window.api` is already
 * assigned before this file executes: `src/preload.ts` exposes it through `contextBridge`, and a preload
 * runs before any page script.
 *
 * The stylesheet is imported through the `@` alias rather than by package subpath because it is a *file*
 * this bundle has to inline, not a value the package exports.
 */
import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/space-grotesk/400.css";
import "@fontsource/space-grotesk/500.css";
import "@fontsource/space-grotesk/600.css";
import "@fontsource/jetbrains-mono/400.css";
import "@/styles.css";
import { App } from "@bifurc/gui";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
