# Protocol changes

The protocol is **frozen at the end of P1**. Any change after that must be recorded here with a
justification, because every change costs a multiple of what it would have cost in P1.

**Rule:** if you are about to change a command name, a payload shape, an error code, or an event
envelope — write the entry first, then make the change.

---

## Template

```markdown
### YYYY-MM-DD — <short title>

**Phase:** P< n >
**Type:** breaking | additive | fix
**Change:** what changed
**Why it could not wait:**
**Affected:** commands / events / clients
**Migration:** what each client must do
**Approved by:**
```

---

## Change log

### 2026-09-15 — Protocol frozen at v1.0.0 (89 commands)

**Phase:** P1
**Type:** n/a — this is the freeze point, not a change
**Change:** `@bifurc/protocol` (`packages/protocol/`) built and frozen at `PROTOCOL_VERSION = "1.0.0"`,
covering all 80 ENGINE + 14 SPLIT (engine-half) commands enumerated in
`plan/handler-classification.md`, collapsed per work item 2 to 89 wire commands (36 generated CRUD
channels → the 6 generic `entity.*` commands). Full command list, params schemas, error taxonomy,
event envelope (with `seq`), `hello` handshake, and `subscribe`/`unsubscribe` are in
`packages/protocol/src/`.
**Why it could not wait:** n/a — this entry exists so every subsequent row has a clear baseline to
diff against.
**Affected:** the entire command surface, for the first time.
**Migration:** none yet — P2 onward consume this package; no client has been rewired to it yet
(that starts at P2/P5/P6).
**Approved by:** this session, per `plan/02-phase-1-protocol.md` acceptance criteria (all met — see
"Gate" note below).

*No changes since freeze.*

---

## Guidance

### Breaking changes

Require a `PROTOCOL_VERSION` major bump. Every client must be updated in lockstep (D9).

Examples: renaming a command, removing a field, changing a payload type, changing an error code's meaning.

### Additive changes

Minor bump. Clients must tolerate their absence — this is what capability negotiation is for
(`02-phase-1-protocol.md` item 7).

Examples: a new command, a new optional field, a new capability string.

### Do not do this

- Rename a command for aesthetics.
- Change a payload shape to be "cleaner" while clients are in flight.
- Add a command that duplicates an existing one because the name is inconvenient.
- Change the error envelope's structure.

### Before adding any command, check

1. Does an existing command already do this with a different payload?
2. Should this be a `kind` discriminator on `entity.*` instead of its own command? (This is how 36 CRUD
   channels became 6.)
3. Does the payload contain a filesystem path? It must not (`File_Ops_Protocol.md` §8).
4. Is there a schema, and does the engine validate against it?
5. Is it in the right scope class?
