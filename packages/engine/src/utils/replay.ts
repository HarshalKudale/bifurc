/**
 * `replayRequest` — send a saved request to its target and return the response, for the **replay**
 * feature and for the proxy's own request path.
 *
 * ## Why this is in `utils/` and not in `proxy/`
 *
 * It lived at `proxy/serverReplay.ts` until engine-layout phase 7, and it is genuinely shared: the
 * proxy's routing path (`proxy/routingUtils.ts`, via `proxy/server.ts`'s re-export) needs it, and so
 * does the `request.replay` command — which phase 7 moved to `rest/` per `Engine_Module_Plan.md` §4.5.
 * `rest/`'s contract (§6) permits importing `kernel/`, `utils/` and `workspace/` but **not** `proxy/`,
 * because §5.1 forbids two feature modules importing each other.
 *
 * §5.1's own remedy for exactly this case is to extract the shared behaviour *down*: *"Where two
 * features genuinely share behaviour (the mock engine, the certificate manager, the rebind seam), that
 * behaviour is extracted down into a shared module rather than imported sideways."* This file is that
 * extraction. It is the same shape as phase 4's move of `proxy/{decompressUtils,responseUtils,serverUtils}`
 * into `utils/{decompress,response,server}`, which happened for the same reason.
 *
 * It qualifies as a `utils/` file by §5.1's test — *pure and stateless, no module-level caches, no
 * filesystem, no config reads*: it imports two Node builtins and two sibling `utils/` modules, holds no
 * state, and is a pure function of its arguments (its only effect is the outbound request itself, which
 * is the point of calling it).
 *
 * ## The `agent: false` trap, carried over verbatim
 *
 * Each request gets its own non-pooling agent so the `connection: close` header is actually honoured.
 * Without it the global agent returns the socket to its pool whenever the upstream answers `keep-alive`
 * (very common — servers routinely ignore the request's close), and the next request to the same origin
 * is rejected by the server with `HPE_CLOSED_CONNECTION`, surfacing as a spurious 400 / "socket hang up".
 */
import * as http from "http";
import * as https from "https";
import { HOP_BY_HOP } from "./http";
import { decompressBody, stripContentEncoding } from "./decompress";

export function replayRequest(
  method: string,
  url: string,
  headers: Record<string, string>,
  bodyBase64: string,
): Promise<{ status: number; headers: Record<string, string>; body: string }> {
  return new Promise((resolve, reject) => {
    let hostname: string;
    let port: number;
    let path: string;
    try {
      const u = new URL(url);
      hostname = u.hostname;
      port = u.port ? parseInt(u.port, 10) : (u.protocol === "https:" ? 443 : 80);
      path = u.pathname + u.search;
    } catch {
      return reject(new Error("Invalid URL"));
    }

    const body = bodyBase64 ? Buffer.from(bodyBase64, "base64") : Buffer.alloc(0);
    const upHeaders: Record<string, string> = { ...headers, connection: "close" };
    if (body.length > 0) upHeaders["content-length"] = String(body.length);

    const isHttps = new URL(url).protocol === "https:";
    const transport = isHttps ? https : http;
    // `agent: false` gives each request its own non-pooling agent, so the
    // `connection: close` above is actually honoured. Without it the global agent
    // returns the socket to its pool whenever the upstream answers `keep-alive`
    // (very common — servers routinely ignore the request's close), and the next
    // request to the same origin is rejected by the server with
    // `HPE_CLOSED_CONNECTION`, surfacing as a spurious 400 / "socket hang up".
    const req = transport.request({ hostname, port, path, method, headers: upHeaders, agent: false }, (res) => {
      const chunks: Buffer[] = [];
      res.on("data", (c: Buffer) => chunks.push(c));
      res.on("end", () => {
        const resHeaders: Record<string, string> = {};
        for (const [k, v] of Object.entries(res.headers)) {
          if (v != null && !HOP_BY_HOP.has(k.toLowerCase()))
            resHeaders[k] = Array.isArray(v) ? v.join(", ") : v;
        }
        const raw = Buffer.concat(chunks);
        const ce = resHeaders["content-encoding"] ?? "";
        const decompressed = decompressBody(raw, ce);
        const logHeaders = ce ? stripContentEncoding(resHeaders) : resHeaders;
        resolve({
          status: res.statusCode ?? 0,
          headers: logHeaders,
          body: decompressed.toString("base64"),
        });
      });
    });
    req.on("error", reject);
    if (body.length > 0) req.write(body);
    req.end();
  });
}
