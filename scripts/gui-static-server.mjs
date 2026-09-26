/**
 * The web GUI's static server — the image's half of `plan/17` work item 5.
 *
 * ## Why the engine does not serve this
 *
 * `plan/15`'s model is that the GUI is a **target**, not a directory: the edge routes the base domain to
 * `guiTarget` and never learns what is behind it (`EdgeRouterOptions.guiTarget` is a `host:port` string).
 * A document root inside the engine would put a second responsibility behind that flag and give the engine
 * a file-serving surface it has no other use for. So the entrypoint owns the static server and hands the
 * engine a `--gui-target` — see `docker-entrypoint.mjs`.
 *
 * ## Why there are no dependencies
 *
 * The runtime image is `npm prune --omit=dev`, so there is no `express`, no `serve-handler` and no `mime`.
 * Adding one would mean shipping a static-file library as a *production* dependency of a package that does
 * not serve files, or a second install in the runtime stage — both worse than the ~110 lines below.
 *
 * ## It binds loopback, and that is not a limitation
 *
 * The image's rule is *"Never default to `0.0.0.0`"* (`plan/05` work item 5), and the GUI is no exception:
 * it is reached **through the edge** on the base domain, or directly on the loopback of a container sharing
 * the host's network namespace (`network_mode: host`). The GUI itself is never published and never needs to
 * be: the edge reaches it on loopback *inside* the container, which is the whole reason `--gui-target` exists.
 *
 * This paragraph used to explain that an operator who published the edge with `--edge-allow-non-loopback` did
 * not need the GUI published either. That flag was removed on 2026-09-25 — the edge binds loopback
 * unconditionally — so there is no publishing decision left for the paragraph to describe.
 */
import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import path from "node:path";

/**
 * The extensions a Vite bundle actually contains.
 *
 * An unknown extension is served as `application/octet-stream` rather than guessed: a wrong `Content-Type`
 * on a `.js` is a page that fails to execute with a MIME error in the console, which is a worse failure
 * than a download. The bundle's own types are all in this table, so the fallback should be unreachable.
 */
const MIME = new Map([
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".mjs", "text/javascript; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".map", "application/json; charset=utf-8"],
  [".svg", "image/svg+xml"],
  [".png", "image/png"],
  [".jpg", "image/jpeg"],
  [".jpeg", "image/jpeg"],
  [".gif", "image/gif"],
  [".ico", "image/x-icon"],
  [".webp", "image/webp"],
  [".woff", "font/woff"],
  [".woff2", "font/woff2"],
  [".ttf", "font/ttf"],
  [".txt", "text/plain; charset=utf-8"],
]);

/** `stat`, or `null` when the path does not exist. Any other error still throws. */
async function statOrNull(p) {
  try {
    return await stat(p);
  } catch (err) {
    if (err && err.code === "ENOENT") return null;
    throw err;
  }
}

/**
 * The file a request path names, or `null` if it escapes `root`.
 *
 * **Decoded before it is checked, and that order is the point.** `%2e%2e%2f` is a traversal that never
 * appears as `..` in the raw URL, so a guard that pattern-matched the *encoded* path would pass it and
 * `path.resolve` would then walk out of the root. Decoding first means the guard sees what the filesystem
 * will see.
 *
 * The check is a **prefix test on the resolved path** rather than a scan for `..`: `path.resolve` has
 * already collapsed every `..`, `.` and duplicate separator, so there is nothing left to pattern-match —
 * and a scan would reject a legitimate file named `..foo`.
 */
export function safeResolve(root, requestPath) {
  let decoded;
  try {
    decoded = decodeURIComponent(requestPath);
  } catch {
    // A malformed percent-escape. `decodeURIComponent` throws `URIError`, which is not an fs error and
    // would otherwise escape as a 500 for what is plainly a bad request.
    return null;
  }
  // A NUL byte truncates the path in some syscalls; Node refuses it, but refusing it here keeps the
  // contract "null means the request was not a path we will serve".
  if (decoded.includes("\0")) return null;

  /**
   * `root` is resolved **here** rather than trusted from the caller, because the guard below compares two
   * paths and both have to be in the same form. An unresolved root (`/srv/web`) compared against a resolved
   * path (`I:\srv\web\assets\a.js` on Windows) fails the prefix test for **every** request, legitimate ones
   * included — so the "guard" becomes a total refusal.
   *
   * `createGuiStaticServer` happens to pass an already-resolved root, which is precisely why this survived
   * the socket tests and was caught only by exercising the exported function directly.
   */
  const base = path.resolve(root);
  const resolved = path.resolve(base, decoded.replace(/^\/+/, ""));
  if (resolved !== base && !resolved.startsWith(base + path.sep)) return null;
  return resolved;
}

