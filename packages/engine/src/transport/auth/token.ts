/**
 * P4 work item 4 — the engine's per-start session secret.
 *
 * `plan/05`: "Token generation: `crypto.randomBytes(32).toString("base64url")`, rotated per engine
 * start, written `0600` to `<dataDir>/engine.token`. Docker reads it from an env var or a mounted
 * secret instead."
 *
 * ## Rotated per start, not reused
 *
 * `ensureEngineToken()` **generates a new token on every call** rather than reusing a file left by a
 * previous run. A token that survives a restart is a credential that survives a restart: it would
 * still be valid against a freshly-started engine, so anything that ever read it — a stale shell, a
 * log, a backup of the data dir — stays authorised forever. Rotation is what makes the token a
 * *session* secret rather than a password.
 *
 * ## Why the comparison is not `===`
 *
 * `verifyToken()` uses `crypto.timingSafeEqual`. A `===` on strings short-circuits at the first
 * differing byte, so the time it takes to reject reveals how many leading bytes were correct — which
 * turns a 2^256 search into a byte-at-a-time one. Over loopback that is theoretical; the whole point
 * of this module is the remote case, where it is not.
 *
 * `timingSafeEqual` **throws** on length mismatch, so the lengths are compared first. That leaks the
 * length, which for a fixed 32-byte token is public anyway — the secret is the contents. The
 * alternative (padding to a fixed length) would be more code guarding nothing.
 *
 * ## The `0600` limitation, stated rather than assumed
 *
 * `writeFile` only applies `mode` when it **creates** the file, so the mode is passed at creation and
 * `chmod` is applied afterwards for the overwrite case. On **Windows neither has any effect** — there
 * are no POSIX permission bits, and Node maps `mode` to the read-only attribute only. So on Windows
 * the token is protected by the data directory's ACL and nothing else. That is a real gap and it is
 * the same class of gap `plan/05` flags for the named-pipe transport, so it belongs in P9/P12 rather
 * than being papered over here with a `chmod` that silently does nothing.
 */
import { randomBytes, timingSafeEqual } from "node:crypto";
import { chmod, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

/** `<dataDir>/engine.token`. */
export const ENGINE_TOKEN_FILENAME = "engine.token";

/**
 * Set this to supply the token out of band instead of letting the engine generate one — the Docker
 * and mounted-secret path. When it is set the file is **not** written: a container that mounts its
 * secret should not also have it written into a volume that might be more widely readable.
 */
export const ENGINE_TOKEN_ENV_VAR = "BIFURC_ENGINE_TOKEN";

/**
 * 32 bytes = 256 bits. `base64url` so the value survives a URL, a JSON string, an env var and a file
 * without escaping — every one of which it has to pass through.
 */
const TOKEN_BYTES = 32;

/** File mode for the token file: owner read/write, nobody else. */
const TOKEN_FILE_MODE = 0o600;

export function generateToken(): string {
  return randomBytes(TOKEN_BYTES).toString("base64url");
}

export function tokenPath(dataDir: string): string {
  return join(dataDir, ENGINE_TOKEN_FILENAME);
}

export interface EngineToken {
  readonly token: string;
  /**
   * Where the token lives, or `undefined` when it came from the environment — in which case nothing
   * was written and there is no path to hand to a client.
   */
  readonly path?: string;
  readonly source: "generated" | "environment";
}

/**
 * Produce this run's token: from the environment if `ENGINE_TOKEN_ENV_VAR` is set, otherwise freshly
 * generated and written `0600` into `dataDir`.
 *
 * Assumes `dataDir` already exists — `initWorkspaceDir()` owns creating it, and creating directories
 * from here would make this module responsible for a directory it does not own.
 */
export async function ensureEngineToken(
  dataDir: string,
  env: NodeJS.ProcessEnv = process.env,
): Promise<EngineToken> {
  const fromEnv = env[ENGINE_TOKEN_ENV_VAR];
  if (typeof fromEnv === "string" && fromEnv.length > 0) {
    return { token: fromEnv, source: "environment" };
  }

  const token = generateToken();
  const path = tokenPath(dataDir);
  await writeFile(path, token, { encoding: "utf8", mode: TOKEN_FILE_MODE });
  // `mode` is honoured only when `writeFile` *creates* the file. A token left over from a previous
  // run was already created, possibly with a looser mode, so the mode is applied again explicitly.
  await chmod(path, TOKEN_FILE_MODE);

  return { token, path, source: "generated" };
}

/**
 * Read the token a previous `ensureEngineToken()` wrote. Returns `undefined` when there is no usable
 * file — which is the normal state for a client that should be reading the environment instead.
 *
 * An empty or whitespace-only file counts as absent rather than as a token, because a truncated write
 * would otherwise become a credential that is trivially guessable.
 */
export async function readEngineToken(dataDir: string): Promise<string | undefined> {
  try {
    const raw = await readFile(tokenPath(dataDir), "utf8");
    const trimmed = raw.trim();
    return trimmed.length > 0 ? trimmed : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Does `presented` match `expected`?
 *
 * `presented` is `unknown` because it arrives from a client and has not been validated yet — a
 * `hello` whose `token` is a number, an object or absent must be rejected here rather than throwing a
 * `TypeError` out of a comparison and crashing the session.
 *
 * Both arguments are compared as UTF-8 **bytes**, so the comparison is over the secret itself rather
 * than over a JS string's representation of it.
 */
export function verifyToken(expected: string, presented: unknown): boolean {
  if (typeof presented !== "string") return false;

  const a = Buffer.from(expected, "utf8");
  const b = Buffer.from(presented, "utf8");
  if (a.length !== b.length) return false;

  return timingSafeEqual(a, b);
}
