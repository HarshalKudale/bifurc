/**
 * TLS interception — the `CONNECT` path.
 *
 * WHY THIS EXISTS
 * ---------------
 * `tests/proxy/tlsCert.test.ts` mocks `mkcert`, so it pins the *wiring* of
 * `loadCA` / `generateHostCert` but never produces a real certificate, and
 * `packages/engine/src/proxy/tlsIntercept.ts` sat at 0% — the highest-risk untested file in the repo.
 * This is the code that terminates TLS for every `https://` request a user routes
 * through Bifurc, so a bug here is a bug in the product's core promise.
 *
 * This suite generates a REAL CA (`createCA` from mkcert — not mocked), boots the
 * REAL proxy with TLS enabled, and speaks REAL TLS through a REAL `CONNECT` tunnel.
 *
 * The decisive assertion is a handshake performed with `rejectUnauthorized: true`
 * against the generated CA. That check is performed by Node/OpenSSL, not by our code:
 * it only succeeds if the leaf certificate is genuinely signed by our CA, is within
 * its validity window, and carries a SAN matching the requested hostname. No amount
 * of mocking can fake that, and it is exactly what a browser does after the user
 * installs the CA.
 *
 * Covered here:
 *   - the `200 Connection Established` handshake,
 *   - a per-host certificate that chains to the configured CA,
 *   - the negative control (a cert for host A must NOT validate for host B),
 *   - routing over the decrypted stream (mock, and a proxy rule to a real upstream),
 *   - passthrough to a real TLS upstream when nothing matches,
 *   - certificate caching across tunnels to the same host,
 *   - capture of the decrypted request on the `log:entry` stream,
 *   - the blind-tunnel fallback when `tlsEnabled` is false,
 *   - the `502` path when the CA is unreadable as a key.
 */

