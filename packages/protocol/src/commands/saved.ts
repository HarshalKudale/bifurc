/**
 * Executing a **saved request entity by id** — and, since 2026-09-30, the **REST pipeline**.
 *
 * ## The problem this closes, stated as the CLI sees it
 *
 * Every request-executing command in the protocol takes the **whole request**: `request.replay` wants
 * `{method, url, headers, body}`, `graphql.execute` wants the endpoint, the query and the variables, and so
 * on. That is the right contract for the *editor* — a tab that has never been saved, or that has unsaved
 * edits, has nothing else to send. It is the wrong contract for any consumer that is **not** an editor.
 *
 * A CLI is the case that makes it concrete. `bifurc requests send <id>` has an id and nothing else: it did
 * not load the entity, it has no editor state, and requiring it to fetch the entity and then re-send every
 * field back would mean the *client* owns the stored shape — exactly the coupling the engine exists to
 * remove. A saved request is on disk; the engine can read it.
 *
 * ## `request.send` — the id form and the inline form, one pipeline
 *
 * `request.send` is the group's first member and the only one that has stopped being a thin counterpart.
 * `plan/19` D24 makes `resolve → pre-script → send → post-script → test` **one engine function**, and D27
 * gives it one command per protocol whose payload is a **union of two complete input forms**:
 *
 * | form | payload | source of the request |
 * |---|---|---|
 * | id | `{ workspaceId, id }` | the stored entity |
 * | inline | `{ workspaceId, request, scripts }` | the caller |
 *
 * The two forms differ **only** in where the request comes from — that is D24's whole claim, and it is why
 * they share a name rather than getting one each. The inline form is what a draft tab sends; the id form is
 * what a saved tab and the CLI send.
 *
 * **`body` is text in both forms.** The stored `SavedRequest.body` is text and all five importers store
 * text, so the pipeline's input is text; `request.replay` keeps base64 because binary safety is a wire
 * concern and it predates this. See `plan/protocol-changes.md`'s 2026-09-30 entry for the full argument.
 *
 * ## Four commands, three different shapes, and the differences are decisions
 *
 * | command | counterpart of | entity kind | what it does |
 * |---|---|---|---|
 * | `request.send` | `request.replay` | `requests` | the **full pipeline** — resolve + scripts |
 * | `graphql.send` | `graphql.execute` | `graphqlRequests` | **resolves every field**; runs no scripts |
 * | `soap.send` | `soap.execute` | `soapRequests` | **resolves every field**; runs no scripts |
 * | `grpc.executeSaved` | `grpc.execute` | `grpcRequests` | load + dispatch |
 *
 * `plan/19` §8 trap 1 is *"one commit per protocol: the engine starts and the renderer stops"*, so the one
 * that remains is migrated on its own turn and keeps the 2026-09-29 contract until then: load the entity,
 * dispatch to the command that executes it, and nothing else. **It still does not resolve `{{variables}}`** —
 * that is the renderer's half for that tab, and its `handleSend` still does it.
 *
 * **Why `graphql.send` and `soap.send` resolve but run no scripts.** The script sandbox's context is a
 * `{method, url, headers, body}` request (`mocks/scriptExecutor.ts`'s `IpcScriptOpts`), and neither a GraphQL
 * nor a SOAP request is one — so neither has a `scripts` member at all, rather than one nothing could run.
 * The `graphql.send` entry in `plan/protocol-changes.md` also records the latent under-resolution these
 * migrations fix: each tab's Send gate scanned more fields than its `handleSend` resolved, so a `{{token}}` in
 * a GraphQL query — or a SOAP `soapAction` — never resolved in **either** path. The engine resolves every
 * field the gate scans, which is the only form under which `usesVariables` can leave the gate.
 *
 * ## Why the four live in one file rather than one per namespace
 *
 * Every other file in this directory is a namespace (`graphql.ts`, `soap.ts`, …). This one is a
 * **capability**, and the grouping is not an accident of the wire names — it is the reason the group exists.
 * All four serve the same consumer, and the two that remain unmigrated still share the *same* params contract
 * (`SavedEntityRef`) and the same semantics. Splitting them across four namespace files would put four copies
 * of one schema in four places, which is the "a count or a shape that exists in two places" hazard
 * `scope.ts`'s header spells out — and it would hide the fact that these are one thing.
 *
 * The namespace is still readable: `saved.ts` is where `request.send` and `graphql.send` live and `misc.ts`
 * is where `request.replay` lives, and each file's header says why.
 *
 * ## Why `workspaceId` is required, when `WorkspaceScope` makes it optional elsewhere
 *
 * The precedent is `entity.load` — the command the id form composes with — whose params are
 * `{workspaceId, kind, id}` with `workspaceId` **required** (`entity.ts`). "Read one named entity" takes an
 * explicit workspace, because an entity id is only unique *within* a workspace and the silent fallback to
 * "whatever is active" is a stateful guess. A host serves N workspaces behind one endpoint, and a CLI
 * attaches to a host, so both are precisely the callers for whom the guess is least defensible.
 * `entity.list` makes the same choice for the same reason.
 *
 * `WorkspaceScope`'s optional field exists for the desktop, which addresses "whatever the user has
 * selected" and therefore has no id to send (D13). Nothing here is a desktop caller, so the optional form
 * buys nothing and costs explicitness.
 *
 * ## The four that were considered and left out
 *
 * A counterpart is only meaningful when the command's **entire** input is derivable from a saved entity.
 * Four near-misses fail that test, and they are listed so the omission reads as a decision:
 *
 * - `graphql.introspect` and `soap.fetchWsdl` take a bare `url`, and the GUI drives both from a
 *   **typed-in** address (`SchemaExplorer.tsx`, `WsdlExplorer.tsx`) — a saved request's endpoint is a
 *   coincidence, not the source. `wsdls` stores the fetched document, not a target.
 * - `grpc.reflect` likewise takes a bare `serverAddress` from a typed-in field (`ProtoExplorer.tsx`).
 * - `script.execute` takes a script *plus the response context* it runs against; a script is a field of a
 *   request entity rather than an entity, so there is nothing to load by id. **It stays a command** —
 *   `plan/19` §8 trap 7 — and the pipeline calls the same sandbox internally.
 * - `healthbar.checkUrl` takes a url from `healthbar.saveServices`' per-workspace list, which is not a CRUD
 *   entity kind and has no id to name.
 *
 * ## Frozen-protocol note
 *
 * The 2026-09-29 names were **additive**. `request.send` **replaces** one of them, which `## Guidance`
 * calls breaking — and it is not a `PROTOCOL_VERSION` bump because that name was never released: this
 * source is the only place it has ever existed and the command count is unchanged at 93. The reasoning is
 * in `plan/protocol-changes.md`'s 2026-09-30 entry, which is the entry to read before touching this file.
 */
