/**
 * The **reverse half's** three HTML pages — the home page, the "not mapped" 404, and the "not serving" 503.
 *
 * ## What left this file, and why
 *
 * It used to export `sendHtml()` as well, and that function was imported by **both** proxy halves: the edge
 * for these pages, and the forward half for its 502s. That made `proxy/` import `reverseproxy/` in three
 * files — a feature→feature edge §5.1 forbids and §6 does not grant. `sendHtml` is a pure, stateless socket
 * writer, so §5.1's remedy applied: it was extracted **down** into `utils/sendHtml.ts`, where both halves may
 * import it. The page chrome and the reason-phrase table went with it.
 *
 * What is left is what is genuinely the edge's: three functions that **build strings** from an `AppConfig`,
 * called only by `reverseproxy/edge.ts`. They are pure — no socket, no state — and they stay here because the
 * home page lists *this* half's mappings and the other two pages are answers only this half gives.
 */
import { AppConfig } from "../workspace/config";

export function buildHomePage(cfg: AppConfig, currentPort: number): string {
  const active = cfg.mappings.filter((m) => m.enabled);
  const portSuffix = currentPort === 80 ? "" : `:${currentPort}`;
  const rows = active.map((m) =>
    `<li><span class="dot"></span><a href="http://${m.domain}${portSuffix}/" class="domain">${m.domain}</a><span class="arr">→</span><span class="tgt">${m.target}</span></li>`
  ).join("");
  return `<div class="logo"><div class="logo-icon"></div><span class="logo-name">Bifurc</span></div>
<div class="badge">Port ${currentPort}</div>
<h1>Local proxy is active</h1>
<p>Requests to a mapped domain are routed to that mapping's target. <code>*.localhost${portSuffix}</code> needs no setup; any other domain needs a DNS or hosts entry pointing at this engine.</p>
${active.length > 0 ? `<ul>${rows}</ul>` : `<p style="margin-top:1rem;color:#5b8dee">No mappings yet. Open the Bifurc app to add one.</p>`}`;
}

export function buildNotMappedPage(host: string, currentPort: number): string {
  const portSuffix = currentPort === 80 ? "" : `:${currentPort}`;
  return `<div class="logo"><div class="logo-icon"></div><span class="logo-name">Bifurc</span></div>
<div class="badge">Not Mapped</div>
<h1>${host}</h1>
<p>This domain is not mapped to any local service.</p>
<p>Open <a href="http://localhost${portSuffix}/">Bifurc</a> and add a mapping for <code>${host}</code>.</p>`;
}

/**
 * The edge's page for "this host is not serving the active workspace" (`plan/15` work item 3).
 *
 * It exists because both alternatives are a lie, in opposite directions. Rendering the ordinary home page
 * with an empty mapping list says *"No mappings yet — open the Bifurc app to add one"*, which is false when
 * the workspace has plenty and this host simply is not serving it (it was skipped for a taken port
 * override, or filtered out by `--workspace`). Rendering **another** workspace's mappings is worse: it
 * would send the browser to a target the active workspace never configured — a wrong-target answer of
 * exactly the kind `runtimeRegistry.ts` exists to prevent.
 *
 * 503 rather than 200: the edge is up and cannot serve this, so the state belongs to the host rather than
 * to the request. The port is not named, because the request already reached it.
 */
export function buildUnservedPage(): string {
  return `<div class="logo"><div class="logo-icon"></div><span class="logo-name">Bifurc</span></div>
<div class="badge">Not Serving</div>
<h1>No workspace is being served</h1>
<p>The edge is listening, but this host is not running the active workspace — it was skipped (a port override that was taken) or filtered out of the workspace list.</p>
<p>The host's startup log names the workspace it declined to start, and why.</p>`;
}
