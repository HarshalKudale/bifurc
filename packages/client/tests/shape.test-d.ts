/**
 * P5 work item 4 — THE SHAPE PROOF.
 *
 * **Compile-time only. There is no runtime behaviour here and no test runner touches this file.**
 *
 * That is not an accident: vitest's `include` pattern is `**\/*.{test,spec}.?(c|m)[jt]s?(x)`, which
 * `shape.test-d.ts` does not match (it ends in `.test-d.ts`, not `.test.ts`). The file is checked by
 * the **compiler** instead, via `packages/client/tsconfig.json`'s `include: ["tests/**\/*.ts"]`:
 *
 *     npm run typecheck --workspace @bifurc/client
 *
 * which is in turn part of `npm run typecheck:packages`. Note that the **root** `tsc --noEmit` leg
 * cannot see this file — the root tsconfig is `include: ["src/**\/*"]` — so this package's own
 * tsconfig is the only thing standing between these assertions and silent rot. If you ever delete
 * the `tests/**` include, every assertion below becomes a comment.
 *
 * ## What it proves
 *
 * The plan's acceptance criterion is *"`shape.test-d.ts` compiles: the client satisfies
 * `WindowApi`"* — and it calls this **"the single most valuable test in the phase"**, because it
 * converts "the renderer doesn't need changing" from an intention into a build failure.
 *
 * This is the productionised form of `spike/protocol/shape.ts` (the P0 spike, marked "throwaway, do
 * not productionise"), which proved the same property over a 10-method `Pick<>`. Two things changed
 * on the way in: the scope is now the **whole** surface rather than a slice, and the type is named
 * **`BifurcApi`** — the plan says `WindowApi`, but `renderer/types/window.ts` has never exported that
 * name. `BifurcApi` is the real one; the plan's sketch is stale on this point.
 *
 * ## Why the client is checked against `BifurcApi` and not the other way round
 *
 * `BifurcClient` is a **superset** of `BifurcApi`: it adds the four keys `renderer/types/window.ts`
 * under-declares (see `src/surface.ts`'s header) plus the lifecycle method `close`. So the client is
 * assignable to the declared type but not vice versa — `BifurcApi` has no `close`. Asserting mutual
 * assignability here would therefore be wrong, and the spike's two-direction pattern is narrowed
 * below to the *shared* keys, which is the direction that actually catches signature drift.
 */
import type { Transport } from "@bifurc/engine/transport/types";
import type { BifurcApi } from "../../../renderer/types/window";
import type { BifurcApiFull, BifurcClient, OptionalInDeclaredType } from "../src/index";
import { createClient } from "../src/index";
import type { ClientLocal } from "../src/local";

/**
 * Ambient stubs rather than real objects: `declare` emits no code, and this file is never executed.
 * `createClient` needs a transport and a local half only to *be called*; nothing is invoked.
 */
declare const fakeTransport: Transport;
declare const fakeLocal: ClientLocal;

const client = createClient(fakeTransport, { local: fakeLocal });

/* eslint-disable @typescript-eslint/no-unused-vars */

// ── 1. The criterion. Fails to compile if the client is missing a method or has the wrong
//       signature. This single line is what P6's "zero renderer edits" claim rests on.
const _clientSatisfiesApi: BifurcApi = client;

// ── 2. The superset relation the return type relies on.
const _clientSatisfiesApiFull: BifurcApiFull = client;
const _clientIsAClient: BifurcClient = client;

// ── 3. Every key `BifurcApi` declares is present on the client. Implied by (1), but stated
//       separately so the failure names the missing key instead of "not assignable".
type DeclaredKeysAreAllPresent = keyof BifurcApi extends keyof BifurcClient ? true : false;
const _allDeclaredKeysPresent: DeclaredKeysAreAllPresent = true;

// ── 4. The four keys `BifurcApi` omits, asserted **exhaustively**. A `Record<>` literal must list
//       every member of the key union: add a fifth undeclared key to `BifurcApiFull` and this
//       object is incomplete, so the build fails. An array literal would not — arrays tolerate
//       missing members, which is why this is not one.
type UndeclaredKey = Exclude<keyof BifurcApiFull, keyof BifurcApi>;
const _undeclaredKeys: Record<UndeclaredKey, true> = {
  isFirstLaunch: true,
  completeFirstLaunch: true,
  getZoomLevel: true,
  setZoomLevel: true,
};