import { describe, it, expect, beforeAll, afterAll, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as net from "net";
import * as tls from "tls";
import * as https from "https";
import { createCert } from "mkcert";

import {
  createWorkspace,
  startProxy,
  startUpstream,
  getFreePort,
  mockFixture,
  ruleFixture,
  type RunningProxy,
  type WorkspaceFixture,
} from "./proxyHarness";
import { generateCA } from "@bifurc/engine/proxy/certManager";
import { isCALoaded } from "@bifurc/engine/proxy/tlsCert";
import { logEmitter, type RequestLogEntry } from "@bifurc/engine/proxy/logEmitter";

// ── Fixtures ──────────────────────────────────────────────────────────────────

let caDir: string;
let caCertPath: string;
let caKeyPath: string;
let caCertPem: string;
let caKeyPem: string;

beforeAll(async () => {
  caDir = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-tls-ca-"));
  // The production CA generator — a real RSA CA written to disk as PEM.
  const paths = await generateCA(caDir);
  caCertPath = paths.certPath;
  caKeyPath = paths.keyPath;
  caCertPem = fs.readFileSync(caCertPath, "utf-8");
  caKeyPem = fs.readFileSync(caKeyPath, "utf-8");
  expect(caCertPem).toContain("BEGIN CERTIFICATE");
});

afterAll(() => {
  try { fs.rmSync(caDir, { recursive: true, force: true }); } catch { /* best effort */ }
});

// ── Raw CONNECT + TLS plumbing ────────────────────────────────────────────────

interface Tunnel {
  socket: net.Socket;
  statusLine: string;
  /** Any bytes the proxy sent after the CONNECT response headers. */
  rest: Buffer;
}

/**
 * Open a raw TCP connection to the proxy and send a `CONNECT host:port`.
 * Resolves once the proxy has answered the CONNECT (headers complete) — i.e. before
 * any TLS bytes are exchanged, which is how a real client behaves.
 */
function connectTunnel(proxyPort: number, host: string, port: number): Promise<Tunnel> {
  return new Promise((resolve, reject) => {
    const socket = net.connect(proxyPort, "127.0.0.1");
    let buf = Buffer.alloc(0);
    const onData = (chunk: Buffer): void => {
      buf = Buffer.concat([buf, chunk]);
      const sep = buf.indexOf("\r\n\r\n");
      if (sep === -1) return;
      socket.off("data", onData);
      resolve({
        socket,
        statusLine: buf.slice(0, buf.indexOf("\r\n")).toString("utf-8"),
        rest: buf.slice(sep + 4),
      });
    };
    socket.on("data", onData);
    socket.on("error", reject);
    socket.on("connect", () => {
      socket.write(`CONNECT ${host}:${port} HTTP/1.1\r\nHost: ${host}:${port}\r\n\r\n`);
    });
  });
}

/** Upgrade a tunnelled socket to TLS, optionally verifying against a CA. */
function tlsUpgrade(
  socket: net.Socket,
  opts: { servername?: string; ca?: string; rejectUnauthorized?: boolean },
): Promise<tls.TLSSocket> {
  return new Promise((resolve, reject) => {
    const t = tls.connect(
      {
        socket,
        // RFC 6066 forbids an IP literal in SNI; Node warns if you pass one.
        servername: opts.servername && !/^\d+\.\d+\.\d+\.\d+$/.test(opts.servername)
          ? opts.servername
          : undefined,
        ca: opts.ca,
        rejectUnauthorized: opts.rejectUnauthorized ?? true,
      },
      () => resolve(t),
    );
    t.on("error", reject);
  });
}

interface RawResponse {
  status: number;
  headers: Record<string, string>;
  body: string;
}

/**
 * Write a plain HTTP/1.1 request over an established (TLS) socket and read one
 * response. Parsed by hand so the test does not depend on Node's client internals
 * coping with an externally-owned socket.
 */
function httpOverSocket(
  sock: tls.TLSSocket | net.Socket,
  opts: { host: string; path: string; method?: string; body?: string },
): Promise<RawResponse> {
  return new Promise((resolve, reject) => {
    let buf = Buffer.alloc(0);
    let headerEnd = -1;
    let contentLength = -1;
    let status = 0;
    let headers: Record<string, string> = {};
    let settled = false;

    const finish = (): void => {
      if (settled) return;
      settled = true;
      sock.off("data", onData);
      const body = headerEnd === -1 ? "" : buf.slice(headerEnd + 4).toString("utf-8");
      resolve({ status, headers, body });
    };

    const onData = (chunk: Buffer): void => {
      buf = Buffer.concat([buf, chunk]);
      if (headerEnd === -1) {
        const sep = buf.indexOf("\r\n\r\n");
        if (sep === -1) return;
        headerEnd = sep;
        const lines = buf.slice(0, sep).toString("utf-8").split("\r\n");
        status = parseInt(lines[0].split(" ")[1] ?? "0", 10);
        for (const line of lines.slice(1)) {
          const i = line.indexOf(":");
          if (i > 0) headers[line.slice(0, i).trim().toLowerCase()] = line.slice(i + 1).trim();
        }
        const cl = headers["content-length"];
        contentLength = cl ? parseInt(cl, 10) : -1;
      }
      if (contentLength >= 0 && buf.length - (headerEnd + 4) >= contentLength) finish();
    };

    sock.on("data", onData);
    sock.on("end", finish);
    sock.on("error", reject);

    const bodyBuf = opts.body ? Buffer.from(opts.body, "utf-8") : null;
    const head = [
      `${opts.method ?? "GET"} ${opts.path} HTTP/1.1`,
      `Host: ${opts.host}`,
      "Connection: close",
      ...(bodyBuf ? [`Content-Length: ${bodyBuf.length}`] : []),
      "",
      "",
    ].join("\r\n");
    sock.write(head);
    if (bodyBuf) sock.write(bodyBuf);

    // Safety net so a framing mistake fails loudly instead of hanging the suite.
    setTimeout(() => {
      if (!settled) {
        settled = true;
        sock.off("data", onData);
        reject(new Error(`timed out reading response from ${opts.path}`));
      }
    }, 8000).unref();
  });
}

/** A real HTTPS origin, so the passthrough path has something to reach. */
async function startHttpsUpstream(): Promise<{ port: number; close: () => Promise<void> }> {
  // mkcert's createCert requires a CA — it never issues self-signed leaf certs.
  const pair = await createCert({
    domains: ["localhost", "127.0.0.1"],
    validity: 365,
    ca: { cert: caCertPem, key: caKeyPem },
  });
  const server = https.createServer({ cert: pair.cert, key: pair.key }, (_req, res) => {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ upstream: "https" }));
  });
  const port = await getFreePort();
  await new Promise<void>((r) => server.listen(port, "127.0.0.1", r));
  return { port, close: () => new Promise<void>((r) => server.close(() => r())) };
}

/**
 * A mock that fully replaces the upstream.
 *
 * `isFullyMocked()` requires every response header key to be listed in
 * `mockedResponseHeaders`; without that the mock is treated as a *partial* overlay
 * and the proxy still calls the upstream (which for a fake host means a 502).
 */
function fullyMockedMock(overrides: Record<string, unknown> = {}): any {
  return mockFixture({
    responseHeaders: { "content-type": "application/json" },
    mockedResponseHeaders: ["content-type"],
    responseStatusMocked: true,
    responseBodyMocked: true,
    responseDelayMocked: true,
    ...overrides,
  });
}

