/**
 * The browser host's entry.
 *
 * It mounts **`<Container />`**, not `<App />` — and that is the difference from `renderer/main.tsx`.
 *
 * The Electron entry mounts the UI directly because `window.api` is already assigned before it runs: a
 * preload executes before any page script, and `src/preload.ts` exposes the client through
 * `contextBridge`. A browser has no preload phase, and — unlike the Electron shell, whose transport is
 * `in-process` and has no boundary — it has to **authenticate before its first request**. So the browser
 * cannot install `window.api` and then render; it has to connect, and only then render. `Container`
 * owns that order.
 *
 * The consequence for this file is that it is short, and stays short: the ordering lives in one place
 * rather than being spread between an entry and a bootstrap module.
 */
import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/space-grotesk/400.css";
import "@fontsource/space-grotesk/500.css";
import "@fontsource/space-grotesk/600.css";
import "@fontsource/jetbrains-mono/400.css";
import "@/styles.css";
import iconUrl from "@/icon.png";

import { Container } from "./Container";

/**
 * The tab icon, taken from the shared UI rather than shipped as a second copy.
 *
 * Set from script because `index.html` cannot reference it: Vite resolves an `href` in the HTML relative to
 * the host's root, and this file lives in `packages/gui`. Without it the browser requests `/favicon.ico`,
 * which does not exist — and a **404 is a console error**, which is precisely the class of noise this host
 * was cleaned up to remove. Measured: it was the only one left.
 */
const favicon = document.createElement("link");
favicon.rel = "icon";
favicon.type = "image/png";
favicon.href = iconUrl;
document.head.appendChild(favicon);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Container />
  </React.StrictMode>
);
