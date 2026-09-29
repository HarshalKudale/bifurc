/**
 * Which hosts count as **loopback**, for a bind guard.
 *
 * ## Why this is its own module rather than a private helper in `transport/ws.ts`
 *
 * It was private there until `plan/15` work item 2 gave the **proxy** a bind host, and with it a second
 * guard that has to ask exactly this question. There were two ways to share it and only one of them is
 * sound:
 *
 * - `proxy/server.ts` importing `isLoopbackHost` from `../transport/ws` would work, and it drags the whole
 *   transport stack — the `ws` package, `node:http`, `node:https`, and `assertBridgeIsTotal()`'s
 *   module-scope side effect — into every import path that reaches the proxy server. Measured, that matters:
 *   `tests/proxy/server.test.ts` mocks `http` and `https` outright, and a proxy module that now loads the
 *   WebSocket server to learn a two-element array is a dependency nobody asked for.
 * - A dependency-free module that both guards import is one definition with no edge between the layers.
 *
 * **A second copy is the thing to avoid.** `LOOPBACK_HOSTS.includes(host.toLowerCase())` written again in
 * `proxy/server.ts` is how "the transport thinks `::1` is remote but the proxy thinks it is loopback"
 * happens — and the two would disagree in the *permissive* direction, which is the direction that publishes
 * a port. `transport/ws.ts` re-exports both names, so its own callers and the transport barrel are unchanged.
 */

/**
 * Hosts treated as loopback for a bind guard.
 *
 * `localhost` is included because it resolves to one — and it is the spelling a user writes, so a guard that
 * accepted `127.0.0.1` and refused `localhost` would be refusing the form it is most likely to see.
 *
 * Compared **case-insensitively** (see `isLoopbackHost`): `LOCALHOST` and `LocalHost` are the same host, and
 * a DNS name is case-insensitive by definition.
 */
export const LOOPBACK_HOSTS: readonly string[] = ["127.0.0.1", "::1", "localhost"];

/**
 * The hostname every engine answers as. `localhost` — and it is no longer a *default* for anything.
 *
 * **It stopped being a default on 2026-09-24, when `--domain` was removed** (`runtime/hostArgs.ts`'s header
 * records the removal and the reasoning). The engine is no longer deployable in the cloud: it runs inside the
 * Electron app or as a locally deployed container, so the name it is addressed as is a fact of the deployment
 * rather than a choice an operator makes. `runtime/host.ts` composes it directly for `status()` and for the
 * RPC's `auth.hostDomain`, and `reverseproxy/edge.ts` defaults its own `hostDomain` to it.
 *
 * **Kept separate from `LOOPBACK_HOSTS`, which contains the same string.** They are different facts: this one
 * says *what to call yourself*, the other says *what cannot be reached from elsewhere*. Until **2026-09-25** the
 * difference was visible in the configuration — a deployment could answer as `localhost` and bind a
 * non-loopback interface, via the `--edge-bind-host` opt-ins. Those opt-ins are gone and every listener binds
 * loopback unconditionally, so the two constants now happen to describe the same deployments; they are kept
 * apart anyway, because the *questions* are still distinct and folding them would make "answer as localhost"
 * and "bind loopback" read as one decision — which is the mistake that would be made again by whoever adds the
 * next bind option.
 */
export const DEFAULT_HOST_DOMAIN = "localhost";

/**
 * Is `host` a loopback address?
 *
 * The predicate every bind guard uses — `transport/ws.ts`'s, `proxy/server.ts`'s and `reverseproxy/edge.ts`'s — and
 * `isAllowedOrigin()` below, which has to tell "this page is on this machine" from "this page is not".
 *
 * It used to have two other callers, both of which decided a bind **policy** before reaching a guard:
 * `runtime/hostArgs.ts`, so an opt-in against a loopback host was refused rather than accepted and ignored, and
 * `runtime/host.ts`, where the fail-closed token rule had to tell a published bind from a local one. Both went
 * on **2026-09-25** with the opt-ins and the token. There is no policy left to decide, so the guards are what
 * remains — which is the change this predicate's callers record.
 *
 * Not a general-purpose address classifier, and deliberately so: it answers "would binding this expose the
 * engine?", which is a question about the *string* a caller passed, not about what it resolves to. A
 * hostname that resolves to `127.0.0.1` is not in the list and is therefore treated as remote — the
 * fail-closed direction, which is the only direction a bind guard may err in.
 */
export function isLoopbackHost(host: string): boolean {
  return LOOPBACK_HOSTS.includes(host.toLowerCase());
}

