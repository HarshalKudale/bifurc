/**
 * The **protocol-module ratchet** — `plan/19` WI 7, and the guard for `Protocol_Module_Contract.md`.
 *
 * ## What this is for
 *
 * `tests/gui/boundary.test.ts` asks *"is a piece of engine logic implemented in the renderer?"*, and answers
 * it by **name**, for the constructs `plan/19` §1 enumerated (resolvers, randomizers, the pipeline, the
 * report renderer, cURL parsing, path helpers). This file asks the complementary question:
 *
 * > **Does a tab speak the wire itself?**
 *
 * A protocol tab may call `window.api`. It may **not** construct a socket, open a stream, or reach for a
 * transport library — because that is precisely how a new protocol module lands in the wrong place. MQTT is
 * the module `Protocol_Module_Contract.md` exists for, and the failure this test prevents is not "MQTT is
 * slow to write" but "MQTT is written in the renderer, and the next five phases inherit it".
 *
 * ## Why it is a second file rather than a seventh construct in the boundary ratchet
 *
 * The two scan for different **categories** and are emptied by different work. The boundary allowlist holds
 * engine *logic* and emptied when the last renderer resolver went; this one holds engine *I/O* and emptied
 * when the last tab stopped opening its own connection — `hooks/useWebSocket.ts`, which was `plan/19` WI 6.
 * Folding them together would have given one allowlist whose entries retire for unrelated reasons, and the
 * per-construct staleness assertion (below) is only readable when an entry's owner is one work item.
 *
 * ## Detection is textual, and the gap is recorded
 *
 * The patterns match a **construction**, not an import: `new WebSocket(`, `new EventSource(`,
 * `http.request(`, `net.connect(`, `dgram.createSocket(`, `new XMLHttpRequest(`, and a bare `fetch(`. A
 * transport reached through an alias, or a dependency added to `packages/gui/package.json`, is invisible
 * here. The stronger form — asserting the gui package declares no socket library — is cheap and belongs with
 * the first module that needs one; `Protocol_Module_Contract.md` §6 records it rather than pretending this
 * scan is complete.
 *
 * **⚠ "Detection is by definition, not by use" is only half true, and the false half bit this file's own
 * change.** The scan reads the whole file as **text**, so `strings.sockets.createSocket` is correctly not a
 * violation (the pattern needs the `dgram.` qualifier) — but a *comment* that quotes a construction **is**
 * one. That is not hypothetical: `useWebSocket.ts`'s new docblock described what it used to do by writing
 * the constructor out, and this suite failed on it. The docblock was reworded, which is the right fix —
 * **a false positive is safe and a false negative is not**, so a scan that errs toward flagging prose is
 * better than one that strips comments and can therefore hide a real construction on the same line as a
 * `//` inside a string literal.
 *
 * ## ⚠ The staleness assertion **used** to be this file's vacuity guard, and no longer can be
 *
 * While the allowlist held one entry, `the allowlist has no stale entries` did double duty: it passed only
 * if the scan found that live violation, so a `detect` regex that silently matched nothing — the failure mode
 * a name-based ratchet is most prone to, and the reason the boundary ratchet needs a separate "the tree is
 * non-empty" case — failed this run instead of passing it quietly.
 *
 * **With `ALLOWED` empty there is no entry whose liveness depends on a hit, so that property is gone.** The
 * guard is therefore explicit: `each detect pattern still matches a real construction` runs every pattern
 * against a sample that must match and one that must not. Without it, emptying the allowlist would have
 * silently removed the one thing that proved the scanner worked.
 */
import fs from "node:fs";
import path from "node:path";
import { describe, it, expect } from "vitest";

const ROOT = process.cwd();
const GUI_SRC = path.join(ROOT, "packages", "gui", "src");

/**
 * The transport constructions a renderer must not contain, each with what it would mean.
 *
 * The list is deliberately short and concrete. It is not "everything that could reach a network" — a
 * classifier that tried to be complete would flag `window.api` itself. It is the handful of ways a
 * contributor would actually reach for a socket from inside a component.
 */
