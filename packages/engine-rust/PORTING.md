# Porting the engine from Node to Rust

`packages/engine` (TypeScript) is being rewritten here, module by module, until this crate has full parity
and the Node engine can be deleted. This file is the contract every port follows.

## What "parity" means

- **The wire is frozen.** Clients (the GPUI desktop UI in `ui/`, the CLI) must not be able to tell which
  engine they talk to: same command names, same result shapes and field names, same events.
  `packages/protocol/src` is the definition; the TypeScript handler is the behavior.
- **The data directory is shared.** Both engines read and write the same `app.json` and `data/<wsId>/…`
  files. Keep file names, JSON field names, field order and two-space indentation
  (`utils::json::write_json_pretty`).
- **The TypeScript is the spec.** Port what the code does, including its quirks, unless this file says
  otherwise. The TS files are extremely comment-heavy; the comments explain history, the code is what counts.
- Out of scope: Electron (`runtime/desktop` is the tray helper, not Electron), the in-process transport,
  webhooks. Mocks have no scripts.

## The engine's shape

- One process, one **current-thread tokio runtime**. Command handlers run on that one thread, so a handler's
  read-modify-write of a file cannot interleave with another handler's between `.await`s — the same model as
  Node. Do not add locks around store files.
- Handler futures must still be `Send` (`CommandRegistry::register` requires it). Keep `parking_lot` guards
  out of `.await` points. Global state is `static` + `parking_lot::Mutex` / `RwLock`.
- Never block the thread for long. File reads/writes of workspace JSON are fine synchronously (`std::fs`).
  Child processes use `tokio::process` via `utils::process::hidden_command` (no console window on Windows).
  CPU-bound or blocking work (running a user script) goes through `tokio::task::spawn_blocking`.
- Entities, settings and config snapshots are `serde_json::Value` (built with `preserve_order`), **not**
  typed structs: a file must round-trip with fields this engine does not know, in their original order.

## The API you build on

Read these files before writing; they are short.

| Need | Use |
|---|---|
| register a command | `kernel::CommandRegistry::register(action, \|params, ctx\| async move { … })` / `register_sync(action, \|params, ctx\| …)`; both return `protocol::Result<Value>` |
| the payload | `params: Value`, already validated against the protocol schema: required fields exist with the right types, unknown keys of strict objects were rejected. Read with `utils::json::{str_of, opt_str, bool_of, i64_of, array_of, truthy, string_map}` |
| call another command | `ctx.registry.invoke("request.replay", payload, ctx.clone()).await` |
| errors | `protocol::EngineError::{bad_request, not_found, conflict, unsupported, upstream, engine}`. A TS `throw new Error(m)` is `EngineError::engine(m)`; a TS `throw new EngineError("NOT_FOUND", m)` is `EngineError::not_found(m)`. `?` works on `std::io::Error` and `serde_json::Error` |
| events | `kernel::bus().emit("entity.changed", json!({…}))` — bus names have no `event.` prefix. Listen with `bus().on(name, \|payload\| …)` |
| captured traffic | `kernel::telemetry::{RequestLogEntry, Headers, emit_log, emit_log_chunk, emit_server_error}` |
| ids, time | `utils::id::{generate_id, mk_id, now_ms}` |
| settings (`app.json`) | `workspace::settings::{load_settings, save_settings, AppSettings}` |
| config snapshot | `workspace::config::{load_config, save_config, load_entity, load_workspace_dispatch_slice}` |
| entity files | `workspace::fs::*` (`read_all_entities`, `read_entity`, `write_entity`, `write_flat_entity`, `find_entity_rel_path`, `read_index`/`write_index`, names index, enabled set, pending deletions, `ws_dir`, `entity_dir`, `sanitize_dir_name`) |
| entity kinds | `workspace::kinds::{CRUD_KINDS, crud_kind, is_simple_kind, kind_name_entry}`; wire kind names ↔ store names via `kernel::kinds` |
| git | `workspace::git_store::{git, git_in, commit_mutation, query_log, get_entity_at_commit, …}` — git is driven through its CLI |
| environments | `env::{load_merged_environment, resolve_vars, resolve_headers, effective_environment_id, merge_env_vars}` |
| proxies | `runtime::instances::{Listener, forward_proxy, edge, notify_config_changed, …}` |
| git status push | `runtime::entity_status::emit_entity_status(ws_id)` |
| notices to the operator | `runtime::notice(msg)` (never `println!` directly: stdout carries frames in stdio mode) |

TypeScript → Rust habits:

- `x ?? d`, `x || d`: mind the difference. `||` treats `""`, `0`, `false` as missing — use
  `utils::json::is_truthy` / `truthy`.
- `{ ...a, ...b }` is `utils::json::merged(&a, &b)`.
- A handler that returns `undefined` returns `Value::Null`. A handler that returns `{ ok: false, error }` is
  returning **data**, not failing: return `Ok(json!({ "ok": false, "error": … }))`.
- An optional result field that TS leaves `undefined` must be **absent** from the JSON, not `null`.
- `new RegExp(userPattern)` is `regress::Regex::new(pattern)` (JavaScript regex semantics). Never the
  `regex` crate for a user-supplied pattern.
- `Buffer.from(s, "base64")` / `.toString("base64")` is `base64::engine::general_purpose::STANDARD`.
- `Date.now()` is `now_ms()`.
- HTTP header maps are `Record<string, string>` in TS: use `telemetry::Headers` (an `IndexMap`, insertion
  ordered) in Rust code, and a JSON object on the wire.

## Rules for a porting task

1. Work only in the files you were assigned. Other modules are being ported in parallel by other people.
   `lib.rs`, `register_all.rs` and the other modules' `mod.rs` already declare your module and call your
   `register(registry)` — do not edit them. If you believe you must change a shared file, make the smallest
   change and say so in your report.
2. Keep the public signatures that already exist in your module's stub (`pub fn register`, and any other
   `pub fn` there): other modules call them.
3. `Cargo.toml` already lists the dependencies planned for the port. Add one only if truly needed and
   report it.
4. Write unit tests beside the code (`#[cfg(test)] mod tests`) for the logic you port — matching, parsing,
   merging, file round-trips. Tests that touch the store call `crate::testing::scratch_data_root()` first and
   keep the returned guard alive (the data root is process-wide). Tests may bind loopback ports (port `0`) but
   must never reach the internet, write outside a temp dir, or touch the real user data directory or
   certificate store.
5. Match the surrounding code: module doc comment saying what the module is for, short doc comments on
   public items, no history or "ported from" narration, no `TODO`s left behind. `max_width = 120`.
6. Before you report: `cargo fmt -p bifurc-engine`, `cargo check -p bifurc-engine` with **no warnings** in
   your files, `cargo test -p bifurc-engine` green.
7. Commit your work on your branch with a conventional message (`feat(engine-rust): …`). Plain
   `git commit -m`; do **not** add a `Co-Authored-By` trailer or any other attribution.
8. Never write source files with shell heredocs or `echo` — use the file editing tools (the shell mangles
   backticks and backslashes).
9. In your final report list: the commands you registered, anything you deliberately did differently from
   the TypeScript and why, anything you could not port, and any shared file you touched.
