import * as net from "net";
import * as tls from "tls";
import { generateHostCert } from "@/proxy/tlsCert";

export type DecryptedRequestCallback = (
  socket: tls.TLSSocket,
  method: string,
  rawTarget: string,
  headerLines: string[],
  bodyBuf: Buffer,
  hostname: string,
  port: number,
) => void;

export function interceptTls(
  clientSocket: net.Socket,
  hostname: string,
  port: number,
  onDecryptedRequest: DecryptedRequestCallback,
): void {
  generateHostCert(hostname).then((pair) => {
    // Respond to the CONNECT request before wrapping with TLS
    clientSocket.write("HTTP/1.1 200 Connection Established\r\n\r\n");

    const secureContext = tls.createSecureContext({ cert: pair.cert, key: pair.key });

    const tlsSocket = new tls.TLSSocket(clientSocket, {
      isServer: true,
      secureContext,
    });

    tlsSocket.on("error", () => tlsSocket.destroy());

    let buf = Buffer.alloc(0);
    let dispatched = false;

    tlsSocket.on("data", (chunk: Buffer) => {
      if (dispatched) return;
      buf = Buffer.concat([buf, chunk]);
      const sep = buf.indexOf("\r\n\r\n");
      if (sep === -1) return;
      dispatched = true;

      const headerSection = buf.slice(0, sep).toString("utf-8");
      const bodyBuf = buf.slice(sep + 4);
      const lines = headerSection.split("\r\n");
      const parts = lines[0].split(" ");
      const method = parts[0] ?? "GET";
      // rawTarget is relative path (e.g. "/api/users") since TLS is already terminated
      const rawTarget = parts[1] ?? "/";

      onDecryptedRequest(tlsSocket, method, rawTarget, lines.slice(1), bodyBuf, hostname, port);
    });
  }).catch(() => {
    try { clientSocket.write("HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n"); } catch { /* ignore */ }
    clientSocket.destroy();
  });
}
