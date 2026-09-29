/**
 * `websocket/` — **reserved. Empty as a module, and correct as one.**
 *
 * `Engine_Module_Plan.md` §3 lists this directory as *"reserved — the `wsConnections` kind; no
 * behaviour yet"*, and §9 states the rule that keeps it that way: **do not give `websocket/` invented
 * content.** There is no WebSocket server here to extract, no handler to move and no state to own.
 *
 * ## What exists today, and why none of it belongs in this file
 *
 * - **The `wsConnections` kind is a stub.** `workspace/config.ts` emits `wsConnections` as stubs
 *   (`wsConnectionStubs`), and `kernel/kinds.ts` maps the engine-internal name `sockets` to the wire
 *   name `wsConnections`. The kind is registered through the shared generic CRUD table
 *   (`workspace/entities/kinds/crudKinds.ts`), exactly like `mappings`/`rules`/`mocks` — it has no
 *   behaviour of its own to own. Import/export handles it as data
 *   (`importexport/{exporters,importers}/websockets-json.ts`), which is also data, not protocol.
 * - **`reverseproxy/edge.ts`'s `upgrade` branch is reserved and deliberately has no body.** It is a
 *   routing decision that nobody has made yet, and a branch returning "not built yet" would be
 *   inventing that decision. §9 names this explicitly as a thing not to do.
 *
 * ## Why the file exists at all
 *
 * So the module map is *complete*: §7.2's phase 7 creates the directory, and §5.2's phase-8 ratchet
 * asserts that every module in the layer table is imported by something. An absent directory cannot be
 * referenced; an empty one can, and it marks the intended boundary for the first real WebSocket work
 * without pretending any of it is done.
 *
 * `export {}` below is what makes this a *module* rather than a script — it has no runtime effect and
 * exports nothing. When the first WebSocket behaviour lands, it replaces this line.
 */
export {};