/**
 * `true` when the request is for a **document**.
 *
 * This is the condition for the SPA fallback, and it has to be `text/html` specifically rather than "not a
 * known asset": a browser requests a missing `.js` with a wildcard `Accept` header, so a looser test would
 * answer a missing script with `index.html` and the failure would surface as a syntax error instead of a 404.
 */
function wantsHtml(req) {
  const accept = req.headers.accept ?? "";
  return accept.includes("text/html");
}

function sendFile(res, filePath, { headOnly }) {
  const parent = path.basename(path.dirname(filePath));
  const headers = {
    "content-type": MIME.get(path.extname(filePath).toLowerCase()) ?? "application/octet-stream",
    /**
     * Vite emits **content-hashed** filenames, so an asset's bytes never change under a given name and it
     * can be cached forever. `index.html` is the file that *names* those hashed assets, so it must not be
     * cached — otherwise a rebuilt container would keep handing out the previous build's script names.
     */
    "cache-control":
      parent === "assets" ? "public, max-age=31536000, immutable" : "no-cache",
  };
  res.writeHead(200, headers);
  if (headOnly) {
    res.end();
    return;
  }
  const stream = createReadStream(filePath);
  stream.on("error", () => res.destroy());
  stream.pipe(res);
}

/**
 * Start the GUI's static server and resolve once it is **listening**.
 *
 * Resolving on `listening` rather than on `createServer` matters to the caller: the entrypoint reports the
 * GUI's URL before the host starts, and a port that was still unbound at that moment would make the log a
 * claim rather than a fact.
 *
 * `port: 0` is honoured and the real port reported back, which is what lets a test bind an ephemeral port.
 */
export async function createGuiStaticServer({ root, port, host = "127.0.0.1" }) {
  const absoluteRoot = path.resolve(root);
  const indexFile = path.join(absoluteRoot, "index.html");

  const server = createServer((req, res) => {
    handle(req, res, absoluteRoot, indexFile).catch(() => {
      if (!res.headersSent) res.writeHead(500, { "content-type": "text/plain; charset=utf-8" });
      res.end("Internal error");
    });
  });

  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, host, resolve);
  });

  const address = server.address();
  const boundPort = address && typeof address === "object" ? address.port : port;

  return {
    port: boundPort,
    host,
    url: `http://${host}:${boundPort}`,
    close: () =>
      new Promise((resolve) => {
        server.close(() => resolve());
      }),
  };
}

async function handle(req, res, root, indexFile) {
  if (req.method !== "GET" && req.method !== "HEAD") {
    res.writeHead(405, { allow: "GET, HEAD", "content-type": "text/plain; charset=utf-8" });
    res.end("Method not allowed");
    return;
  }

  const pathname = new URL(req.url ?? "/", "http://localhost").pathname;
  const target = safeResolve(root, pathname);
  if (target === null) {
    res.writeHead(400, { "content-type": "text/plain; charset=utf-8" });
    res.end("Bad request");
    return;
  }

  const direct = await statOrNull(target);
  if (direct?.isFile()) {
    sendFile(res, target, { headOnly: req.method === "HEAD" });
    return;
  }

  // A directory is not a file: `/` resolves to the root itself, and serving it as a stream would fail.
  const asIndex = direct?.isDirectory() ? path.join(target, "index.html") : null;
  if (asIndex && (await statOrNull(asIndex))?.isFile()) {
    sendFile(res, asIndex, { headOnly: req.method === "HEAD" });
    return;
  }

  /**
   * The SPA fallback: a document request for a path the bundle does not contain is the UI's own route, so
   * it gets `index.html` and the client router takes it from there. Anything else — a missing script, a
   * missing font — is a genuine 404, because answering it with HTML is how a broken asset becomes a
   * confusing parse error.
   */
  if (wantsHtml(req) && (await statOrNull(indexFile))?.isFile()) {
    sendFile(res, indexFile, { headOnly: req.method === "HEAD" });
    return;
  }

  res.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
  res.end("Not found");
}
