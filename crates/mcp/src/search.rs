//! Finding a tool by what it is for. Only a handful of tools is listed upfront; the rest are found here, read
//! with `describe_tool` and run with `call_tool`: the catalog, inspect, execute layers the protocol's guidance
//! describes for a server with more tools than a model should carry around.
//!
//! The search is by keyword: the catalog is a few dozen rows with descriptive names, and a keyword match on
//! names, summaries and argument names finds them without a model or an index.

use crate::tools::Tool;

/// How many tools a search returns when it is not told.
pub const DEFAULT_LIMIT: usize = 10;

/// What a word of the query is worth where it is found.
const IN_NAME: u32 = 6;
const IN_SUMMARY: u32 = 3;
const IN_DETAILS: u32 = 1;
const IN_PARAMETER: u32 = 1;

/// Words that say nothing about which tool is wanted.
const NOISE: [&str; 14] = ["a", "an", "the", "of", "to", "for", "in", "on", "and", "or", "my", "all", "with", "bifurc"];

/// What people call things, as the catalog calls them.
fn canonical(word: &str) -> &str {
    match word {
        "api" | "call" | "http" | "endpoint" => "request",
        "stub" | "fake" => "mock",
        "domain" | "route" | "host" => "mapping",
        "environment" | "variable" | "var" => "env",
        "healthcheck" | "ping" | "uptime" => "health",
        "secret" | "password" | "credential" => "vault",
        "port" | "process" => "service",
        "repo" | "repository" | "git" => "sync",
        "new" | "add" | "make" | "save" => "create",
        "edit" | "change" | "modify" | "rename" => "update",
        "remove" | "erase" | "drop" => "delete",
        "copy" | "clone" => "duplicate",
        "certificate" | "cert" | "https" | "ssl" => "tls",
        "traffic" | "captured" => "capture",
        "script" | "field" => "patch",
        "get" | "view" | "read" | "inspect" => "show",
        "run" | "execute" | "invoke" | "fire" => "send",
        "switch" | "select" | "activate" => "use",
        "on" => "enable",
        "off" => "disable",
        other => other,
    }
}

/// A word without its plural, so `mocks` finds `mock` and the other way round.
fn stem(word: &str) -> &str {
    let word = word.strip_suffix("es").filter(|rest| rest.ends_with("ss")).unwrap_or(word);
    match word.strip_suffix('s') {
        Some(rest) if rest.len() > 2 && !rest.ends_with('s') => rest,
        _ => word,
    }
}

/// The words of `text` that are worth matching: lowercase, singular.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty() && !NOISE.contains(word))
        .map(|word| stem(word).to_string())
        .collect()
}

/// How well `tool` answers the query's words: each word counts once, where it is worth most.
fn score(tool: &Tool, wanted: &[String]) -> u32 {
    let name = words(tool.name);
    let summary = words(tool.summary);
    let details = words(&tool.details);
    let params: Vec<String> = tool.params.iter().flat_map(|param| words(param.name)).collect();
    wanted
        .iter()
        .map(|word| {
            // The word as it was said, or as the catalog says it, whichever is found where it is worth more.
            let said = [word.as_str(), stem(canonical(word))];
            let found = |among: &[String]| among.iter().any(|candidate| said.contains(&candidate.as_str()));
            if found(&name) {
                // A name said as it is spelt beats one reached through a synonym (`vars` is not `env`).
                IN_NAME + u32::from(name.contains(word))
            } else if found(&summary) {
                IN_SUMMARY
            } else if found(&details) {
                IN_DETAILS
            } else if found(&params) {
                IN_PARAMETER
            } else {
                0
            }
        })
        .sum()
}

/// The tools that answer `query`, best first, at most `limit`. An empty query is the whole catalog in its order.
pub fn search<'t>(tools: &'t [Tool], query: &str, limit: usize) -> Vec<&'t Tool> {
    let wanted = words(query);
    if wanted.is_empty() {
        return tools.iter().take(limit).collect();
    }
    let mut scored: Vec<(u32, usize, &Tool)> = tools
        .iter()
        .enumerate()
        .map(|(at, tool)| (score(tool, &wanted), at, tool))
        .filter(|(score, _, _)| *score > 0)
        .collect();
    // Best first; among equals, the catalog's order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().take(limit).map(|(_, _, tool)| tool).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::catalog;

    fn found(query: &str) -> Vec<&'static str> {
        search(&catalog(), query, DEFAULT_LIMIT).into_iter().map(|tool| tool.name).collect()
    }

    #[test]
    fn a_task_finds_its_tool_first() {
        for (query, expected) in [
            ("create a health check", "health_create"),
            ("rename workspace", "workspaces_update"),
            ("switch environment", "env_use"),
            ("disable a mock", "mocks_disable"),
            ("add forward proxy rule", "rules_create"),
            ("sync workspace to git", "workspaces_sync"),
            ("grpc reflection services", "requests_reflect"),
            ("list secrets", "vault_list"),
            ("update saved request headers", "requests_update"),
            ("github repositories", "account_repos"),
            ("which ports are listening", "services_list"),
            ("run health checks", "health_run"),
        ] {
            assert_eq!(found(query).first().copied(), Some(expected), "{query}: {:?}", found(query));
        }
    }

    #[test]
    fn a_group_name_finds_the_whole_group_singular_or_plural() {
        for query in ["rules", "rule"] {
            let mut names = found(query);
            names.sort_unstable();
            assert_eq!(
                names,
                [
                    "folders_list",
                    "rules_create",
                    "rules_delete",
                    "rules_disable",
                    "rules_duplicate",
                    "rules_enable",
                    "rules_list",
                    "rules_patch",
                    "rules_show",
                    "rules_update",
                ]
            );
        }
    }

    #[test]
    fn an_empty_query_is_the_catalog_and_nonsense_finds_nothing() {
        let all = catalog();
        assert_eq!(search(&all, "", 1000).len(), all.len());
        assert_eq!(search(&all, "  the of ", 3).len(), 3);
        assert!(found("xyzzy plugh").is_empty());
    }

    #[test]
    fn the_limit_is_kept() {
        assert_eq!(search(&catalog(), "list", 4).len(), 4);
    }

    #[test]
    fn every_tool_is_found_by_its_own_name() {
        for tool in catalog() {
            assert_eq!(found(tool.name).first().copied(), Some(tool.name));
        }
    }
}
