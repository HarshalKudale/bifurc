import { describe, it, expect } from "vitest";
import { isBinaryContentType as ENGINE_IS_BINARY } from "@bifurc/engine/utils/contentType";
import { isBinaryContentType as UI_IS_BINARY } from "@/lib/bodyUtils";

/**
 * The binary-content-type predicate exists **twice**, and this file is the reason that is allowed.
 *
 * ## Why there is a copy at all
 *
 * `plan/19` WI 5 item 2 moved capture → entity construction into the engine, and that construction decides
 * a mock's `responseBodyEncoding` — so the engine needs to answer "is this content type binary?"
 * (`packages/engine/src/utils/contentType.ts`). The renderer needs the same answer for a different reason:
 * which viewer to mount, and whether to show a body as text. `@bifurc/gui` has **no** `@bifurc/engine`
 * dependency by design (`plan/19` finding 1), so the copy in `lib/bodyUtils.ts` stays.
 *
 * That is the **D28 list-vs-logic split a third time**, after `RANDOMIZER_TOKENS` and `SKIP_CURL_HEADERS`,
 * and the same consequence follows: *"a copy with nothing checking it is exactly §1 row 2's defect"*. The
 * copy is permitted only alongside this assertion.
 *
 * ## What is pinned, and why it compares behaviour rather than arrays
 *
 * The engine's list is **substring prefixes** (`image/`, `audio/`, …), not exact types, so a set-equality
 * assertion would be comparing an implementation detail. What matters is that the two **agree on inputs**,
 * so the pin runs a sample through both and requires identical answers.
 *
 * **⚠ The failure this catches is asymmetric, which is why the sample includes both directions.** A type the
 * engine calls binary but the renderer does not shows base64 in a text viewer; the reverse **decodes a PNG
 * as UTF-8 and corrupts it on save**. Both are in the sample, so neither can drift unnoticed.
 */

/**
 * A deliberately mixed sample: every prefix the engine's list names, the text types that must stay false,
 * the two boundary cases (`multipart/form-data`, which is *not* in the list despite carrying a file part),
 * and a capitalised one, because both implementations lower-case before comparing.
 */
const SAMPLES = [
  "image/png",
  "image/svg+xml",
  "audio/mpeg",
  "video/mp4",
  "font/woff2",
  "application/octet-stream",
  "application/pdf",
  "application/zip",
  "application/gzip",
  "application/json",
  "text/plain",
  "text/html",
  "application/xml",
  "application/x-www-form-urlencoded",
  "multipart/form-data",
  "IMAGE/PNG",
  "",
];

describe("the renderer's binary-content-type predicate is a pinned copy of the engine's", () => {
  it("agrees with the engine on every sample", () => {
    const disagreements = SAMPLES.filter((ct) => ENGINE_IS_BINARY(ct) !== UI_IS_BINARY(ct));
    expect(
      disagreements,
      `${disagreements.length} content type(s) are classified differently by the engine and the renderer: ` +
        `${disagreements.join(", ")}. The engine's copy is the authority; update the renderer's to match, ` +
        `or change both deliberately. A disagreement in the "engine says binary" direction corrupts a body ` +
        `on save.`,
    ).toEqual([]);
  });

  it("is a real predicate, not a constant", () => {
    // Guards the assertion above from passing vacuously: if either side answered `true` (or `false`) for
    // everything, "they agree" would still hold.
    expect(SAMPLES.some((ct) => ENGINE_IS_BINARY(ct))).toBe(true);
    expect(SAMPLES.some((ct) => !ENGINE_IS_BINARY(ct))).toBe(true);
  });
});
