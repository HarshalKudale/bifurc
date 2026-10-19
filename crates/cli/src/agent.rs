//! `bifurc agent init`: teach an AI agent this CLI in one read. A short cheat sheet and the names in the
//! workspace go into the repo's `AGENTS.md` / `CLAUDE.md` between markers (re-running replaces them in place),
//! and with `--skill` into a skill file too.

use std::path::Path;

use serde_json::{Value, json};

use crate::{
    commands::{Context, unknown_verb},
    error::{CliError, Result},
    format::js_template,
    usage::group_usage,
};

const BEGIN: &str = "<!-- bifurc:begin -->";
const END: &str = "<!-- bifurc:end -->";
const SKILL: &str = ".claude/skills/bifurc/SKILL.md";
/// Names listed per fact line before it is cut with `…`.
const MAX_NAMES: usize = 15;

/// The cheat sheet. Every `bifurc …` in backticks is parsed against the help texts by a test.
const SHEET: &str = r#"## Bifurc
Bifurc sends the API calls of the user's workspace (shared cookies, environments, mocks, captured traffic). Use it instead of curl.
- Call once and save it: `bifurc curl -X POST {{baseUrl}}/users -d '{"email":"a@b.co"}' --save "Create user" --folder auth`
- Call it again: `bifurc requests send "Create user" --var email=x@y.co --expect-status 201 --select .id`; `bifurc requests list` lists the saved ones.
- Output is one summary line (`201 Created · 142ms · 4.2KB json`); `--body`, `--headers` and `--select <path>` show more.
- Exit codes: 0 ok, 1 a check failed, 2 usage, 3 not found, 4 network, 5 no engine, 6 needs a person, 7 blocked by policy: stop and ask.
- Traffic the proxies captured: `bifurc capture list --status 4xx --last 5`, then `bifurc capture show #k3f9 --select .error`
- Mocks: `bifurc mocks list`, `bifurc mocks create http://api.localhost/users/1 --status 200 -d '{"id":1}'`, `bifurc mocks disable "Get user"`
- Mock states: `bifurc mocks scenario "Get user" error500` (also empty, slow, timeout, notfound; default returns), `bifurc mocks status`
- Flows: `bifurc flows create --steps checkout.yaml`; `bifurc flows run "Checkout"`. A folder of requests: `bifurc runner run auth --bail`
- Variables: `bifurc env show`; a send takes `--env <name>`; `bifurc env auth dev --login Login` re-logs in on a 401.
- Set BIFURC_SESSION=<your role> (e.g. frontend) so your cookies, variables and env never clash with another agent or the user; `--extract id=.id` keeps a value for later {{id}}. `bifurc changes` lists what others changed since you last asked.
- Cookies: `bifurc cookies list`, `bifurc cookies clear --domain api.localhost`
- Wait for a service: `bifurc wait http://api.localhost/health`, `bifurc wait --port 3000`. What is on: `bifurc status`
- Secrets print masked and vault values never print. Do not paste tokens into commands: use {{variables}}.
- Windows Git Bash rewrites arguments that start with `/`: set `MSYS_NO_PATHCONV=1` or use a glob: `bifurc capture list --path '*/orders'`
- More: bifurc <group> --help, bifurc help <group> --examples."#;

const SKILL_HEAD: &str = "---\nname: bifurc\ndescription: Use for API calls, mocks, captured traffic and flows of the user's Bifurc workspace, instead of curl.\n---\n\n";

/// `<label>: a, b, c` for the names, cut at [`MAX_NAMES`]; `None` when there are none.
fn fact(label: &str, names: Vec<String>, suffix: &str) -> Option<String> {
    let mut shown: Vec<String> = names.iter().take(MAX_NAMES).map(|n| format!("{n}{suffix}")).collect();
    if shown.is_empty() {
        return None;
    }
    if names.len() > MAX_NAMES {
        shown.push("…".to_string());
    }
    Some(format!("{label}: {}", shown.join(", ")))
}

/// The facts of the active workspace, one line each; `None` when no engine answers.
fn facts(ctx: &mut Context) -> Option<Vec<String>> {
    let config = ctx.request("config.get", json!({})).ok()?;
    let rows = |key: &str| config.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let name = |row: &Value| js_template(row.get("name"));
    // Global is always applied, so it is not a choice an agent makes.
    let environments: Vec<Value> = rows("environments").into_iter().filter(|e| e["id"] != "__global__").collect();
    let selected = environments.iter().find(|e| Some(&e["id"]) == config.get("activeEnvironmentId")).map(name);
    let mut lines = Vec::new();
    lines.extend(fact("Environments", environments.iter().map(name).collect(), "").map(|line| match selected {
        Some(selected) => format!("{line} (selected: {selected})"),
        None => line,
    }));
    // The top of the tree is what a folder path starts with.
    let top = rows("requestFolders")
        .into_iter()
        .filter(|f| f["parentId"].is_null() && f["deleted"] != true)
        .map(|f| name(&f))
        .collect();
    lines.extend(fact("Request folders", top, "/"));
    lines.extend(fact("Flows", rows("flows").iter().map(name).collect(), ""));
    Some(lines)
}

