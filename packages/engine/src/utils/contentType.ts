/**
 * Is this content type binary? — the engine's answer, moved down from the renderer
 * (`packages/gui/src/lib/bodyUtils.ts`) by `plan/19` WI 5, item 2.
 *
 * ## Why the engine needs it at all
 *
 * Capture → entity construction decides a mock's `responseBodyEncoding`: a binary response is stored as
 * base64 and flagged, a text one is decoded and stored as text. That decision is a fact about the wire
 * format, not about the screen, so it belongs wherever the entity is built — and after this work item that
 * is the engine.
 *
 * ## Why it is a separate module and not a private function
 *
 * The fact is **general**: the renderer asks the same question to choose a viewer, and the mock handler
 * would ask it if it ever needed to re-encode a body. `Engine_Module_Plan.md` §5.1's placement rule puts a
 * stateless shared predicate in `utils/`, and burying it inside `captureEntities.ts` would make the next
 * caller import a capture module to ask a question about MIME types.
 *
 * ## ⚠ The renderer keeps a copy, and that is the D28 pattern a third time
 *
 * `@bifurc/gui` has no `@bifurc/engine` dependency by design (`plan/19` finding 1), and the renderer still
 * needs this for presentation — which viewer to mount, whether a body is text. So the copy stays, and
 * **`tests/gui/binaryContentType.test.ts` pins it to this list**, exactly as
 * `tests/gui/randomizerCatalogue.test.ts` pins `RANDOMIZER_TOKENS` (D28) and
 * `tests/gui/curlHeaderSkip.test.ts` pins `SKIP_CURL_HEADERS`. A copy with no pin is a second
 * implementation nobody notices drifting; the pin is what makes it legitimate.
 *
 * The list is **substring prefixes, not exact types** — `image/svg+xml` and `image/png` both answer true —
 * so the pin compares behaviour on a sample rather than comparing two arrays.
 */

const TEXT_APPLICATION_TYPES = [
  "application/json",
  "application/xml",
  "application/javascript",
  "application/ecmascript",
  "application/x-www-form-urlencoded",
  "application/graphql",
  "application/x-ndjson",
  "application/yaml",
];

/**
 * `true` when a body of this type is text a mock can replace: JSON, HTML, XML, JavaScript, plain text and the
 * other text formats. An empty type counts as text. Images, audio, video, fonts, archives and any other
 * `application/*` data are not. Mirrors the desktop UI's `media::classify`.
 */
export function isTextContentType(ct: string): boolean {
  const kind = ct.split(";")[0].trim().toLowerCase();
  if (kind === "" || kind.startsWith("text/") || TEXT_APPLICATION_TYPES.includes(kind)) return true;
  if (kind.endsWith("+json") || kind.endsWith("+xml")) return true;
  const family = kind.split("/")[0];
  return !(family === "image" || family === "audio" || family === "video" || family === "font"
    || family === "application" || family === "multipart");
}

/**
 * `true` when a body of this type must be kept as base64 rather than decoded to text.
 *
 * The list is deliberately short and prefix-based. It is not "everything non-textual": a `text/*` type
 * missing from it decodes as text (harmless), while a *binary* type missing from it would be decoded as
 * UTF-8 and corrupted — which is why the prefixes here are the common ones rather than an exhaustive
 * registry.
 */
export function isBinaryContentType(ct: string): boolean {
  const lower = ct.toLowerCase();
  return (
    lower.includes("image/") ||
    lower.includes("audio/") ||
    lower.includes("video/") ||
    lower.includes("font/") ||
    lower.includes("application/octet-stream") ||
    lower.includes("application/pdf") ||
    lower.includes("application/zip") ||
    lower.includes("application/gzip")
  );
}