/** A plain TCP echo server, to prove the non-intercepting path is a raw byte pipe. */
function startTcpEcho(): Promise<{ port: number; close: () => Promise<void> }> {
  const server = net.createServer((s) => { s.pipe(s); });
  return getFreePort().then(
    (port) =>
      new Promise((resolve) => {
        server.listen(port, "127.0.0.1", () =>
          resolve({
            port,
            close: () => new Promise<void>((r) => server.close(() => r())),
          }),
        );
      }),
  );
}

// ── Lifecycle ─────────────────────────────────────────────────────────────────

let ws: WorkspaceFixture | null = null;
let proxy: RunningProxy | null = null;

afterEach(() => {
  proxy?.stop();
  proxy = null;
  ws?.cleanup();
  ws = null;
});

const TLS_HOST = "secure.localhost";

/** Write a workspace with TLS interception enabled, pointing at the real generated CA. */
function tlsWorkspace(extra: Parameters<typeof createWorkspace>[0] = {}): void {
  ws = createWorkspace({
    tlsEnabled: true,
    tlsCaCertPath: caCertPath,
    tlsCaKeyPath: caKeyPath,
    ...extra,
  });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("TLS interception — CONNECT handling", () => {
  it("answers CONNECT with 200 Connection Established", async () => {
    tlsWorkspace();
    proxy = await startProxy();

    const tunnel = await connectTunnel(proxy.port, TLS_HOST, 443);
    expect(tunnel.statusLine).toBe("HTTP/1.1 200 Connection Established");
    expect(tunnel.rest.length).toBe(0); // the proxy waits for the ClientHello
    tunnel.socket.destroy();
  });

  it("presents a per-host certificate that chains to the configured CA", async () => {
    tlsWorkspace();
    proxy = await startProxy();

    const tunnel = await connectTunnel(proxy.port, TLS_HOST, 443);
    // rejectUnauthorized:true — OpenSSL verifies the chain and the hostname.
    const t = await tlsUpgrade(tunnel.socket, {
      servername: TLS_HOST,
      ca: caCertPem,
      rejectUnauthorized: true,
    });

    const cert = t.getPeerCertificate();
    expect(cert.subjectaltname ?? "").toContain(TLS_HOST);
    // The leaf must be signed by our CA, not self-signed.
    expect(cert.issuer.CN).not.toBe(cert.subject.CN);

    t.destroy();
  });

  it("does not validate a host's certificate against a different hostname", async () => {
    tlsWorkspace();
    proxy = await startProxy();

    const tunnel = await connectTunnel(proxy.port, TLS_HOST, 443);
    // The proxy mints a cert for secure.localhost; claiming a different SNI must fail.
    await expect(
      tlsUpgrade(tunnel.socket, {
        servername: "evil.localhost",
        ca: caCertPem,
        rejectUnauthorized: true,
      }),
    ).rejects.toThrow(/altname|hostname|ERR_TLS_CERT_ALTNAME_INVALID/i);
    tunnel.socket.destroy();
  });

  it("reuses the cached certificate for a second tunnel to the same host", async () => {
    tlsWorkspace();
    proxy = await startProxy();

    const first = await connectTunnel(proxy.port, TLS_HOST, 443);
    const t1 = await tlsUpgrade(first.socket, { servername: TLS_HOST, rejectUnauthorized: false });
    const raw1 = t1.getPeerCertificate().raw;

    const second = await connectTunnel(proxy.port, TLS_HOST, 443);
    const t2 = await tlsUpgrade(second.socket, { servername: TLS_HOST, rejectUnauthorized: false });
    const raw2 = t2.getPeerCertificate().raw;

    expect(raw1.length).toBeGreaterThan(0);
    expect(raw2.equals(raw1)).toBe(true);

    t1.destroy();
    t2.destroy();
  });
});

describe("TLS interception — routing over the decrypted stream", () => {
  it("serves a mock for the https URL", async () => {
    tlsWorkspace({
      mocks: [
        fullyMockedMock({
          id: "mock-tls",
          method: "GET",
          urlPattern: `https://${TLS_HOST}/api/tls`,
          responseBody: JSON.stringify({ mocked: "over-tls" }),
        }),
      ],
    });
    proxy = await startProxy();

    const tunnel = await connectTunnel(proxy.port, TLS_HOST, 443);
    const t = await tlsUpgrade(tunnel.socket, { servername: TLS_HOST, rejectUnauthorized: false });
    const res = await httpOverSocket(t, { host: TLS_HOST, path: "/api/tls" });

    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ mocked: "over-tls" });
    t.destroy();
  });

  it("routes the https request through a proxy rule to a real upstream", async () => {
    const upstream = await startUpstream();
    try {
      tlsWorkspace({
        rules: [
          ruleFixture({
            id: "rule-tls",
            pattern: `https://${TLS_HOST}/api/rule`,
            targetType: "external",
            targetExternal: upstream.target,
          }),
        ],
      });
      proxy = await startProxy();

      const tunnel = await connectTunnel(proxy.port, TLS_HOST, 443);
      const t = await tlsUpgrade(tunnel.socket, { servername: TLS_HOST, rejectUnauthorized: false });
      const res = await httpOverSocket(t, { host: TLS_HOST, path: "/api/rule" });

      expect(res.status).toBe(200);
      expect(JSON.parse(res.body)).toEqual({ upstream: true, path: "/api/rule" });
      expect(upstream.requests).toHaveLength(1);
      expect(upstream.requests[0].url).toBe("/api/rule");
      t.destroy();
    } finally {
      await upstream.close();
    }
  });

  it("passes an unmatched https request through to the real TLS upstream", async () => {
    const origin = await startHttpsUpstream();
    try {
      tlsWorkspace();
      proxy = await startProxy();

      const tunnel = await connectTunnel(proxy.port, "127.0.0.1", origin.port);
      const t = await tlsUpgrade(tunnel.socket, { servername: "127.0.0.1", rejectUnauthorized: false });
      const res = await httpOverSocket(t, { host: "127.0.0.1", path: "/anything" });

      expect(res.status).toBe(200);
      expect(JSON.parse(res.body)).toEqual({ upstream: "https" });
      t.destroy();
    } finally {
      await origin.close();
    }
  });

  it("captures the decrypted request on the log stream", async () => {
    tlsWorkspace({
      mocks: [
        fullyMockedMock({ id: "mock-cap", method: "GET", urlPattern: `https://${TLS_HOST}/api/cap` }),
      ],
    });
    proxy = await startProxy();

    const seen: RequestLogEntry[] = [];
    const onRequest = (e: RequestLogEntry): void => { seen.push(e); };
    logEmitter.on("request", onRequest);

    try {
      const tunnel = await connectTunnel(proxy.port, TLS_HOST, 443);
      const t = await tlsUpgrade(tunnel.socket, { servername: TLS_HOST, rejectUnauthorized: false });
      await httpOverSocket(t, { host: TLS_HOST, path: "/api/cap" });

      const entry = seen.find((e) => e.url.endsWith("/api/cap"));
      expect(entry).toBeDefined();
      expect(entry?.host).toBe(TLS_HOST);
      expect(entry?.url).toBe(`https://${TLS_HOST}/api/cap`);
      expect(entry?.method).toBe("GET");
      expect(entry?.via).toBe("mock");
      t.destroy();
    } finally {
      logEmitter.off("request", onRequest);
    }
  });
});

