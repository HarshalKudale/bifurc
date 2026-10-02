/**
 * The six CRUD kinds, registered directly — **no channels**.
 *
 * ## It lives in the engine now (2026-09-23, `plan/15` work item 10)
 *
 * Moved from `src/ipc/handlers/crudKinds.ts` to `packages/engine/src/~~store/kinds/~~crudKinds.ts` — the `store/` hop
 * was transitional and is now `workspace/entities/kinds/` (engine-layout phases 3+4, 2026-09-29) — with its
 * four siblings. **Nothing in it was ever Electron** — two imports, both of engine types — and that is the
 * whole point: this file is the *engine half* of the CRUD factory's inversion, so leaving it in the shell
 * meant a headless host registered the four `entity.*` commands and left this file's container **empty**.
 * Every kind then threw `is not routed through the CommandRegistry yet` while `isRegistered("entity.delete")`
 * was `true`. `registerAllEngineKinds()` calls it now; the shell still calls it too, for the reasons its own
 * call site records.
 *
 * ⚠ **The path above is the 2026-09-23 destination, kept as the historical record.** The file is now at
 * `packages/engine/src/workspace/entities/kinds/crudKinds.ts` (`Engine_Module_Plan.md` phase 3 split
 * `store/`); `store/kinds/coreKinds.ts` and `crudKinds.ts` moved together, and their three protocol
 * siblings stay behind until phase 5.
 *
 * Renamed from `crudHandlers.ts` on 2026-09-21, when the P6 reduction's fourth group deleted the file's
 * last five channels (`webhook:registerActive`, `webhook:unregisterActive`,
 * `webhookServer:start|stop|status`). `plan/07` §5b-septies had recorded a decision to *keep* the
 * `*Handlers` name, on the grounds that the file registered "six kinds **and** five live `webhook*`
 * channels", so "handlers" was still half-true. **That premise has expired, so the exception expires with
 * it**, and this file joins the three siblings stage 3b renamed for the same reason — `soapKinds.ts`,
 * `graphqlKinds.ts`, `grpcKinds.ts`.
 *
 * What remains is the engine half of the CRUD factory's inversion (P6 stage 3b): `registerCrudKind()` is
 * the engine's per-kind options registry, and calling it from the shell is how the four collapsed
 * `entity.*` commands learn a kind's `configKey`, folder key, validation and conflict rule. The channels
 * that used to sit beside these calls were one-line `commandRegistry.invoke()` adapters, and they are
 * gone — the client reaches the commands directly.
 *
 * The five webhook channels' behaviour is unchanged and is covered at the **command** level
 * (`tests/ipc/handlers.test.ts`'s "slice 3" block asserts the port-resolution chain — explicit `port` →
 * `cfg.webhookPort` → `9101` — and the three-field status shape), which is why deleting them cost no
 * tests. `registerWebhookCommands()` is called from `src/ipc/handlers.ts`.
 */
import { EngineError } from "@bifurc/protocol";
import { registerCrudKind } from "../entityCrudCommands";
import { LocalMapping, ProxyRule, MockRule, SavedRequest, SavedWsConnection } from "../../config";

