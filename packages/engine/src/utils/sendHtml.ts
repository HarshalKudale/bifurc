/**
 * `sendHtml()` — write a small HTML page to a raw socket and close it.
 *
 * ## Why this is in `utils/`
 *
 * It lived in `reverseproxy/pages.ts` and was imported by **both** proxy halves: the edge uses it for its
 * home / 404 / unserved pages, and the forward half uses it for its 502s. That made `proxy/` import
 * `reverseproxy/` in three files — a feature→feature edge §5.1 forbids and §6 does not grant. It is a
 * **pure, stateless** socket writer with no module state and no config read, so §5.1's own remedy applies:
 * extract the shared behaviour *down*, where both halves may import it.
 *
 * It is the same shape as phase 4's `proxy/{decompressUtils,responseUtils,serverUtils}` → `utils/` move and
 * phase 7's `proxy/serverReplay.ts` → `utils/replay.ts`.
 *
 * The three **builders** that shared the old file (`buildHomePage`, `buildNotMappedPage`,
 * `buildUnservedPage`) stayed in `reverseproxy/pages.ts`: they are called only by the edge, they take an
 * `AppConfig`, and the home page lists the reverse half's mappings. Those are the reverse half's pages; this
 * is the write.
 *
 * ## The reason phrase, and the wart it fixed
 *
 * `sendHtml` used to write `OK` for **every** status, so the 404 page went out as `HTTP/1.1 404 OK` and a
 * 502 as `502 OK`. That is legal — RFC 9110 makes the reason phrase advisory and says a client must not rely
 * on it — and every caller here is a browser or a test, both of which read the code. It is still wrong, and
 * it survived only until a new page had to be added: writing `503 OK` in fresh code is harder to defend than
 * inheriting an old wart, and the fix is smaller than the paragraph explaining it.
 *
 * ## The inline stylesheet is deliberate
 *
 * `wrapPage()` carries the whole page chrome as one string. It is a self-contained error page served on a
 * connection that is about to close, from a process that may be running inside a container with no static
 * file root — so there is nothing to fetch the CSS from, and a `<link>` would turn a 502 into a 502 plus a
 * failed request.
 */
import * as net from "net";

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
