/**
 * Write the Rust engine's command schemas from `@bifurc/protocol`'s zod schemas.
 *
 * The Rust engine validates every inbound payload against `packages/engine-rust/schemas/commands.json`
 * (`src/protocol/schema.rs`), the way the Node engine's `CommandRegistry` runs the zod schema. Generating the
 * file keeps one definition of the protocol while both engines exist.
 *
 * Run `npm run build --workspace @bifurc/protocol` first, then `node scripts/gen-engine-rust-schemas.mjs`.
 */
import { createRequire } from "node:module";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const { z } = require("zod");
const protocol = require(join(root, "packages/protocol/dist/index.cjs"));

function strip(node) {
  if (Array.isArray(node)) return node.map(strip);
  if (node === null || typeof node !== "object") return node;
  const out = {};
  for (const [key, value] of Object.entries(node)) {
    if (key === "$schema") continue;
    out[key] = strip(value);
  }
  return out;
}

const commands = {};
for (const [name, spec] of Object.entries(protocol.COMMANDS)) {
  // `io: "input"` keeps the difference between a strict object (`additionalProperties: false`, unknown keys
  // are an error) and a plain one (no keyword, unknown keys are dropped).
  commands[name] = strip(z.toJSONSchema(spec.params, { io: "input", unrepresentable: "throw" }));
}

const out = join(root, "packages/engine-rust/schemas/commands.json");
writeFileSync(out, `${JSON.stringify({ protocolVersion: protocol.PROTOCOL_VERSION, commands }, null, 1)}\n`);
console.log(`${Object.keys(commands).length} command schemas -> ${out}`);
