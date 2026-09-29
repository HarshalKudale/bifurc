/**
 * `packages/engine/src/utils/net.ts` — the two facts about a hostname **string**.
 *
 * ## Why this file exists now, and did not before
 *
 * `isLoopbackHost` has been in the tree since `plan/15` work item 2 extracted it out of `transport/ws.ts`, and
 * until 2026-09-23 it had **no direct test at all** — it was covered only through the three bind guards that
 * call it, which means every assertion about it was really an assertion about `createWsServer`,
 * `createProxyServer` or `createEdgeRouter`. That is usually the right trade (test behaviour, not helpers), and
 * it stopped being the right trade when `hostArgs.ts` began using the predicate to decide whether a *flag
 * combination* is coherent: at that point "is `::1` loopback?" became a question with a user-visible answer, and
 * a question that a parser test could only ask indirectly.
 *
 * `normalizeHostname` was added in the same work and has the same property from the other direction: it is the
 * function that makes `Engine.Example.COM.` and `engine.example.com` one name, which the write-time collision
 * check (`plan/15` §3.3) will depend on. A canonicaliser whose two rules — fold case, strip one root dot — are
 * only exercised through a caller is a canonicaliser whose *second* rule is easy to lose, because every caller
 * that happens to pass lower-case input still passes.
 *
 * ## What is deliberately not tested
 *
 * Resolution. Neither function asks DNS or parses an address, and a test that appeared to check
 * `isLoopbackHost("127.0.0.1.nip.io")` would be asserting about the machine's resolver rather than about this
 * module. The cases below are all string-in, boolean-or-string-out.
 */
import { describe, expect, it } from "vitest";

import { DEFAULT_HOST_DOMAIN, isAllowedOrigin, isLoopbackHost, LOOPBACK_HOSTS, normalizeHostname } from "../../src/utils/net";

describe("isLoopbackHost", () => {
  it("accepts every spelling of loopback, case-insensitively", () => {
    // Case matters because a DNS name is case-insensitive and this predicate is what decides whether an
    // operator's opt-in is *coherent*. `LOCALHOST` refused here would refuse the form a person actually types.
    for (const host of ["127.0.0.1", "::1", "localhost", "LOCALHOST", "LocalHost"]) {
      expect(isLoopbackHost(host), host).toBe(true);
    }
  });

  it("treats 0.0.0.0 as NOT loopback — it is the bind that exposes the port", () => {
    // The one that matters most. `0.0.0.0` means "every interface", so a guard that accepted it as loopback
    // would skip the opt-in on precisely the bind the opt-in exists for.
    expect(isLoopbackHost("0.0.0.0")).toBe(false);
    expect(isLoopbackHost("::")).toBe(false);
  });

  it("treats the rest of 127/8 as NOT loopback, and that is the fail-closed direction", () => {
    // `127.0.0.2` *is* loopback in the RFC's terms — the whole of `127.0.0.0/8` is — and this predicate says
    // otherwise. Pinned here because it is a deliberate asymmetry rather than an oversight: the module's own
    // docblock records that the question is "would binding this expose the engine?" asked of the **string** a
    // caller passed, and the only direction a bind guard may err in is the one that demands an extra
    // acknowledgement. A future "fix" that added `127.0.0.2` here would be a fix that silently stopped
    // requiring the opt-in for a bind the operator may not have intended.
    expect(isLoopbackHost("127.0.0.2")).toBe(false);
  });

  it("treats an ordinary hostname as remote", () => {
    for (const host of ["example.com", "engine.example.com", "192.168.1.50", "bifurc"]) {
      expect(isLoopbackHost(host), host).toBe(false);
    }
  });

  it("has one list, and the predicate reads it rather than repeating it", () => {
    // Cheap, and it is the property the module exists for: `LOOPBACK_HOSTS` is exported so that a *second*
    // consumer cannot end up with a second copy — the disagreement that would publish a port.
    for (const host of LOOPBACK_HOSTS) expect(isLoopbackHost(host)).toBe(true);
    expect(LOOPBACK_HOSTS).toContain(DEFAULT_HOST_DOMAIN);
  });
});

