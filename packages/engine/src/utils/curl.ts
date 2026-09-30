/**
 * The **cURL parser** — one implementation, engine-side, shared by the `request.parseCurl` command
 * and the cURL importer (`importexport/importers/requests-curl.ts`).
 *
 * ## Why it is in `utils/`
 *
 * `Engine_Module_Plan.md` §5.1's placement rule is *what a module READS decides its home*. This
 * reads nothing — no file, no config, no sibling module — and it has two callers in two different
 * feature modules (`rest/` via the command, `importexport/` via the importer). That is exactly the
 * "stateless + shared → extract **down** into `utils/`" case, and it is why neither caller imports
 * the other.
 *
 * ## Where it came from, and the two copies it replaces
 *
 * `plan/19` WI 5. Before this file there were **two** parsers: the renderer's
 * `packages/gui/src/lib/curlParser.ts` and a near-verbatim copy inside the cURL importer whose own
 * comment read *"duplicated from packages/gui/src/lib/curlParser.ts for main process use"*. The
 * renderer's copy is **deleted** by WI 5 — the boundary ratchet (`tests/gui/boundary.test.ts`) is
 * what made that checkable rather than remembered — and the importer now delegates here.
 *
 * ## What the two copies did differently, and what that means for the importer
 *
 * The renderer's parser was the more complete of the two, so it is the one that became the
 * authority. The importer's private copy therefore **gains** two things it never had: `-b` /
 * `--cookie` (folded into a `cookie` header) and `--data-ascii`. Both are supersets — a cURL
 * command that used them previously produced an empty body or a missing cookie, and now does not.
 * The one thing the importer had that this does not is its `null` contract (*"not a curl command"*
 * / *"no url"*), which stays in the importer as a thin wrapper rather than as a second parser —
 * see `parseCurlCommand()` below, which is that wrapper, exported so the importer does not have to
 * re-derive it.
 *
 * ## `SKIP_CURL_HEADERS` is here, and the renderer keeps a pinned copy
 *
 * The set is *data*, not a parser, and the renderer needs it for one thing the engine cannot do for
 * it: filtering a stored mock's `capturedHeaders` when that mock is loaded into a tab
 * (`restTabHelpers.ts#buildMockReqHeaders`). The renderer cannot import this file — `@bifurc/gui`
 * has **no** dependency on `@bifurc/engine` by design (`plan/19` finding 1) — so the copy is
 * hand-mirrored and **pinned by `tests/gui/curlHeaderSkip.test.ts`**, which is the same
 * list-vs-logic split `plan/19` D28 made for `RANDOMIZER_TOKENS`: the engine holds the authority,
 * the UI holds a copy, and the pin is what makes the copy legitimate rather than a second
 * implementation nobody checks.
 */

/**
 * Hop-by-hop and connection headers that are dropped when a cURL command or a captured request is
 * turned into an editable request. They describe *one* connection, not the request's intent, so
 * replaying them is wrong and showing them is noise.
 */
export const SKIP_CURL_HEADERS = new Set([
  "host", "proxy-connection", "connection", "content-length", "transfer-encoding",
]);

/** Shell-ish tokenizer: handles single quotes (literal), double quotes (backslash-escaped), and bare words. */
export function tokenizeCurl(input: string): string[] {
  const str = input.replace(/\\\r?\n/g, " ");
  const tokens: string[] = [];
  let i = 0;
  while (i < str.length) {
    while (i < str.length && /[ \t]/.test(str[i])) i++;
    if (i >= str.length) break;
    if (str[i] === "'") {
      i++;
      let tok = "";
      while (i < str.length && str[i] !== "'") tok += str[i++];
      i++;
      tokens.push(tok);
    } else if (str[i] === '"') {
      i++;
      let tok = "";
      while (i < str.length && str[i] !== '"') {
        if (str[i] === "\\" && i + 1 < str.length) { i++; tok += str[i]; }
        else tok += str[i];
        i++;
      }
      i++;
      tokens.push(tok);
    } else {
      let tok = "";
      while (i < str.length && !/[ \t]/.test(str[i])) tok += str[i++];
      tokens.push(tok);
    }
  }
  return tokens;
}

/** The shape a cURL command parses to. Header keys are lower-cased. */
export interface ParsedCurl {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string;
}

/**
 * Parse a cURL command. **Never returns `null`** — a string that is not a cURL command parses to an
 * empty request (blank method, blank url), which is the renderer's long-standing behaviour and is
 * what lets the import field parse on every keystroke without special-casing a half-typed command.
 *
 * Headers are returned **unfiltered** — `SKIP_CURL_HEADERS` is the caller's decision, because the
 * two callers want different things from it (the tab drops them; the importer drops them too, but
 * at a different point in its pipeline).
 */
export function parseCurl(curlStr: string): ParsedCurl {
  const tokens = tokenizeCurl(curlStr);
  let method = "";
  let url = "";
  const headers: Record<string, string> = {};
  let body = "";
  let i = 0;
  while (i < tokens.length) {
    const tok = tokens[i];
    if (tok === "curl") { i++; continue; }
    if (tok === "-X" || tok === "--request") {
      if (++i < tokens.length) method = tokens[i].toUpperCase();
    } else if (tok === "-H" || tok === "--header") {
      if (++i < tokens.length) {
        const ci = tokens[i].indexOf(":");
        if (ci > 0) headers[tokens[i].slice(0, ci).trim().toLowerCase()] = tokens[i].slice(ci + 1).trim();
      }
    } else if (tok === "-b" || tok === "--cookie") {
      if (++i < tokens.length) {
        headers["cookie"] = headers["cookie"] ? headers["cookie"] + "; " + tokens[i] : tokens[i];
      }
    } else if (tok === "-d" || tok === "--data" || tok === "--data-raw" || tok === "--data-binary" || tok === "--data-ascii") {
      if (++i < tokens.length) body = tokens[i];
    } else if (tok === "--url") {
      if (++i < tokens.length) url = tokens[i];
    } else if (!tok.startsWith("-") && !url && (tok.startsWith("http://") || tok.startsWith("https://"))) {
      url = tok;
    }
    i++;
  }
  if (!method) method = body ? "POST" : "GET";
  return { method, url, headers, body };
}

/**
 * The importer's contract: a block is only a cURL command if it **starts with `curl`** and yields a
 * url, and its headers are filtered. Returns `null` otherwise, so the importer can skip a block.
 *
 * The `tokens[0] === "curl"` test is the importer's own rule (it splits a multi-command paste into
 * blocks and must not treat a stray line as one), which is why it lives here as a wrapper rather
 * than inside `parseCurl`.
 */
export function parseCurlCommand(curlStr: string): ParsedCurl | null {
  const trimmed = curlStr.trim();
  const tokens = tokenizeCurl(trimmed);
  if (!tokens.length || tokens[0] !== "curl") return null;
  const parsed = parseCurl(trimmed);
  if (!parsed.url) return null;
  const headers: Record<string, string> = {};
  for (const [k, v] of Object.entries(parsed.headers)) {
    if (!SKIP_CURL_HEADERS.has(k)) headers[k] = v;
  }
  return { ...parsed, headers };
}