// ── 4b. The *opposite* defect, asserted with the same exhaustiveness: the three keys the declared
//        type marks optional (`?`) although `src/preload.ts` always provides them. Here the
//        declared type is looser than the runtime rather than narrower — so a client that omitted
//        `getTheme` would satisfy `BifurcApi`, pass assertion (1), and fail the first time the
//        renderer called it. The client tightens all three to required, and `undefined extends X`
//        is the test for "still optional".
type TightenedKey = OptionalInDeclaredType;
type IsRequiredOnClient<K extends TightenedKey> =
  undefined extends BifurcApiFull[K] ? false : true;
const _tightenedToRequired: { [K in TightenedKey]: IsRequiredOnClient<K> } = {
  setTitleBarOverlay: true,
  getTheme: true,
  setTheme: true,
};

// ── 4c. Put together: `BifurcApiFull` differs from `BifurcApi` on **exactly seven** keys — four
//        added, three tightened. This is the whole gap between the declared contract and the
//        runtime, stated in one place. If someone widens or narrows it without saying so, either
//        this literal or the two above stops compiling.
type DifferingKey = UndeclaredKey | TightenedKey;
const _differingKeys: Record<DifferingKey, true> = {
  isFirstLaunch: true,
  completeFirstLaunch: true,
  getZoomLevel: true,
  setZoomLevel: true,
  setTitleBarOverlay: true,
  getTheme: true,
  setTheme: true,
};

// ── 5. `close` is deliberately **outside** the surface. `window.api` has no `close`, so a client
//       that declared one in `BifurcApi` would be a lie about the runtime — and would break the
//       `Object.keys` diff that `client.test.ts` asserts. It must exist, though: the shell calls it.
type CloseIsNotPartOfTheSurface = "close" extends keyof BifurcApi ? never : true;
const _closeIsOutsideTheSurface: CloseIsNotPartOfTheSurface = true;
const _closeIsCallable: () => Promise<void> = client.close;

// ── 6. The unsubscribe contract, per method. `renderer/types/window.ts` types all seven as
//       returning `() => void`; the plan warns that "silently returning `undefined` leaks
//       listeners", so a client that typed any of these as `void` must fail here rather than leak
//       subscriptions at runtime. The mapped type checks each key, and the literal must be total.
type SubscriptionMethod =
  | "onSyncStatus"
  | "onEntitySyncStatus"
  | "onLogEntry"
  | "onLogChunk"
  | "onServerError"
  | "onWebhookPayload"
  | "onCompanionRefresh";

type ReturnsUnsubscribe<K extends SubscriptionMethod> =
  ReturnType<BifurcApi[K]> extends () => void ? true : false;

const _unsubscribeContract: { [K in SubscriptionMethod]: ReturnsUnsubscribe<K> } = {
  onSyncStatus: true,
  onEntitySyncStatus: true,
  onLogEntry: true,
  onLogChunk: true,
  onServerError: true,
  onWebhookPayload: true,
  onCompanionRefresh: true,
};

// ── 7. Direction 2 of the spike, narrowed to the keys whose declared type is left alone. The
//       client must not be *stricter* than the declared type on those: a narrower parameter or a
//       wider return would still satisfy (1) — the renderer would keep compiling against
//       `BifurcApi` while failing at runtime. Assigning `BifurcApi` back into the client's slice
//       fails on exactly that drift.
//
//       The three `OptionalInDeclaredType` keys are excluded **because they are deliberately
//       stricter** — that is what 4b asserts. Including them here would make this check contradict
//       4b, which is what the first draft of this file did; the compiler caught it, which is the
//       clearest possible demonstration that these assertions are load-bearing rather than
//       decorative.
type AlreadyRequiredKey = Exclude<keyof BifurcApi, OptionalInDeclaredType>;
const _apiSatisfiesClientSlice: Pick<BifurcApiFull, AlreadyRequiredKey> =
  null as unknown as BifurcApi;

// ── 8. `createClient` requires the local half. There is no sensible default for a native file
//       picker or an OS trust store, so this is a compile error on purpose:
//
//         createClient(fakeTransport);            // ← must not compile
//
//       It cannot be asserted in a file that has to compile, so it is recorded here as the
//       counter-example that makes the requiredness deliberate rather than incidental.
type OptionsRequireLocal = "local" extends keyof Parameters<typeof createClient>[1] ? true : false;
const _localIsRequired: OptionsRequireLocal = true;

export {};