describe("normalizeHostname", () => {
  it("lower-cases", () => {
    expect(normalizeHostname("Engine.Example.COM")).toBe("engine.example.com");
  });

  it("strips ONE trailing dot, not all of them", () => {
    // `example.com.` and `example.com` are one name (RFC 1034's root label). `example.com..` is not a name at
    // all — and a normaliser that made it one would be repairing a typo rather than leaving it to the validation
    // that refuses it, which is the whole reason this function does not validate.
    expect(normalizeHostname("engine.example.com.")).toBe("engine.example.com");
    expect(normalizeHostname("engine.example.com..")).toBe("engine.example.com.");
  });

  it("is idempotent, which is why applying it twice is safe", () => {
    // `hostArgs.ts` canonicalises the CLI value and `host.ts` canonicalises the option, so the value is
    // normalised twice on the real path. Harmless **only** because of this property — the reasoning the host's
    // option docblock records for why that double application is fine while a double *validation* would not be.
    for (const host of ["Engine.Example.COM.", "localhost", "192.168.1.50", "bifurc"]) {
      const once = normalizeHostname(host);
      expect(normalizeHostname(once), host).toBe(once);
    }
  });

  it("leaves a single-label name and an IP literal alone", () => {
    expect(normalizeHostname("bifurc")).toBe("bifurc");
    expect(normalizeHostname("192.168.1.50")).toBe("192.168.1.50");
    expect(normalizeHostname("LOCALHOST")).toBe("localhost");
  });

  it("does NOT trim — a space is malformed rather than untidy", () => {
    // The boundary this function deliberately stops at. Repairing `" engine.example.com"` here would mean the
    // parser's shape check never saw the input the operator wrote, so the typo would be silently accepted and
    // the value would reach the UI as a suffix with a leading space in it.
    expect(normalizeHostname(" engine.example.com")).toBe(" engine.example.com");
    expect(normalizeHostname("engine.example.com ")).toBe("engine.example.com ");
  });

  it("does not reject anything — it has no opinion about validity", () => {
    // Not an accident of the implementation: `hostArgs.ts` validates, and a normaliser that also refused would
    // be a second opinion about what is acceptable, which is the copy `lib/net.ts`'s header warns against.
    for (const junk of ["", ".", "http://x/", "a..b", "-x-"]) {
      expect(() => normalizeHostname(junk)).not.toThrow();
    }
  });
});

/**
 * The predicate that replaces the token for loopback use.
 *
 * It is tested **directly** rather than only through `createWsServer`, for the reason this file's header gives
 * about `isLoopbackHost`: once a guard is the whole access-control story for a listener, every assertion about
 * it routed through a socket is really an assertion about the socket layer, and the table in its docblock
 * becomes something no test can enumerate. The six cases below are that table, one line each.
 */
describe("isAllowedOrigin", () => {
  const DOMAIN = DEFAULT_HOST_DOMAIN;

  it("allows an absent Origin — that is 'not a browser', not 'unknown'", () => {
    // The load-bearing distinction. A browser always sends `Origin` on a WebSocket handshake, so its absence
    // means the CLI, `curl`, the extension or a test. Treating absent as suspicious would break all of them,
    // and treating it as allowed is what makes the guard usable at all.
    expect(isAllowedOrigin(undefined, DOMAIN)).toBe(true);
    expect(isAllowedOrigin("", DOMAIN)).toBe(true);
  });

  it("allows every loopback spelling, on any port and either scheme", () => {
    // Port-agnostic and scheme-agnostic on purpose: the GUI is served from `:80` through the edge, from
    // `:4748` directly and from vite's `:5199` in development, and the scheme is not the security property —
    // the *host* is.
    for (const origin of [
      "http://localhost",
      "http://localhost:5199",
      "http://127.0.0.1:4748",
      "http://[::1]:8080",
      "https://localhost",
      "HTTP://LOCALHOST",
    ]) {
      expect(isAllowedOrigin(origin, DOMAIN), origin).toBe(true);
    }
  });

  it("allows any *.localhost subdomain, which is where the GUI's mapping lives", () => {
    // `bifurc.localhost` is a reverse-proxy mapping, and browsers resolve `*.localhost` to loopback by
    // specification — so a subdomain cannot be a remote page however it is spelled.
    for (const origin of ["http://bifurc.localhost", "http://app.localhost:8080", "http://a.b.localhost"]) {
      expect(isAllowedOrigin(origin, DOMAIN), origin).toBe(true);
    }
  });

  it("REFUSES a foreign origin — the case the whole function exists for", () => {
    for (const origin of ["http://evil.example.com", "https://evil.com", "http://192.168.1.50:8080"]) {
      expect(isAllowedOrigin(origin, DOMAIN), origin).toBe(false);
    }
  });

  it("REFUSES `Origin: null` and any other unparseable value", () => {
    // `null` is what a `file://` page and a sandboxed iframe send. Reading "unparseable" as "not a browser"
    // would let a downloaded HTML file drive the engine, which is the same hole with a shorter URL.
    for (const origin of ["null", "file:///tmp/x.html", "not a url", "://"]) {
      expect(isAllowedOrigin(origin, DOMAIN), origin).toBe(false);
    }
  });

  it("REFUSES a lookalike suffix, because the check is on the label boundary", () => {
    // `notlocalhost` is a different host. This pins the difference between `endsWith(".localhost")` and the
    // `includes("localhost")` that would accept `http://localhost.evil.com`.
    for (const origin of ["http://notlocalhost", "http://evil-localhost", "http://localhost.evil.com"]) {
      expect(isAllowedOrigin(origin, DOMAIN), origin).toBe(false);
    }
  });

  it("reads the host domain it is given rather than a second `localhost`", () => {
    // The parameter exists so that "the name this engine answers as" and "the origin it accepts" cannot drift.
    // With a different domain the loopback half still holds and the subdomain half follows the argument.
    expect(isAllowedOrigin("http://bifurc.localhost", "example.com")).toBe(false);
    expect(isAllowedOrigin("http://bifurc.example.com", "example.com")).toBe(true);
    expect(isAllowedOrigin("http://127.0.0.1", "example.com")).toBe(true);
  });
});