export function registerCrudKinds() {
  // ── Mappings ───────────────────────────────────────────────────────────────

  registerCrudKind<LocalMapping>({
    kind: "mappings",
    configKey: "mappings",
    isFlat: true,
    hasEnabledState: true,
    // `name` is the card's label, `url` is the `.localhost` domain. The pending-deletion record
    // needs both to rebuild a readable stub in `loadConfig()` after the file is gone — without it a
    // deleted published mapping simply disappeared from the UI with nothing left to recover or
    // commit (see the delete site in `entityCrudCommands.ts`).
    getNameEntry: (m) => ({ name: m.label || m.domain, url: m.domain }),
    validate: validateMapping,
  });

  // ── Proxy Rules ────────────────────────────────────────────────────────────

  registerCrudKind<ProxyRule>({
    kind: "rules",
    configKey: "proxyRules",
    folderConfigKey: "ruleFolders",
    hasEnabledState: true,
    onAddConflict: (cfg, target) => {
      if (!target.enabled) return;
      const sig = `${target.useRegex ? "re" : "exact"}|${target.pattern}`;
      for (const r of cfg.proxyRules) {
        if (r.id !== target.id && r.enabled && `${r.useRegex ? "re" : "exact"}|${r.pattern}` === sig) {
          r.enabled = false;
        }
      }
    },
    getNameEntry: (r) => ({ name: r.name, url: r.pattern }),
  });

  // ── Mocks ──────────────────────────────────────────────────────────────────

  registerCrudKind<MockRule>({
    kind: "mocks",
    configKey: "mocks",
    folderConfigKey: "mockFolders",
    hasEnabledState: true,
    validate: (mock) => {
      if (!mock.urlPattern || !mock.urlPattern.trim()) throw new Error("urlPattern is required for mocks");
      if (!mock.method || !mock.method.trim()) throw new Error("method is required for mocks");
    },
    onAddConflict: (cfg, target) => {
      if (!target.enabled) return;
      const sig = `${target.method.toUpperCase()}|${target.urlPattern}|${target.capturedBody ?? ""}`;
      for (const m of cfg.mocks) {
        if (m.id !== target.id && m.enabled && `${m.method.toUpperCase()}|${m.urlPattern}|${m.capturedBody ?? ""}` === sig) {
          m.enabled = false;
        }
      }
    },
    getNameEntry: (m) => ({ name: m.name, method: m.method, url: m.urlPattern }),
  });

  // ── Saved Requests ─────────────────────────────────────────────────────────

  registerCrudKind<SavedRequest>({
    kind: "requests",
    configKey: "requests",
    folderConfigKey: "requestFolders",
    validate: (req) => {
      if (!req.url || !req.url.trim()) throw new Error("url is required for requests");
      if (!req.method || !req.method.trim()) throw new Error("method is required for requests");
    },
    getNameEntry: (req) => ({ name: req.name, method: req.method, url: req.url }),
  });

  // ── WebSocket Connections ──────────────────────────────────────────────────

  registerCrudKind<SavedWsConnection>({
    kind: "sockets",
    configKey: "wsConnections",
    folderConfigKey: "wsFolders",
    getNameEntry: (conn) => ({ name: conn.name, url: conn.url }),
  });

}

/** A DNS label: letters, digits and inner hyphens. */
const DNS_LABEL = /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/i;

function mappingError(field: "domain" | "target", message: string): EngineError {
  return new EngineError("BAD_REQUEST", message, { field });
}

/**
 * The rules a mapping must satisfy, whichever client sends it. The edge routes by `domain` (RFC 6761's
 * `.localhost`) and dials `target` as `host:port` or an `http(s)://` URL — `proxy/response.ts` accepts both.
 * Throws `BAD_REQUEST` with `details.field` naming the offending field, so a form can place the message.
 */
export function validateMapping(m: LocalMapping, siblings: readonly LocalMapping[]): void {
  const domain = (m.domain ?? "").trim().toLowerCase();
  if (domain === "" || domain === ".localhost") throw mappingError("domain", "A domain is required.");
  if (!domain.endsWith(".localhost")) {
    throw mappingError("domain", `"${m.domain}" is not a .localhost domain.`);
  }
  if (!domain.slice(0, -".localhost".length).split(".").every((l) => DNS_LABEL.test(l))) {
    throw mappingError("domain", `"${m.domain}" is not a valid domain: use letters, digits and hyphens.`);
  }
  if (siblings.some((o) => (o.domain ?? "").trim().toLowerCase() === domain)) {
    throw mappingError("domain", `${domain} is already mapped.`);
  }
  if ((m.target ?? "").trim() === "") throw mappingError("target", "A target is required.");
  if (!isDialableTarget((m.target ?? "").trim())) {
    throw mappingError("target", `"${m.target}" is not a target: use host:port or an http(s):// URL.`);
  }
}

function isDialableTarget(target: string): boolean {
  if (/^https?:\/\//i.test(target)) {
    try { return new URL(target).hostname !== ""; } catch { return false; }
  }
  const match = /^([a-z0-9.-]+):(\d{1,5})$/i.exec(target);
  return match !== null && Number(match[2]) >= 1 && Number(match[2]) <= 65535;
}