import { z } from "zod";
import type { HttpLikeResult } from "./graphql";
import type { SoapExecuteResult } from "./soap";
import type { GrpcExecuteResult } from "./grpc";

/** Declared locally, matching `misc.ts`: a header map is `Record<string, string>` everywhere on the wire. */
const HeaderMap = z.record(z.string(), z.string());

/**
 * The whole input to a thin `*Saved` command: which workspace, and which entity in it.
 *
 * The kind is **not** a field. It is fixed by the command name — `graphql.executeSaved` can only ever load
 * a `graphqlRequests` entity — which is the difference between this and `entity.load`, whose `kind` is a
 * parameter because it is the generic loader. Making it a field here would let a caller ask
 * `graphql.executeSaved` for a SOAP entity and get an answer about neither.
 *
 * `.strict()` on the outermost call, per `scope.ts`'s rule (there is no `merge` here, so the order hazard
 * that note describes does not arise, but the strictness itself is not optional: a payload carrying an
 * extra key is a caller that believes it can pass something it cannot).
 */
export const SavedEntityRef = z.object({
  workspaceId: z.string(),
  id: z.string(),
}).strict();
export type SavedEntityRef = z.infer<typeof SavedEntityRef>;

// ─────────────────────────────────────────────────────────────────────────────
// request.send — the REST pipeline (plan/19 D24/D27)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A request as the **caller** holds it, with every field still unresolved.
 *
 * `body` is **text**, not base64 — see the header. Resolution is the engine's: a `{{host}}` in any of these
 * fields is resolved against the workspace's own environment state before anything is sent (`plan/19`
 * D23), which is why there is no environment field here. A caller that resolved the fields itself would be
 * the second resolver D22's boundary ratchet exists to prevent.
 */
