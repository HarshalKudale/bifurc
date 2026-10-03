//! Argument parsing: `bifurc <positionals…> --flag value --flag=value -H value --json`. Flags may repeat.

use std::collections::HashMap;

/// Flags that take no value.
const BOOLEAN_FLAGS: [&str; 8] =
    ["json", "help", "no-start", "no-tray", "passphrase-stdin", "value-stdin", "public", "force"];

/// Single-letter aliases; any other letter stands for itself.
fn alias(letter: char) -> String {
    match letter {
        'H' => "header",
        'X' => "method",
        'd' => "body",
        'h' => "help",
        'w' => "workspace",
        other => return other.to_string(),
    }
    .to_string()
}

/// The positionals and flags of one command line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedArgs {
    pub positionals: Vec<String>,
    flags: HashMap<String, Vec<String>>,
}

impl ParsedArgs {
    /// The last value given for `name`.
    pub fn flag(&self, name: &str) -> Option<&str> {
        self.flags.get(name).and_then(|values| values.last()).map(String::as_str)
    }

    /// Every value given for `name`, in order.
    pub fn flags(&self, name: &str) -> &[String] {
        self.flags.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Whether `name` was given at all.
    pub fn has(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    /// The positional at `index`.
    pub fn positional(&self, index: usize) -> Option<&str> {
        self.positionals.get(index).map(String::as_str)
    }

    fn add(&mut self, name: &str, value: String) {
        self.flags.entry(name.to_string()).or_default().push(value);
    }
}

/// `--name` or `--name=value`: the name and the inline value, when the argument has that shape.
fn long_flag(raw: &str) -> Option<(&str, Option<&str>)> {
    let rest = raw.strip_prefix("--")?;
    match rest.split_once('=') {
        Some((name, value)) if !name.is_empty() => Some((name, Some(value))),
        Some(_) => None,
        None if rest.is_empty() => None,
        None => Some((rest, None)),
    }
}

/// `-x`: a single ASCII letter.
fn short_flag(raw: &str) -> Option<char> {
    let mut chars = raw.strip_prefix('-')?.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), None) if letter.is_ascii_alphabetic() => Some(letter),
        _ => None,
    }
}

/// Parse the arguments after the program name. A value flag with no value is an error.
pub fn parse_args(argv: &[String]) -> Result<ParsedArgs, String> {
    let mut parsed = ParsedArgs::default();
    let mut i = 0;
    while i < argv.len() {
        let raw = argv[i].as_str();
        if raw == "--" {
            parsed.positionals.extend(argv[i + 1..].iter().cloned());
            break;
        }
        let (name, inline_value) = match (long_flag(raw), short_flag(raw)) {
            (Some((name, value)), _) => (name.to_string(), value.map(str::to_string)),
            (None, Some(letter)) => (alias(letter), None),
            (None, None) => {
                parsed.positionals.push(raw.to_string());
                i += 1;
                continue;
            }
        };
        i += 1;
        if BOOLEAN_FLAGS.contains(&name.as_str()) {
            parsed.add(&name, "true".to_string());
            continue;
        }
        let value = match inline_value {
            Some(value) => value,
            None => {
                let next = argv.get(i).ok_or_else(|| format!("--{name} needs a value"))?;
                i += 1;
                next.clone()
            }
        };
        parsed.add(&name, value);
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ParsedArgs, String> {
        parse_args(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    fn ok(args: &[&str]) -> ParsedArgs {
        parse(args).unwrap()
    }

    #[test]
    fn separates_positionals_from_flags_in_either_form() {
        let a = ok(&["mappings", "add", "api", "127.0.0.1:3000", "--label", "My API", "--data-dir=/d"]);
        assert_eq!(a.positionals, ["mappings", "add", "api", "127.0.0.1:3000"]);
        assert_eq!(a.flag("label"), Some("My API"));
        assert_eq!(a.flag("data-dir"), Some("/d"));
    }

    #[test]
    fn collects_repeated_flags_and_expands_the_curl_style_aliases() {
        let a = ok(&["requests", "send", "--url", "http://x", "-X", "POST", "-H", "a: 1", "-H", "b: 2", "-d", "{}"]);
        assert_eq!(a.flag("method"), Some("POST"));
        assert_eq!(a.flags("header"), ["a: 1", "b: 2"]);
        assert_eq!(a.flag("body"), Some("{}"));
    }

    #[test]
    fn the_last_value_of_a_repeated_flag_wins() {
        let a = ok(&["--label", "one", "--label=two"]);
        assert_eq!(a.flag("label"), Some("two"));
        assert_eq!(a.flags("label"), ["one", "two"]);
    }

    #[test]
    fn json_and_help_are_switches() {
        let a = ok(&["status", "--json", "-h"]);
        assert!(a.has("json"));
        assert!(a.has("help"));
        assert_eq!(a.positionals, ["status"]);
    }

    #[test]
    fn no_start_and_no_tray_are_switches() {
        let a = ok(&["engine", "start", "--no-tray", "--no-start", "x"]);
        assert!(a.has("no-tray") && a.has("no-start"));
        assert_eq!(a.positionals, ["engine", "start", "x"]);
    }

    #[test]
    fn a_switch_ignores_an_inline_value() {
        let a = ok(&["--json=false"]);
        assert_eq!(a.flag("json"), Some("true"));
    }

    #[test]
    fn workspace_has_a_short_form() {
        assert_eq!(ok(&["-w", "ws1"]).flag("workspace"), Some("ws1"));
    }

    #[test]
    fn an_unknown_short_letter_stands_for_itself() {
        assert_eq!(ok(&["-q", "v"]).flag("q"), Some("v"));
    }

    #[test]
    fn keeps_everything_after_double_dash_as_positionals() {
        assert_eq!(ok(&["call", "--", "--not-a-flag"]).positionals, ["call", "--not-a-flag"]);
    }

    #[test]
    fn a_value_may_start_with_a_dash() {
        assert_eq!(ok(&["--body", "-5"]).flag("body"), Some("-5"));
    }

    #[test]
    fn an_inline_value_may_contain_equals_signs_and_be_empty() {
        let a = ok(&["--url=http://x?a=b", "--label="]);
        assert_eq!(a.flag("url"), Some("http://x?a=b"));
        assert_eq!(a.flag("label"), Some(""));
    }

    #[test]
    fn arguments_that_only_look_like_flags_are_positionals() {
        let a = ok(&["-", "-ab", "--=x", "-1"]);
        assert_eq!(a.positionals, ["-", "-ab", "--=x", "-1"]);
    }

    #[test]
    fn refuses_a_value_flag_with_no_value() {
        assert_eq!(parse(&["mappings", "add", "--label"]).unwrap_err(), "--label needs a value");
        assert_eq!(parse(&["-H"]).unwrap_err(), "--header needs a value");
    }

    #[test]
    fn absent_flags_read_as_empty() {
        let a = ok(&["status"]);
        assert_eq!(a.flag("json"), None);
        assert!(a.flags("header").is_empty());
        assert!(!a.has("json"));
        assert_eq!(a.positional(0), Some("status"));
        assert_eq!(a.positional(1), None);
    }
}
