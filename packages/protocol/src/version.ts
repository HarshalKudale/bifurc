/**
 * P1 work item 7 — version handshake.
 *
 * Under D9 (lockstep versioning) `protocolVersion` and `engineVersion` move together for the
 * Electron shell, the web UI, and the CLI. They are kept as SEPARATE fields anyway (per the
 * plan: "you will want to decouple them later and retrofitting a version field is painful"),
 * because the companion extension is an external client that trails by one minor version —
 * see "Protocol compatibility" in `plan/README.md`.
 */

/** Bumped on any wire-incompatible change. See `protocol-changes.md` for the change log. */
export const PROTOCOL_VERSION = "1.0.0";

/** A parsed `major.minor.patch` protocol version. */
export interface ParsedVersion {
  major: number;
  minor: number;
  patch: number;
}

export function parseVersion(version: string): ParsedVersion {
  const [major, minor, patch] = version.split(".").map((n) => parseInt(n, 10) || 0);
  return { major, minor, patch };
}

export type VersionCompatibility = "exact" | "compatible" | "incompatible";

/**
 * Major mismatch -> refuse to connect. Minor mismatch -> connect, degrade (work item 7).
 * A client with an older protocolVersion than the engine is the *expected* state for weeks
 * after every release (the companion extension case) — never refuse on minor alone.
 */
export function checkVersionCompatibility(clientVersion: string, engineVersion: string): VersionCompatibility {
  const client = parseVersion(clientVersion);
  const engine = parseVersion(engineVersion);
  if (client.major !== engine.major) return "incompatible";
  if (client.minor !== engine.minor) return "compatible"; // degrade, do not refuse
  return "exact";
}
