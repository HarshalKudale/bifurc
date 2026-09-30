import { describe, it, expect } from "vitest";
import {
  SKIP_CURL_HEADERS as ENGINE_SKIP,
  parseCurl,
  parseCurlCommand,
} from "@bifurc/engine/utils/curl";
import { SKIP_CURL_HEADERS as UI_SKIP } from "@/lib/utils";

/**
 * The hop-by-hop header list exists **twice**, and this file is the reason that is allowed.
 *
 * ## Why there is a copy at all
 *
 * `plan/19` WI 5 moved the cURL parser into the engine (`utils/curl.ts`) and deleted the renderer's
 * `lib/curlParser.ts`. The *parser* had exactly one honest home. The **header list** did not, because
 * the renderer still needs it for something the engine cannot do for it: filtering a **stored** mock's
 * `capturedHeaders` when that mock is loaded into a tab (`restTabHelpers.ts#buildMockReqHeaders`). A
 * mock written before this change carries `host` / `content-length` rows on disk, and the filter is what
 * keeps them out of the editor — a *read*-side concern, so it cannot move to the write side.
 *
 * The renderer cannot import the engine — `@bifurc/gui` has **no** `@bifurc/engine` dependency by design
 * (`plan/19` finding 1) — so the list is hand-mirrored into `@/lib/utils`.
 *
 * This is the same **list vs logic** split `plan/19` D28 made for `RANDOMIZER_TOKENS`, and the same
 * reasoning applies word for word: *"a copy with nothing checking it is exactly §1 row 2's defect"*. So
 * the copy is permitted only alongside these assertions — this test is the pin, not an optional extra.
 * The boundary ratchet's `curlParser` construct was narrowed to the **parser** accordingly; flagging the
 * list there would contradict the decision, exactly as flagging `RANDOMIZER_TOKENS` would contradict D28.
 *
 * ## What is pinned
 *
 * 1. **Same members**, in both directions — an extra name on either side is a divergence.
 * 2. **The list is load-bearing, not decorative.** `parseCurlCommand` (the importer's wrapper) must
 *    actually drop these headers, and `parseCurl` must actually **not** — the two contracts are
 *    deliberately different, and a refactor that made the raw parser filter would silently change what
 *    the REST tab imports.
 * 3. **Not empty**, so the assertions above cannot pass by both sides being `undefined`-ish.
 */

describe("the renderer's cURL skip-list is a pinned copy of the engine's", () => {
  it("has the same members, both directions", () => {
    expect([...UI_SKIP].sort()).toEqual([...ENGINE_SKIP].sort());
  });

  it("is not empty, and names no duplicates", () => {
    expect(ENGINE_SKIP.size).toBeGreaterThan(0);
    expect(new Set(UI_SKIP).size).toBe(UI_SKIP.size);
  });

  it("the engine's `parseCurlCommand` drops exactly these headers", () => {
    // The importer's contract. If the list stopped being applied here, this file's whole premise — that
    // the copy mirrors something the engine actually uses — would be false.
    const parsed = parseCurlCommand(
      "curl -X POST 'http://x/api' -H 'Host: x' -H 'Content-Length: 3' -H 'Accept: application/json' -d '{}'",
    );
    expect(parsed).not.toBeNull();
    expect(parsed!.headers).toEqual({ accept: "application/json" });
  });

  it("the engine's raw `parseCurl` does NOT drop them — the caller decides", () => {
    // `request.parseCurl` returns the command faithfully, which is why the REST tab still needs the copy
    // above. A change that made the raw parser filter would make this assertion fail rather than silently
    // changing what the tab imports.
    const parsed = parseCurl("curl 'http://x/api' -H 'Host: x' -H 'Accept: application/json'");
    expect(parsed.headers).toEqual({ host: "x", accept: "application/json" });
  });
});
