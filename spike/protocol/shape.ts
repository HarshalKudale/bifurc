/**
 * Spike 4 — THE SHAPE PROOF.
 *
 * Compile-time-only. No runtime behaviour. If this file type-checks, then for the
 * 10 spike methods the protocol client and the renderer's declared `window.api`
 * surface are **mutually assignable** — the client is a drop-in replacement and
 * the renderer needs ZERO edits.
 *
 * Run it with:  npm run spike:typecheck
 *
 * SPIKE CODE — throwaway, do not productionise.
 */
import type { BifurcApi } from "../../renderer/types/window";
import type { ProtocolClient } from "./client";

/** The 10 window.api method names the spike covers. */
export const SPIKE_METHODS = [
  "getConfig",
  "serverStatus",
  "addMock",
  "deleteMock",
  "addFolder",
  "moveFolder",
  "setEntityEnabled",
  "graphqlIntrospect",
  "soapExecute",
  "tlsGenerate",
] as const;

type ApiSlice = Pick<BifurcApi, (typeof SPIKE_METHODS)[number]>;

/* eslint-disable @typescript-eslint/no-unused-vars */

// Direction 1 — the client can stand in for the api slice.
// A real client instance would be assigned here; the casts keep this file
// importable without a live transport.
const _clientSatisfiesApi: ApiSlice = null as unknown as ProtocolClient;

// Direction 2 — the api slice can stand in for the client (no extra requirements).
const _apiSatisfiesClient: ProtocolClient = null as unknown as ApiSlice;

// Both directions compiling = mutually assignable = drop-in.
export type { ApiSlice };
