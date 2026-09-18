/**
 * P4 work item 4 — authentication and authorisation.
 *
 * Three pieces, deliberately separate:
 *
 * | Module | Responsibility |
 * |---|---|
 * | `scopes.ts` | which scope each command requires, asserted total against `COMMANDS` at import |
 * | `token.ts` | the per-start secret: generation, `0600` persistence, constant-time verification |
 * | `authenticated.ts` | the `Transport` decorator that enforces both, plus the `hello` handshake |
 *
 * They are split because their failure modes are different and are tested separately: a wrong scope
 * is a *policy* bug, a leaked token is a *secret* bug, and a gate that can be walked around is a
 * *control-flow* bug. One module holding all three would make the tests for one look like the tests
 * for another.
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
  ENGINE_TOKEN_ENV_VAR,
  ENGINE_TOKEN_FILENAME,
  ensureEngineToken,
  generateToken,
  readEngineToken,
  tokenPath,
  verifyToken,
  type EngineToken,
} from "./token";
export {
  createAuthenticatedTransport,
  HELLO_ACTION,
  IDENTITY_FAILURE_CODES,
  type AuthenticatedTransportOptions,
} from "./authenticated";