const CONSTRUCTS: Record<string, { what: string; detect: RegExp }> = {
  websocket: { what: "a raw WebSocket", detect: /new\s+WebSocket\s*\(/ },
  eventsource: { what: "a raw EventSource (server-sent events)", detect: /new\s+EventSource\s*\(/ },
  httpRequest: { what: "a Node http/https request", detect: /\b(?:http|https)\.request\s*\(/ },
  netSocket: { what: "a raw TCP socket", detect: /\bnet\.(?:connect|createConnection)\s*\(/ },
  udpSocket: { what: "a raw UDP socket", detect: /\bdgram\.createSocket\s*\(/ },
  xhr: { what: "a raw XMLHttpRequest", detect: /new\s+XMLHttpRequest\s*\(/ },
  fetch: { what: "a bare fetch()", detect: /(?<![\w.])fetch\s*\(/ },
};

/**
 * Constructions that exist today. **Keyed by `gui/src`-relative `file#construct`**, so each entry names one
 * exact construction rather than a whole file — a file-level key would silently cover the next one added to
 * that file.
 *
 * **`{}` — empty, and the goal the file opened with.** It opened at one entry,
 * `hooks/useWebSocket.ts#websocket`, owned by `plan/19` WI 6; that entry retired with the file's rewrite,
 * because the socket is now the engine's (`packages/engine/src/websocket/`) and the hook reaches it through
 * `window.api` like every other client.
 *
 * The value must name the work item that deletes an entry, which `every allowlist entry names the work item
 * that deletes it` asserts — debt with no owner outlives the phase, which is the failure `plan/19` §11 warns
 * about. That assertion now passes vacuously; it is kept because the next entry added is exactly its case.
 */
const ALLOWED: Record<string, string> = {};

/**
 * The positive control — **what replaces the vacuity guard the allowlist used to provide for free**.
 *
 * See the docblock. Each pattern is run against a sample that must match and one that must not; the samples
 * are the constructions themselves rather than prose, which is also what documents what each pattern is
 * looking for. A pattern that stopped matching fails here rather than leaving every assertion above green
 * while measuring nothing.
 *
 * The negatives are chosen as **near misses**, the same discipline `COMMAND_FIXTURES` uses: a longer
 * identifier that merely starts with the same text (`WebSocketServer`), an unqualified call (`createSocket`),
 * a property access that is not a request (`https.get`), and a name that contains the token without being
 * it (`myFetch`) — because those are the shapes a real mistake would take, and a control that only tried
 * `"not a socket"` would pass for any regex at all.
 */
const PATTERN_CONTROL: Record<string, { matches: string; doesNotMatch: string }> = {
  websocket: { matches: `const s = new WebSocket("wss://example.test");`, doesNotMatch: "new WebSocketServer({ port: 0 });" },
  eventsource: { matches: `const e = new EventSource("/sse");`, doesNotMatch: `new EventSourcePolyfill("/sse");` },
  httpRequest: { matches: "https.request(options, onResponse);", doesNotMatch: "https.get(url, onResponse);" },
  netSocket: { matches: "const s = net.connect(8080);", doesNotMatch: "net.isIP(address);" },
  udpSocket: { matches: `const s = dgram.createSocket("udp4");`, doesNotMatch: `createSocket("udp4");` },
  xhr: { matches: "const x = new XMLHttpRequest();", doesNotMatch: "XMLHttpRequest.prototype.open.call(x);" },
  fetch: { matches: `const r = fetch("/api/health");`, doesNotMatch: `const r = myFetch("/api/health");` },
};

// ── scanning ──────────────────────────────────────────────────────────────────────────────────────

function walk(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) return walk(p);
    return /\.(ts|tsx)$/.test(p) ? [p] : [];
  });
}

/** `packages/gui/src/hooks/useWebSocket.ts` → `hooks/useWebSocket.ts`. */
function relToGui(abs: string): string {
  return path.relative(GUI_SRC, abs).split(path.sep).join("/");
}

type Hit = { file: string; construct: string };

function scan(): Hit[] {
  const hits: Hit[] = [];
  for (const abs of walk(GUI_SRC)) {
    const src = fs.readFileSync(abs, "utf8");
    for (const [name, c] of Object.entries(CONSTRUCTS)) {
      if (c.detect.test(src)) hits.push({ file: relToGui(abs), construct: name });
    }
  }
  return hits;
}

const HITS = scan();
const key = (h: Hit) => `${h.file}#${h.construct}`;

// ── the ratchet ───────────────────────────────────────────────────────────────────────────────────

describe("a protocol tab speaks window.api, not the wire (plan/19 WI 7)", () => {
  it("the scanner is looking at a real, non-empty renderer tree", () => {
    // Without this, a renamed or moved `packages/gui/src` makes every assertion below pass by measuring
    // nothing — the same vacuous-green failure both sibling ratchets guard against.
    expect(
      fs.existsSync(GUI_SRC),
      `${GUI_SRC} does not exist. The scan would find zero files and pass vacuously; a moved ` +
        `renderer must move this constant with it.`,
    ).toBe(true);
    expect(
      walk(GUI_SRC).length,
      "the renderer tree scanned to zero files — the ratchet is measuring nothing.",
    ).toBeGreaterThan(0);
  });

  it("no transport is constructed in the renderer, allowlist aside", () => {
    const unexpected = HITS.filter((h) => !(key(h) in ALLOWED)).map(
      (h) => `packages/gui/src/${h.file}   [constructs ${h.construct}: ${CONSTRUCTS[h.construct].what}]`,
    );
    expect(
      unexpected,
      `${unexpected.length} transport construction(s) are in the renderer and are not in ALLOWED. ` +
        `A tab renders what the engine did and sends what the user typed (Protocol_Module_Contract.md ` +
        `§1) — put the connection in an engine module and reach it over the protocol, or, if the copy is ` +
        `pre-existing debt, add it to ALLOWED naming the work item that deletes it.`,
    ).toEqual([]);
  });

  it("the allowlist has no stale entries — it can only shrink", () => {
    const live = new Set(HITS.map(key));
    const stale = Object.keys(ALLOWED).filter((k) => !live.has(k));
    expect(
      stale,
      `${stale.length} allowlist entr(ies) no longer describe a violation — the construction was moved ` +
        `out of the renderer or the file was renamed. Delete the entr(ies); the allowlist must not ` +
        `accumulate permission for code that is already engine-side.`,
    ).toEqual([]);
  });

  it("every allowlist entry names the work item that deletes it", () => {
    const unowned = Object.entries(ALLOWED)
      .filter(([, reason]) => !/^WI \d+\b/.test(reason))
      .map(([k, reason]) => `${k} — "${reason}"`);
    expect(
      unowned,
      `${unowned.length} allowlist entr(ies) do not name a work item. Debt with no owner is the ` +
        `failure mode plan/19 §11 warns about: the entry outlives the phase and becomes permanent.`,
    ).toEqual([]);
  });

  it("each detect pattern still matches a real construction", () => {
    // ⚠ **This is not decoration.** While `ALLOWED` held one entry, the staleness assertion above was also
    // this file's vacuity guard — it passed only if the scan found a live violation, so a `detect` regex that
    // silently stopped matching failed the run. With the allowlist empty that property is gone: every
    // assertion above could pass while the scanner matched nothing at all. So the control is explicit.
    const broken: string[] = [];
    for (const [name, c] of Object.entries(CONSTRUCTS)) {
      const control = PATTERN_CONTROL[name];
      if (!control) {
        broken.push(`${name}: no positive control — a pattern with nothing proving it matches is a pattern nobody can trust`);
        continue;
      }
      if (!c.detect.test(control.matches)) {
        broken.push(`${name}: did NOT match ${JSON.stringify(control.matches)}, which is the construction it exists to find`);
      }
      if (c.detect.test(control.doesNotMatch)) {
        broken.push(`${name}: matched ${JSON.stringify(control.doesNotMatch)}, which is a near miss and must not count`);
      }
    }
    // The control table and the pattern table are kept in step in **both** directions, so adding a construct
    // without a control fails here rather than silently going unproven.
    const orphaned = Object.keys(PATTERN_CONTROL).filter((n) => !(n in CONSTRUCTS));
    expect(orphaned, `${orphaned.length} control(s) name a construct that no longer exists.`).toEqual([]);
    expect(
      broken,
      `${broken.length} detect pattern(s) no longer behave as documented. The scan is measuring less than ` +
        `it claims, and an empty allowlist means nothing else here would have noticed.`,
    ).toEqual([]);
  });
});