/// `old` with the block put in place of the one between the markers, or appended when it has none. The file's
/// own line ending is kept.
fn merge(old: &str, body: &str, file: &str) -> Result<String> {
    let eol = if old.contains("\r\n") { "\r\n" } else { "\n" };
    let block = format!("{BEGIN}\n{body}\n{END}").replace('\n', eol);
    match (old.find(BEGIN), old.find(END)) {
        (Some(begin), Some(end)) if begin < end => Ok(format!("{}{block}{}", &old[..begin], &old[end + END.len()..])),
        (None, None) => {
            let mut text = old.to_string();
            if !text.is_empty() {
                if !text.ends_with('\n') {
                    text.push_str(eol);
                }
                text.push_str(eol);
            }
            Ok(format!("{text}{block}{eol}"))
        }
        // Replacing from a lone marker could eat the user's text.
        _ => {
            Err(CliError::msg(format!("{file} has a bifurc marker without its pair; fix or remove it, then run again")))
        }
    }
}

fn write(path: &Path, text: &str) -> Result<()> {
    std::fs::write(path, text).map_err(|e| CliError::msg(format!("cannot write {}: {e}", path.display())))
}

/// `agent init [--dir <repo>] [--target agents|claude|both] [--skill]`, and the group help for `agent`.
pub fn agent(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub {
        None => return Ok(group_usage("agent").unwrap_or_default()),
        Some("init") => ctx.accept_top(&["dir", "target", "skill"], 1)?,
        Some(other) => return Err(unknown_verb("agent", other)),
    }
    let dir = Path::new(ctx.args.flag("dir").unwrap_or("."));
    if !dir.is_dir() {
        return Err(CliError::usage(format!("--dir {} is not a directory", dir.display())));
    }
    let files: &[&str] = match ctx.args.flag("target") {
        Some("agents") => &["AGENTS.md"],
        Some("claude") => &["CLAUDE.md"],
        Some("both") => &["AGENTS.md", "CLAUDE.md"],
        Some(other) => return Err(CliError::usage(format!("--target is agents, claude or both, not \"{other}\""))),
        None => match (dir.join("AGENTS.md").is_file(), dir.join("CLAUDE.md").is_file()) {
            (false, true) => &["CLAUDE.md"],
            (true, true) => &["AGENTS.md", "CLAUDE.md"],
            _ => &["AGENTS.md"],
        },
    };

    let mut body = SHEET.to_string();
    match facts(ctx) {
        Some(lines) => lines.iter().for_each(|line| body.push_str(&format!("\n{line}"))),
        None => eprintln!("bifurc: no engine answered, so the workspace names are left out"),
    }

    let mut done: Vec<(String, &str)> = Vec::new();
    for file in files {
        let path = dir.join(file);
        let (text, action) = match std::fs::read(&path) {
            Ok(bytes) => {
                let old = String::from_utf8(bytes).map_err(|_| CliError::msg(format!("{file} is not UTF-8 text")))?;
                (merge(&old, &body, file)?, "updated")
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (merge("", &body, file)?, "created"),
            Err(e) => return Err(CliError::msg(format!("cannot read {file}: {e}"))),
        };
        write(&path, &text)?;
        done.push((file.to_string(), action));
    }
    if ctx.args.has("skill") {
        let path = dir.join(SKILL);
        std::fs::create_dir_all(path.parent().unwrap_or(dir))
            .map_err(|e| CliError::msg(format!("cannot create {}: {e}", path.display())))?;
        write(&path, &format!("{SKILL_HEAD}{body}\n"))?;
        done.push((SKILL.to_string(), "wrote"));
    }

    let value = json!({ "files": done.iter().map(|(path, action)| json!({ "path": path, "action": action })).collect::<Vec<_>>() });
    Ok(ctx.print(&value, || {
        let lines: Vec<String> = done
            .iter()
            .map(|(path, action)| match *action {
                "updated" => format!("updated {path} (bifurc block)"),
                action => format!("{action} {path}"),
            })
            .collect();
        lines.join("\n")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, run};

    fn engine() -> Vec<Answer> {
        vec![(
            "config.get",
            Ok(json!({
                "activeEnvironmentId": "e1",
                "environments": [{"id": "__global__", "name": "Global"}, {"id": "e1", "name": "dev"}, {"id": "e2", "name": "staging"}],
                "requestFolders": [
                    {"id": "f1", "name": "auth", "parentId": null},
                    {"id": "f2", "name": "orders"},
                    {"id": "f3", "name": "admin", "parentId": "f1"},
                    {"id": "f4", "name": "old", "deleted": true},
                ],
                "flows": [{"id": "x", "name": "Checkout"}, {"id": "y", "name": "Signup"}],
            })),
        )]
    }

    /// `agent init` in `dir` with the scripted engine (or none); the output and what the engine was asked.
    fn init(dir: &Path, extra: &[&str], answers: Vec<Answer>) -> Result<String> {
        let path = dir.to_str().unwrap();
        let mut argv = vec!["agent", "init", "--dir", path];
        argv.extend_from_slice(extra);
        run(&argv, answers, |ctx| agent(ctx, Some("init"))).0
    }

    fn read(dir: &Path, file: &str) -> String {
        std::fs::read_to_string(dir.join(file)).unwrap()
    }

    #[test]
    fn an_empty_directory_gets_agents_md_with_the_sheet_and_the_facts() {
        let dir = tempfile::tempdir().unwrap();
        let out = init(dir.path(), &[], engine()).unwrap();
        assert_eq!(out, "created AGENTS.md");
        let text = read(dir.path(), "AGENTS.md");
        assert!(text.starts_with(&format!("{BEGIN}\n## Bifurc\n")) && text.ends_with(&format!("\n{END}\n")));
        assert!(text.contains(
            "\nEnvironments: dev, staging (selected: dev)\nRequest folders: auth/, orders/\nFlows: Checkout, Signup\n"
        ));
        assert!(!dir.path().join("CLAUDE.md").exists());
    }

    #[test]
    fn the_sheet_is_fifteen_to_twenty_lines() {
        assert!((15..=20).contains(&SHEET.lines().count()), "{}", SHEET.lines().count());
    }

    #[test]
    fn the_files_that_exist_are_the_targets_and_target_overrides() {
        let (only_claude, both, flag) =
            (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::write(only_claude.path().join("CLAUDE.md"), "# Mine\n").unwrap();
        assert_eq!(init(only_claude.path(), &[], vec![]).unwrap(), "updated CLAUDE.md (bifurc block)");
        assert!(!only_claude.path().join("AGENTS.md").exists());

        for file in ["AGENTS.md", "CLAUDE.md"] {
            std::fs::write(both.path().join(file), "x\n").unwrap();
        }
        assert_eq!(
            init(both.path(), &[], vec![]).unwrap(),
            "updated AGENTS.md (bifurc block)\nupdated CLAUDE.md (bifurc block)"
        );

        std::fs::write(flag.path().join("AGENTS.md"), "x\n").unwrap();
        assert_eq!(init(flag.path(), &["--target", "claude"], vec![]).unwrap(), "created CLAUDE.md");
        assert_eq!(init(flag.path(), &["--target", "both"], vec![]).unwrap().lines().count(), 2);
        let err = init(flag.path(), &["--target", "cursor"], vec![]).unwrap_err();
        assert_eq!(err.code(), 2);
    }

    #[test]
    fn running_again_replaces_the_block_in_place_and_keeps_the_rest_byte_for_byte() {
        for eol in ["\n", "\r\n"] {
            let dir = tempfile::tempdir().unwrap();
            let (before, after) =
                (format!("# Repo{eol}{eol}Rules.{eol}"), format!("{eol}## Later{eol}tail, no newline"));
            let old = format!("{before}{BEGIN}{eol}stale{eol}{END}{after}");
            std::fs::write(dir.path().join("AGENTS.md"), &old).unwrap();
            init(dir.path(), &[], engine()).unwrap();
            let once = read(dir.path(), "AGENTS.md");
            assert!(once.starts_with(&before) && once.ends_with(&after) && !once.contains("stale"), "{once:?}");
            assert_eq!(once.matches(BEGIN).count(), 1);
            // CRLF in, CRLF out: no bare LF anywhere.
            assert!(eol == "\n" || once.replace("\r\n", "").find('\n').is_none(), "{once:?}");
            init(dir.path(), &[], engine()).unwrap();
            assert_eq!(read(dir.path(), "AGENTS.md"), once);
        }
    }

    #[test]
    fn a_file_without_markers_gets_the_block_after_its_text_and_a_lone_marker_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "# Repo").unwrap();
        init(dir.path(), &[], vec![]).unwrap();
        assert!(read(dir.path(), "AGENTS.md").starts_with(&format!("# Repo\n\n{BEGIN}\n")));
        std::fs::write(dir.path().join("AGENTS.md"), format!("keep\n{BEGIN}\nmine\n")).unwrap();
        assert!(init(dir.path(), &[], vec![]).is_err());
        assert_eq!(read(dir.path(), "AGENTS.md"), format!("keep\n{BEGIN}\nmine\n"));
    }

    #[test]
    fn skill_writes_the_frontmatter_and_the_same_body_without_markers_and_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let out = init(dir.path(), &["--skill"], engine()).unwrap();
        assert_eq!(out, "created AGENTS.md\nwrote .claude/skills/bifurc/SKILL.md");
        let skill = read(dir.path(), SKILL);
        assert!(
            skill.starts_with("---\nname: bifurc\ndescription: Use for API calls, mocks, captured traffic and flows")
        );
        assert!(skill.contains("\n---\n\n## Bifurc\n") && skill.contains("Flows: Checkout, Signup"));
        assert!(!skill.contains(BEGIN));
        std::fs::write(dir.path().join(SKILL), "old").unwrap();
        init(dir.path(), &["--skill"], engine()).unwrap();
        assert_eq!(read(dir.path(), SKILL), skill);
    }

    #[test]
    fn json_lists_the_files_and_what_was_done() {
        let dir = tempfile::tempdir().unwrap();
        let out = init(dir.path(), &["--skill", "--json"], vec![]).unwrap();
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            parsed,
            json!({"files": [{"path": "AGENTS.md", "action": "created"}, {"path": SKILL, "action": "wrote"}]})
        );
    }

    #[test]
    fn long_lists_are_cut_and_no_engine_leaves_the_facts_out() {
        let names: Vec<String> = (1..=16).map(|n| format!("f{n}")).collect();
        let line = fact("Flows", names, "").unwrap();
        assert!(line.ends_with("f15, …") && !line.contains("f16"), "{line}");
        assert_eq!(fact("Flows", vec![], ""), None);

        let dir = tempfile::tempdir().unwrap();
        init(dir.path(), &[], vec![]).unwrap();
        let text = read(dir.path(), "AGENTS.md");
        assert!(!text.contains("Environments:") && text.contains("## Bifurc"));
    }

    #[test]
    fn the_group_without_a_verb_is_its_help_and_an_unknown_verb_is_a_usage_error() {
        let (out, _) = run(&["agent"], vec![], |ctx| agent(ctx, None));
        assert_eq!(out.unwrap(), group_usage("agent").unwrap());
        let (out, _) = run(&["agent", "nope"], vec![], |ctx| agent(ctx, Some("nope")));
        assert_eq!(out.unwrap_err().code(), 2);
        let (out, _) = run(&["agent", "init", "--dir", "/no/such/place"], vec![], |ctx| agent(ctx, Some("init")));
        assert_eq!(out.unwrap_err().code(), 2);
    }

    #[test]
    fn through_the_cli_without_an_engine_it_still_writes_and_says_what_it_left_out() {
        let (dir, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let argv: Vec<String> = [
            "agent",
            "init",
            "--no-start",
            "--dir",
            dir.path().to_str().unwrap(),
            "--data-dir",
            data.path().to_str().unwrap(),
        ]
        .iter()
        .map(|a| a.to_string())
        .collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(crate::cli::run(&argv, &mut out, &mut err), 0);
        assert_eq!(String::from_utf8(out).unwrap(), "created AGENTS.md\n");
        assert!(dir.path().join("AGENTS.md").is_file());
    }

    #[test]
    fn every_command_of_the_sheet_parses_and_its_flags_are_in_the_help() {
        let commands: Vec<&str> = SHEET.split('`').skip(1).step_by(2).filter(|s| s.starts_with("bifurc ")).collect();
        assert!(commands.len() >= 10, "{commands:?}");
        for line in commands {
            crate::usage::tests::check_command(line);
        }
        // Each group the task names is in the sheet.
        for needle in [
            "curl",
            "requests send",
            "capture list",
            "capture show",
            "mocks",
            "flows run",
            "runner run",
            "env show",
            "cookies clear",
            "wait",
            "status",
        ] {
            assert!(SHEET.contains(&format!("bifurc {needle}")), "{needle}");
        }
    }
}
