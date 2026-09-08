import * as http from "http";
import * as https from "https";
import { HOP_BY_HOP } from "./constants";
import { decompressBody, stripContentEncoding } from "./decompressUtils";

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
    const req = transport.request({ hostname, port, path, method, headers: upHeaders }, (res) => {
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
