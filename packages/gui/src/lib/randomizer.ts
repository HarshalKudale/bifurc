/**
 * The `{{random.*}}` token **catalogue** — and, as of `plan/19` WI 6, nothing else.
 *
 * ## What this file used to be
 *
 * Until 2026-09-30 it was a full second implementation: the catalogue, the static data pools
 * (`FIRST_NAMES`, `CITIES`, …), the `pick`/`randInt`/`hexStr` helpers, `generateRandom`'s 31-case switch and
 * `resolveRandomizers`. `packages/engine/src/utils/randomizer.ts` held the same thing, character for
 * character, and **nothing pinned the two equal** — `plan/19` §1 row 2: *"the data pools are identical today
 * and nothing pins them equal — they agree by discipline, not by construction."*
 *
 * The generator is gone: it was only ever reachable through `lib/resolveVars.ts`, which is deleted, and the
 * engine's copy is the authority. The file **survives** rather than being deleted, and that is the same
 * outcome `lib/runnerReport.ts` had in WI 4 — the ratchet keys on `file#construct`, so the file staying while
 * the construct leaves is the keying doing its job rather than an oversight.
 *
 * ## Why the catalogue stays, and what makes keeping it legitimate
 *
 * `RandomizerHint.tsx` offers these keys in its autocomplete popover, and the renderer **cannot** import the
 * engine (`@bifurc/gui` has no `@bifurc/engine` dependency by design — `plan/19` finding 1). The owner's
 * decision, 2026-09-29 (D28), is that this is a **list vs resolver** split rather than a renderer-vs-engine
 * one: *"random.\* types can be kept in UI as auto complete but actual value resolver still stays in engine.
 * UI just has copy of list of random.\* attributes."*
 *
 * **⚠ A copy with no pin is the defect §1 row 2 opened with**, so the pin is not optional:
 * `tests/gui/randomizerCatalogue.test.ts` asserts this list equals the engine's, **and** that every
 * catalogued key has a matching `case` in the engine's generator. The second half is what stops the popover
 * offering a key that would be inserted into a request and then returned verbatim by `generateRandom`'s
 * `default:` branch.
 *
 * **What the pin cannot catch**, recorded rather than papered over: a generator `case` with *no* catalogue
 * entry. Nothing can enumerate a `switch`, so that direction is invisible — and it was verified by mutation
 * rather than assumed (the test's docblock records it).
 */

export interface RandomizerToken {
  key: string;          // e.g. "random.name"
  description: string;  // human-readable label
  example: string;      // sample output shown in UI
}

export const RANDOMIZER_TOKENS: RandomizerToken[] = [
  // Person
  { key: "random.name",        description: "Full name",       example: "Jane Smith" },
  { key: "random.firstName",   description: "First name",      example: "Jane" },
  { key: "random.lastName",    description: "Last name",       example: "Smith" },
  { key: "random.email",       description: "Email address",   example: "jane.smith@example.com" },
  { key: "random.username",    description: "Username",        example: "jane_smith42" },
  { key: "random.phone",       description: "Phone number",    example: "+1-555-867-5309" },
  { key: "random.avatar",      description: "Avatar URL",      example: "https://i.pravatar.cc/150?u=..." },
  // Numbers
  { key: "random.int",         description: "Integer (0–9999)", example: "4217" },
  { key: "random.float",       description: "Decimal (0–1)",   example: "0.8312" },
  { key: "random.boolean",     description: "true / false",    example: "true" },
  { key: "random.price",       description: "Price (USD)",     example: "29.99" },
  // Identifiers
  { key: "random.uuid",        description: "UUID v4",         example: "550e8400-e29b-..." },
  { key: "random.id",          description: "Short ID (8 hex)", example: "a1b2c3d4" },
  // Internet
  { key: "random.url",         description: "URL",             example: "https://example.com/path" },
  { key: "random.ip",          description: "IPv4 address",    example: "192.168.1.42" },
  { key: "random.ipv6",        description: "IPv6 address",    example: "2001:db8::1" },
  { key: "random.color",       description: "Hex color",       example: "#a3f0c2" },
  // Date / time
  { key: "random.date",        description: "ISO date",        example: "2025-08-14" },
  { key: "random.timestamp",   description: "Unix timestamp",  example: "1720000000" },
  { key: "random.isoDateTime", description: "ISO datetime",    example: "2025-08-14T10:30:00Z" },
  // Location
  { key: "random.city",        description: "City name",       example: "Portland" },
  { key: "random.country",     description: "Country name",    example: "Germany" },
  { key: "random.zipCode",     description: "ZIP / postal code", example: "97201" },
  { key: "random.address",     description: "Street address",  example: "742 Evergreen Terrace" },
  // Text
  { key: "random.word",        description: "Random word",     example: "cerulean" },
  { key: "random.sentence",    description: "Short sentence",  example: "The quick brown fox." },
  { key: "random.paragraph",   description: "Paragraph text",  example: "Lorem ipsum..." },
  // Misc
  { key: "random.company",     description: "Company name",    example: "Acme Corp" },
  { key: "random.jobTitle",    description: "Job title",       example: "Senior Engineer" },
  { key: "random.currency",    description: "Currency code",   example: "EUR" },
  { key: "random.locale",      description: "Locale string",   example: "en-US" },
];