export const RequestSendRequest = z.object({
  method: z.string(),
  url: z.string(),
  headers: HeaderMap,
  body: z.string(),
  bodyEncoding: z.enum(["utf8", "base64"]).optional(),
  bodyParts: z.array(z.object({
    name: z.string(),
    kind: z.enum(["text", "file"]),
    value: z.string(),
    fileName: z.string().optional(),
    contentType: z.string().optional(),
  }).strict()).optional(),
}).strict();
export type RequestSendRequest = z.infer<typeof RequestSendRequest>;

/**
 * The scripts to run around the send. Each is optional; an absent or whitespace-only script is not run.
 *
 * **Required as an object, so that "no scripts" is stated rather than inferred.** A caller with none passes
 * `{}`. The alternative — making the object itself optional — reads as if omitting it might mean something
 * other than "none", and there is nothing else it could mean.
 */
export const RequestSendScripts = z.object({
  pre: z.string().optional(),
  post: z.string().optional(),
  test: z.string().optional(),
}).strict();
export type RequestSendScripts = z.infer<typeof RequestSendScripts>;

/**
 * `request.send` — run the REST pipeline, from a stored entity or from an inline request.
 *
 * A **union of two complete forms**, and `.strict()` on both is what makes that honest: a payload that is
 * partly one form and partly the other (`{workspaceId, id, scripts}`) matches neither and is rejected,
 * rather than being half-read. `plan/19` §5 rejected option C — putting two modes on `request.replay` — on
 * the grounds that a two-mode schema is "a schema nothing can validate honestly"; the distinction is that
 * `request.replay`'s value is being *one thing*, while this command's two forms are each a complete
 * description of a request. See the 2026-09-30 entry in `plan/protocol-changes.md`.
 */
export const RequestSendParams = z.union([
  SavedEntityRef,
  z.object({
    workspaceId: z.string(),
    request: RequestSendRequest,
    scripts: RequestSendScripts,
  }).strict(),
]);
export type RequestSendParams = z.infer<typeof RequestSendParams>;

/**
 * The response **plus** what the pipeline did on the way through.
 *
 * The response fields are `request.replay`'s, unchanged and still base64 — the renderer's `b64ToText` stays
 * and stays presentation. The rest exists because the pipeline is no longer the caller's job: a client that
 * had to run the scripts itself would need the response *before* it could, which is what made the renderer
 * own this in the first place.
 *
 * `request` is the request **as actually sent** — resolved, and rewritten by the pre-script if there was
 * one. A UI that shows a response without showing the request that produced it cannot explain the response,
 * and after engine-side resolution the caller genuinely does not know what was sent.
 */
export interface RequestSendResult {
  status: number;
  headers: Record<string, string>;
  body: string;
  /** How long the send itself took. This is also what the test-script sees as `responseTime`. */
  durationMs: number;
  request: RequestSendRequest;
  tests: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs: string[];
  preScriptError?: string;
  postScriptError?: string;
}

// ─────────────────────────────────────────────────────────────────────────────
// graphql.send — resolves every field; runs no scripts (plan/19 D23/D24/D27)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A GraphQL request as the **caller** holds it, with every field still unresolved.
 *
 * **The stored entity's shape, not `graphql.execute`'s** — `endpointUrl`, not `url`. The inline form carries
 * what the *editor* holds, and the editor holds `endpointUrl`; using the command's name here would make the
 * renderer translate a field for no reason and would stop the id form and the inline form being
 * interchangeable. The rename stays the engine's business, where it already is for the id form.
 */
export const GraphqlSendRequest = z.object({
  endpointUrl: z.string(),
  headers: HeaderMap,
  query: z.string(),
  variables: z.string(),
  operationName: z.string(),
}).strict();
export type GraphqlSendRequest = z.infer<typeof GraphqlSendRequest>;

