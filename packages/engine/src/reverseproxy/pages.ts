import * as net from "net";
import { AppConfig } from "../workspace/config";

/**
 * The reason phrase for a status code.
 *
 * `sendHtml` used to write `OK` for **every** status, so the 404 page went out as `HTTP/1.1 404 OK` and a
 * 502 as `502 OK`. That is legal — RFC 9110 makes the reason phrase advisory and says a client must not
 * rely on it — and every caller here is a browser or a test, both of which read the code. It is still
 * wrong, and it survived only until a new page had to be added: writing `503 OK` in fresh code is harder
 * to defend than inheriting an old wart, and the fix is smaller than the paragraph explaining it.
 */
const REASON_PHRASES: Record<number, string> = {
  200: "OK",
  400: "Bad Request",
  404: "Not Found",
  500: "Internal Server Error",
  502: "Bad Gateway",
  503: "Service Unavailable",
};

export function sendHtml(socket: net.Socket, status: number, htmlBody: string): void {
  if (!socket.writable) return;
  const page = wrapPage(htmlBody);
  const buf = Buffer.from(page, "utf-8");
  socket.write(
    `HTTP/1.1 ${status} ${REASON_PHRASES[status] ?? ""}\r\n` +
    `content-type: text/html; charset=utf-8\r\n` +
    `content-length: ${buf.length}\r\n` +
    `connection: close\r\n\r\n`
  );
  socket.write(buf);
  socket.end();
}

function wrapPage(body: string): string {
  return `<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Bifurc</title><style>*{box-sizing:border-box;margin:0;padding:0}body{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,sans-serif;background:#0f1117;color:#c9d1e9;min-height:100vh;display:flex;align-items:center;justify-content:center;padding:2rem}.wrap{background:#171b27;border:1px solid #2d3550;border-radius:12px;padding:2.5rem 3rem;max-width:560px;width:100%}.logo{display:flex;align-items:center;gap:10px;margin-bottom:1.5rem}.logo-icon{width:28px;height:28px;background:linear-gradient(135deg,#5b8dee,#3ecf8e);border-radius:7px}.logo-name{font-size:13px;font-weight:700;color:#e8eaf6;letter-spacing:.02em}.badge{display:inline-flex;padding:3px 10px;border-radius:20px;border:1px solid #2d3550;font-size:10px;font-weight:700;letter-spacing:.1em;text-transform:uppercase;color:#6b7799;margin-bottom:1rem}h1{font-size:22px;font-weight:700;color:#e8eaf6;margin-bottom:.6rem;word-break:break-all}p{font-size:13px;color:#6b7799;line-height:1.6;margin-bottom:.75rem}a{color:#5b8dee;text-decoration:none}a:hover{text-decoration:underline}code{font-family:monospace;background:#262d3f;padding:2px 6px;border-radius:4px;font-size:12px;color:#c9d1e9}ul{list-style:none;margin-top:1rem}li{padding:10px 0;border-bottom:1px solid #2d3550;display:flex;align-items:center;gap:10px;font-size:13px}li:last-child{border-bottom:none}.dot{width:6px;height:6px;border-radius:50%;background:#3ecf8e;flex-shrink:0;box-shadow:0 0 6px #3ecf8e}.domain{font-family:monospace;color:#e8eaf6;font-weight:600}.arr{color:#2d3550;margin:0 4px}.tgt{font-family:monospace;font-size:12px;color:#6b7799}</style></head><body><div class="wrap">${body}</div></body></html>`;
}

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