describe("TLS interception — the non-intercepting and failure paths", () => {
  it("blind-tunnels raw bytes when tlsEnabled is false", async () => {
    const echo = await startTcpEcho();
    try {
      // TLS disabled: the proxy must NOT terminate TLS, just pipe bytes.
      ws = createWorkspace({ tlsEnabled: false, tlsCaCertPath: caCertPath, tlsCaKeyPath: caKeyPath });
      proxy = await startProxy();
      expect(isCALoaded()).toBe(false);

      const tunnel = await connectTunnel(proxy.port, "127.0.0.1", echo.port);
      expect(tunnel.statusLine).toBe("HTTP/1.1 200 Connection Established");

      const echoed = await new Promise<string>((resolve, reject) => {
        tunnel.socket.once("data", (c: Buffer) => resolve(c.toString("utf-8")));
        tunnel.socket.on("error", reject);
        tunnel.socket.write("raw-bytes-not-tls");
      });
      expect(echoed).toBe("raw-bytes-not-tls");
      tunnel.socket.destroy();
    } finally {
      await echo.close();
    }
  });

  it("answers 502 when the configured CA cannot be used as a key", async () => {
    const badDir = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-tls-bad-"));
    const badCert = path.join(badDir, "ca-cert.pem");
    const badKey = path.join(badDir, "ca-key.pem");
    // Readable, so loadCA() succeeds and interception is attempted — but not a CA.
    fs.writeFileSync(badCert, "not a certificate", "utf-8");
    fs.writeFileSync(badKey, "not a key", "utf-8");

    try {
      ws = createWorkspace({ tlsEnabled: true, tlsCaCertPath: badCert, tlsCaKeyPath: badKey });
      proxy = await startProxy();

      const tunnel = await connectTunnel(proxy.port, TLS_HOST, 443);
      expect(tunnel.statusLine).toContain("502");
      tunnel.socket.destroy();
    } finally {
      try { fs.rmSync(badDir, { recursive: true, force: true }); } catch { /* best effort */ }
    }
  });

  it("drops the loaded CA when the server stops", async () => {
    tlsWorkspace();
    proxy = await startProxy();
    expect(isCALoaded()).toBe(true);

    proxy.stop();
    proxy = null;
    expect(isCALoaded()).toBe(false);
  });
});
