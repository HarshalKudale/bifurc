/**
 * P4 work item 4 — the handshake and authorisation layer.
 *
 * Two pieces, deliberately separate:
 *
 * | Module | Responsibility |
 * |---|---|
 * | `scopes.ts` | which scope each command requires, asserted total against `COMMANDS` at import |
 * | `authenticated.ts` | the `Transport` decorator that runs the `hello` handshake and enforces scopes |
 *
 * They are split because their failure modes are different and are tested separately: a wrong scope is
 * a *policy* bug, and a gate that can be walked around is a *control-flow* bug. One module holding both
 * would make the tests for one look like the tests for the other.
 *
 * ## There were three, and the third was the token
 *
 * `token.ts` — the per-start secret: generation, `0600` persistence, constant-time verification — was
 * deleted on **2026-09-25**. The engine is never deployed remotely, so its RPC surface binds loopback
 * and refuses anything else, and a credential guarding a boundary that does not exist is a credential
 * that only produces failure modes. See `transport/ws.ts`'s header for the bind rule and
 * `lib/net.ts#isAllowedOrigin` for the check that took its place.
 *
 * ## The directory name is now historical
 *
 * Nothing here *authenticates* any more: `scopes.ts` is authorisation, and `authenticated.ts` is the
 * handshake plus that authorisation. The names are kept because renaming `auth/` and
 * `createAuthenticatedTransport` touches every transport, the host and six test files, and mixing a
 * mechanical rename into a behavioural change makes the behavioural change unreviewable. Recorded as
 * debt rather than left to look intentional.
 */
export {
  assertScopesAreTotal,
  commandIsAllowed,
  COMPANION_SCOPES,
  READ_ONLY_SCOPES,
  requiredScopeFor,
  SCOPE_BY_COMMAND,
  SCOPES,
  SHELL_SCOPES,
  type Scope,
} from "./scopes";
export {
  createAuthenticatedTransport,
  HELLO_ACTION,
  IDENTITY_FAILURE_CODES,
  type AuthenticatedTransportOptions,
} from "./authenticated";
