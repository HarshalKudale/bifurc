/**
 * `plan/17` work item 5 — the image's GUI static server, over **real sockets and real files**.
 *
 * It is an integration test rather than a unit test because the things worth pinning here are the ones a
 * mock would define away: what `Content-Type` a browser is actually handed, whether a traversal reaches the
 * filesystem, and whether `HEAD` really sends no body.
 *
 * ## ⚠ Why every case binds port **0**
 *
 * `TRAPS.md` records that a suite which binds ports degrades every *"the scan lands on `base + N`"*
 * assertion in the tree, and prescribes the shared `holdBaseWithFreeSuccessors()` as the remedy. This suite
 * needs no such remedy, and the reason is deliberate: it asks the OS for an **ephemeral** port (`port: 0`)
 * rather than a number near a scan base, so it cannot occupy a port another suite's scan expects to find
 * free. `createGuiStaticServer` reports the port it actually bound, which is what makes that possible.
 */
import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createGuiStaticServer, safeResolve } from "../../scripts/gui-static-server.mjs";

/** `index.html` is the only unhashed document; the assets stand in for Vite's content-hashed output. */
const INDEX_HTML = "<!doctype html><html><body><div id=root></div><script src=./assets/app-abc123.js></script></body></html>";
const APP_JS = "console.log('bundle');";
const APP_CSS = ".a{color:red}";
/** Sits **beside** the root, so a traversal that escaped would find it. */
const OUTSIDE_SECRET = "TOP-SECRET-OUTSIDE-THE-ROOT";

let parent: string;
let root: string;
let server: { url: string; port: number; close: () => Promise<void> };

beforeAll(async () => {
  parent = await mkdtemp(path.join(tmpdir(), "bifurc-gui-"));
  root = path.join(parent, "web");
  await mkdir(path.join(root, "assets"), { recursive: true });
  await writeFile(path.join(root, "index.html"), INDEX_HTML);
  await writeFile(path.join(root, "assets", "app-abc123.js"), APP_JS);
  await writeFile(path.join(root, "assets", "app-abc123.css"), APP_CSS);
  // A file whose name *starts* with `..` — legitimate, and the reason the guard is a prefix test on the
  // resolved path rather than a scan for `..`.
  await writeFile(path.join(root, "..not-a-traversal.txt"), "fine");
  await writeFile(path.join(parent, "secret.txt"), OUTSIDE_SECRET);

  server = await createGuiStaticServer({ root, port: 0 });
});

afterAll(async () => {
  await server?.close();
  await rm(parent, { recursive: true, force: true });
});

describe("gui static server — the bundle it is handed", () => {
  it("binds an ephemeral port and reports it, rather than assuming the one it was asked for", () => {
    expect(server.port).toBeGreaterThan(0);
    expect(server.url).toBe(`http://127.0.0.1:${server.port}`);
  });

  it("serves the document at / as html and does not cache it", async () => {
    const res = await fetch(`${server.url}/`);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("text/html; charset=utf-8");
    // `index.html` names the hashed assets, so caching it would keep a rebuilt container serving the
    // previous build's script names.
    expect(res.headers.get("cache-control")).toBe("no-cache");
    expect(await res.text()).toBe(INDEX_HTML);
  });

  it("serves a hashed asset with its own type and an immutable cache", async () => {
    const js = await fetch(`${server.url}/assets/app-abc123.js`);
    expect(js.status).toBe(200);
    expect(js.headers.get("content-type")).toBe("text/javascript; charset=utf-8");
    expect(js.headers.get("cache-control")).toBe("public, max-age=31536000, immutable");
    expect(await js.text()).toBe(APP_JS);

    const css = await fetch(`${server.url}/assets/app-abc123.css`);
    expect(css.headers.get("content-type")).toBe("text/css; charset=utf-8");
  });

  it("answers a document request for an unknown path with index.html — the SPA fallback", async () => {
    const res = await fetch(`${server.url}/workspaces/abc/settings`, {
      headers: { accept: "text/html,application/xhtml+xml" },
    });
    expect(res.status).toBe(200);
    expect(await res.text()).toBe(INDEX_HTML);
  });

  it("404s a missing asset instead of answering it with HTML", async () => {
    // The discriminating half: a browser asks for a missing `.js` with a wildcard Accept, and answering it
    // with `index.html` would surface as a syntax error rather than a 404.
    const res = await fetch(`${server.url}/assets/gone-zzz999.js`, { headers: { accept: "*/*" } });
    expect(res.status).toBe(404);
    expect(await res.text()).not.toContain("<div id=root>");
  });

  it("sends no body for HEAD", async () => {
    const res = await fetch(`${server.url}/`, { method: "HEAD" });
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("text/html; charset=utf-8");
    expect(await res.text()).toBe("");
  });

  it("refuses a method it does not implement", async () => {
    const res = await fetch(`${server.url}/`, { method: "POST", body: "x" });
    expect(res.status).toBe(405);
    expect(res.headers.get("allow")).toBe("GET, HEAD");
  });
});

describe("gui static server — the root it will not leave", () => {
  it("refuses an encoded traversal rather than serving the file it names", async () => {
    const res = await fetch(`${server.url}/%2e%2e%2fsecret.txt`);
    expect(res.status).toBe(400);
    expect(await res.text()).not.toContain(OUTSIDE_SECRET);
  });

  it("refuses a plain traversal", async () => {
    const res = await fetch(`${server.url}/../secret.txt`);
    // `fetch` normalises a plain `..` away before it is sent, so this arrives as `/secret.txt` — which is
    // inside the root and simply absent. Either way the file outside is never served.
    expect(await res.text()).not.toContain(OUTSIDE_SECRET);
  });

  it("serves a legitimate filename that merely starts with dots", async () => {
    const res = await fetch(`${server.url}/..not-a-traversal.txt`);
    expect(res.status).toBe(200);
    expect(await res.text()).toBe("fine");
  });
});

describe("safeResolve — the guard, on its own", () => {
  it("resolves an ordinary path inside the root", () => {
    expect(safeResolve("/srv/web", "/assets/a.js")).toBe(path.resolve("/srv/web/assets/a.js"));
  });

  it("returns null for a decoded traversal", () => {
    expect(safeResolve("/srv/web", "/%2e%2e%2fetc%2fpasswd")).toBeNull();
    expect(safeResolve("/srv/web", "/../etc/passwd")).toBeNull();
  });

  it("returns null for a NUL byte and for a malformed escape", () => {
    expect(safeResolve("/srv/web", "/a%00b")).toBeNull();
    expect(safeResolve("/srv/web", "/%E0%A4%A")).toBeNull();
  });

  it("treats the root itself as inside the root", () => {
    expect(safeResolve("/srv/web", "/")).toBe(path.resolve("/srv/web"));
  });
});
