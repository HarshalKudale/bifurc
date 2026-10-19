# UI text and languages

Every sentence the window shows comes from `src/i18n.rs`, looked up by key in the language the user chose
(first step of the first run; stored in `desktop-ui.json` as `language`). Only `crates/app` is covered:
the engine, CLI and MCP server keep their own text.

## Layout

`locales/<code>/<area>.json` is one flat `{ "key": "text" }` map per area. The file name is the first part
of a key: `"stage.terms.title"` in `onboarding.json` is `t("onboarding.stage.terms.title")` in code.

| area | source it belongs to |
| --- | --- |
| `app` | `app_model.rs`, `engine.rs`, `commands.rs`, `bootstrap.rs`, `theme.rs`, `panels/` |
| `onboarding`, `workspace_setup`, `settings`, `workspace`, `vaults`, `capture`, `audit`, `mappings`, `proxy_rules`, `environments`, `services`, `healthbar`, `licenses` | `screens/<same name>.rs` |
| `requests`, `mocks`, `flows` | `screens/requests/`, `screens/mocks/`, `screens/flows/` (+ `model/flow.rs` for flows) |
| `shell` | `shell/` |
| `components` | `components/` |
| `model` | `model/` (except `model/flow.rs`) |
| `session` | `session/` |

## In code

```rust
use crate::i18n::{t, t_with};

Button::new("save").label(t("components.entity_bar.save.label"))             // t() -> &'static str
div().child(t_with("onboarding.problem.server_start", &[("port", &port)]))   // "…port {port}…" -> String
```

- `t` returns `&'static str`, so a helper that returned `&'static str` keeps its signature: `t(match x { A => "area.k1", B => "area.k2" })`.
- A `const`/`static` cannot call `t`: keep the **key** in the const and call `t(key)` where it is drawn (see `GROUPS` in `shell/nav.rs`).
- Variable text: a `{name}` placeholder in the value and `t_with`. Never glue translated pieces together with `format!`/`+`; a language
  orders words differently. Plurals are two keys, `…_one` / `…_other`, chosen in code (see `screens/licenses.rs`).
- Keys are lowercase `snake_case`, dot separated, the part after the area is `section.item[.part]` (e.g. `entity_bar.sync.tooltip_push`). Write
  them as full literals so they can be grepped; do not build them with `format!`. The same sentence used in several places is one key.
- The test `every_key_the_source_asks_for_exists` fails on any quoted key the English files lack.

## What is text, and what is not

Move: labels, titles, buttons, menu items, tooltips, placeholders, empty states, dialog text, status/progress/error
messages the UI writes itself, search keywords users type.

Leave alone: the brand name "Bifurc"; HTTP methods, header names, content types, URLs, file names; JSON keys and engine command/event
names; element ids (`Button::new("…")`, `ElementId`); anything compared with `==`/`match` or sent to or stored by the engine (the
`"Default"` folder name, request kinds on the wire); text that comes back from the engine; `tracing`/`panic!`/`expect` messages;
test modules; code samples and example data.

## Adding a language

Copy `locales/en/` to `locales/<code>/`, translate the values (keep keys and `{placeholders}`), add
`language!("<code>", "<name in that language>")` to `LANGUAGES` in `src/i18n.rs`. The language then shows in the
first-run picker. A key a language lacks falls back to English.