/**
 * Would a browser page loaded from `origin` be **on this machine**?
 *
 * ## Why loopback binding is not enough on its own
 *
 * Every listener in this engine binds loopback, and for a long time that was treated as the authorisation
 * story. It is not one, because **a web page is a local process**: it runs on the same machine as the
 * engine, so its connection to `127.0.0.1` *is* a loopback connection. A WebSocket handshake is not gated
 * by the same-origin policy the way `fetch` is, so without this predicate any page the user visits could
 * open a socket to the RPC endpoint and drive the engine — which executes scripts and spawns processes.
 *
 * This is the check that replaces the engine token for loopback use. It answers a question the token could
 * not be *asked*: not "does this peer hold a secret" but "is this peer a page served from this machine".
 *
 * ## The rule, and each half of it
 *
 * | `Origin` | verdict | why |
 * |---|---|---|
 * | absent | **allowed** | not a browser. A browser always sends `Origin` on a WebSocket handshake, so this is the CLI, `curl`, the extension, or a test. |
 * | `localhost`, `127.0.0.1`, `[::1]` (any port) | **allowed** | the page is served from this machine |
 * | `<sub>.localhost` (any port) | **allowed** | the GUI's own mapping lives here — `bifurc.localhost` is a reverse-proxy mapping, and browsers resolve `*.localhost` to loopback by specification |
 * | anything else | **refused** | including `Origin: null`, which is what a `file://` page and a sandboxed iframe send |
 *
 * **`https:` and `http:` are both accepted**, because the scheme is not the security property here — the
 * *host* is. The engine binds plaintext loopback, so a `https://localhost` origin cannot be talking to it
 * anyway; refusing it would only break a caller whose GUI is fronted by a TLS terminator on the same box.
 *
 * ## What it defeats, stated precisely
 *
 * A page on `evil.com` is refused. **DNS rebinding is refused too**, and that is the case worth naming: the
 * trick makes `evil.com` *resolve* to `127.0.0.1`, so the request really does arrive on loopback — but the
 * browser still sends `Origin: http://evil.com`, which is foreign. Checking the origin rather than the
 * address is exactly what makes rebinding ineffective.
 *
 * It does **not** claim to stop a local process that is not a browser: such a process can already run code
 * on this machine, so a socket is not an escalation. That is the same reasoning `plan/05` work item 4
 * applies to `stdio` ("the pipe is inherited and process-scoped").
 *
 * ## Why it takes the host domain as a parameter
 *
 * The `*.localhost` half is a fact about **this deployment's name**, not a constant — `runtime/host.ts`
 * composes `DEFAULT_HOST_DOMAIN` and passes it down, the same value it reports in the `hello` response, so
 * "the name the engine answers as" and "the origin it accepts" cannot drift apart.
 */
export function isAllowedOrigin(origin: string | undefined, hostDomain: string): boolean {
  // Absent is not "unknown", it is "not a browser" — see the table above.
  if (origin === undefined || origin === "") return true;

  let url: URL;
  try {
    url = new URL(origin);
  } catch {
    // `Origin: null` (a `file://` page, a sandboxed iframe) and any other non-URL value land here.
    return false;
  }

  // `URL.hostname` keeps an IPv6 literal's brackets (`[::1]`), and `LOOPBACK_HOSTS` spells it without
  // them — so stripping them here is what keeps `http://[::1]:5199` from being read as a remote origin.
  const host = url.hostname.replace(/^\[|\]$/g, "").toLowerCase();

  if (isLoopbackHost(host)) return true;

  const domain = normalizeHostname(hostDomain);
  return host === domain || host.endsWith(`.${domain}`);
}

/**
 * Canonicalise a hostname: lower-case, and **one** trailing dot removed.
 *
 * ## Why a name needs canonicalising at all
 *
 * DNS is case-insensitive (RFC 4343) and a fully-qualified name may be written with the root's trailing dot
 * (RFC 1034) — so `App.Example.com`, `app.example.com` and `app.example.com.` are **one host with three
 * spellings**. Every *comparison* in this codebase already behaves as though that were true: `isLoopbackHost`
 * lower-cases before testing, and the edge router lower-cases the `Host` header before matching a mapping
 * (`reverseproxy/edge.ts`). What was missing was a place that produced the canonical form — and that matters as soon
 * as two **stored** names have to be compared, which is what the host's own name was against a mapping's
 * `domain` (`plan/15` §3.3's write-time collision refusal, and §4.3's editor). **The host's name stopped being
 * stored on 2026-09-24** — `--domain` was removed, so it is the constant below — and the remaining production
 * reader is `reverseproxy/edge.ts`'s own-name gate, which canonicalises the parameter it is given.
 *
 * The trailing dot is stripped rather than preserved because its only function is to *state* that the name is
 * fully qualified; two names differing only in that statement are the same name. Only one dot is removed:
 * `example.com..` is not a hostname, and a normaliser that quietly made it one would be hiding a typo instead
 * of leaving it to the validation that refuses it.
 *
 * ## What it deliberately is not
 *
 * Not a validator — it never rejects. Not a resolver — it never asks DNS. Not a trimmer — a space inside a
 * hostname is malformed rather than untidy, so `hostArgs.ts` refuses it instead of repairing it. One function
 * per question: this one answers "what is this name's canonical spelling?".
 */
export function normalizeHostname(host: string): string {
  return host.toLowerCase().replace(/\.$/, "");
}
