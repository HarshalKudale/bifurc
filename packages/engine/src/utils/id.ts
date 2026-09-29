/**
 * `mkId()` — a short, collision-resistant, **sortable-by-time** identifier for a request log entry.
 *
 * ## Why it is its own file
 *
 * It lived in `utils/server.ts` until engine-layout's cleanup. That file was phase 4's `proxy/serverUtils.ts`
 * moved to `utils/` because both proxy halves needed it — and it also carried four functions that read the
 * filesystem, which is what made `utils/` stop being the pure layer §5.1 defines it as. Splitting it left
 * this function needing a home: it is pure and stateless, so `utils/` is right, and it belongs to neither of
 * the two modules the other five exports went to (`workspace/dispatch.ts` for the config derivations,
 * `utils/net.ts` for `hostnameOf`).
 *
 * ## Why not `randomizer.ts`
 *
 * `utils/randomizer.ts` generates **substitutable values** — `{{$random.email}}`-style tokens that appear in
 * a request and are resolved into it. This generates an **identifier** for a log entry. The two are both
 * "random" and nothing else, and folding one into the other would make a reader looking for the
 * request-log id search a module about mock payloads.
 *
 * ## The shape, and what it is not
 *
 * `Date.now()` in base 36, then four base-36 characters of `Math.random()`. The time prefix makes ids sort
 * chronologically as strings, which is what the log list wants; the random suffix keeps two entries created
 * in the same millisecond distinct.
 *
 * **It is not a UUID and is not claimed to be one.** 36⁴ ≈ 1.7M suffixes per millisecond, so a collision
 * needs two entries in the *same millisecond* that also draw the same suffix — and nothing in this codebase
 * treats the id as a security boundary. It is a label that must not repeat within one workspace's log.
 */
export function mkId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
}