/**
 * `graphql.send` — run the `graphqlRequests` entity `id`, or an inline request.
 *
 * **No `scripts` member, unlike `request.send`**: the sandbox's context is a `{method, url, headers, body}`
 * request and a GraphQL request is not one, so there is nothing a script could run against. A field nothing
 * can execute would be a lie about what this command does — see the header.
 */
export const GraphqlSendParams = z.union([
  SavedEntityRef,
  z.object({
    workspaceId: z.string(),
    request: GraphqlSendRequest,
  }).strict(),
]);
export type GraphqlSendParams = z.infer<typeof GraphqlSendParams>;
export type GraphqlSendResult = HttpLikeResult;

// ─────────────────────────────────────────────────────────────────────────────
// soap.send — resolves every field; runs no scripts (plan/19 D23/D24/D27)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A SOAP request as the **caller** holds it, with every field still unresolved.
 *
 * **The stored entity's shape — which for SOAP is also `soap.execute`'s.** All four fields
 * (`endpointUrl`, `soapAction`, `headers`, `body`) are already named identically on both sides, so this type
 * exists to say *which* four rather than to rename any of them. That is the one respect in which this
 * protocol is simpler than GraphQL, whose `endpointUrl` → `url` rename the engine has to perform.
 */
export const SoapSendRequest = z.object({
  endpointUrl: z.string(),
  soapAction: z.string(),
  headers: HeaderMap,
  body: z.string(),
}).strict();
export type SoapSendRequest = z.infer<typeof SoapSendRequest>;

/**
 * `soap.send` — run the `soapRequests` entity `id`, or an inline request.
 *
 * **No `scripts` member**, for the same reason as `graphql.send`: the script sandbox's context is a
 * `{method, url, headers, body}` request and a SOAP request is not one, so there is nothing a script could
 * run against.
 */
export const SoapSendParams = z.union([
  SavedEntityRef,
  z.object({
    workspaceId: z.string(),
    request: SoapSendRequest,
  }).strict(),
]);
export type SoapSendParams = z.infer<typeof SoapSendParams>;
export type SoapSendResult = SoapExecuteResult;

// ─────────────────────────────────────────────────────────────────────────────
// grpc.send — resolves every field; runs no scripts (plan/19 D23/D24/D27)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A gRPC call as the **caller** holds it, with every field still unresolved.
 *
 * The stored entity's shape, and `grpc.execute`'s — the two agree field-for-field, so like SOAP there is no
 * rename here. `protoFileId` and `useReflection` are **required rather than optional** because the *editor*
 * always has them: a proto file may be `null` and reflection may be off, but neither is absent, and making
 * them optional would invite a caller to omit a decision rather than make one.
 */
export const GrpcSendRequest = z.object({
  serverAddress: z.string(),
  serviceName: z.string(),
  methodName: z.string(),
  requestBody: z.string(),
  metadata: HeaderMap,
  protoFileId: z.string().nullable(),
  useReflection: z.boolean(),
}).strict();
export type GrpcSendRequest = z.infer<typeof GrpcSendRequest>;

/**
 * `grpc.send` — run the `grpcRequests` entity `id`, or an inline call.
 *
 * **No `scripts` member**, for the reason recorded in the `graphql.send` and `soap.send` entries: the script
 * sandbox's context is a `{method, url, headers, body}` request and a gRPC call is not one.
 *
 * **`grpc.execute` is a `wontfix` stub** (`grpc/commands.ts`), so this delegates to a resolved
 * `{ok:false, error}`. Wired anyway, and for the same reason its counterpart was: the contract is
 * *load-or-inline → resolve → dispatch*, and it starts working the day the stub is implemented.
 */
export const GrpcSendParams = z.union([
  SavedEntityRef,
  z.object({
    workspaceId: z.string(),
    request: GrpcSendRequest,
  }).strict(),
]);
export type GrpcSendParams = z.infer<typeof GrpcSendParams>;
export type GrpcSendResult = GrpcExecuteResult;
