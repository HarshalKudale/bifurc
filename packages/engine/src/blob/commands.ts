/**
 * P3 work item 1 — the four `blob.*` commands, bound to the store.
 *
 * The store itself is transport-agnostic and knows nothing about the protocol's command names.
 * This module is the single place that connects them, so the wiring is auditable: one file maps
 * one command to one store call, and nothing else in the engine needs to know the store exists.
 *
 * ## Why this is a function and not a side effect
 *
 * `createEngine()` registers **no** commands, and that is a documented, tested invariant (see the
 * `registry` assertion in `tests/integration/engineSmoke.integration.test.ts` and the note in
 * `packages/engine/src/commands/registry.ts`). The consumer registers what it is willing to serve:
 * the Electron shell today, the RPC transport at P4–P6, a CLI or the Docker image later. A module
 * that registered itself on import would break that boundary and make `createEngine()` silently
 * serve a surface nobody asked for.
 *
 * The shell's call site lands with P3 work items 3–4, when the `importExport:*` channels are
 * rewired onto `export.create` / `import.preflight` / `import.commit` and become the first real
 * consumers of a staged blob.
 */
import type { BlobPutParams, BlobReadParams, BlobReleaseParams, BlobStatParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";
import { putBlob, readBlob, releaseBlob, statBlob } from "./store";

/**
 * Register `blob.put` / `blob.stat` / `blob.read` / `blob.release`.
 *
 * Throws if any of them is already registered — `CommandRegistry.register()` is strict by design,
 * and a double registration means two different implementations are competing for one wire name.
 */
export function registerBlobCommands(registry: CommandRegistry): void {
  registry.register("blob.put", (params: BlobPutParams) => putBlob(params));
  registry.register("blob.stat", (params: BlobStatParams) => statBlob(params.blobId));
  registry.register("blob.read", (params: BlobReadParams) => readBlob(params));
  registry.register("blob.release", (params: BlobReleaseParams) => releaseBlob(params.blobId));
}
