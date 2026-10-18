//! Pairing a mock with the saved request it describes: a URL's path, `{{var}}` segments, method match, globs. Pure.

use serde_json::{Value, json};

fn field(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// A saved request as pairing sees it.
pub struct Req {
    pub id: String,
    pub name: String,
    pub method: String,
    pub path: String,
}

/// The path of a URL or URL pattern: no scheme and host (a leading `{{baseUrl}}` counts as one), no query or
/// fragment, no trailing slash. `{{var}}` segments stay.
pub fn path_of(url: &str) -> String {
    let mut rest = url.trim();
    rest = rest.find(['?', '#']).map_or(rest, |at| &rest[..at]);
    if rest.starts_with("{{") {
        rest = rest.find("}}").map_or(rest, |end| &rest[end + 2..]);
    }
    let host_first = !rest.starts_with('/') && rest.split('/').next().is_some_and(|h| h.contains(['.', ':']));
    if let Some((_, after)) = rest.split_once("://").or(rest.strip_prefix("://").map(|r| ("", r))) {
        rest = after;
        rest = rest.find('/').map_or("", |at| &rest[at..]);
    } else if host_first {
        rest = rest.find('/').map_or("", |at| &rest[at..]);
    }
    let rest = rest.trim_end_matches('/');
    if rest.starts_with('/') { rest.to_string() } else { format!("/{rest}") }
}

pub fn is_var(segment: &str) -> bool {
    (segment.starts_with("{{") && segment.ends_with("}}")) || (segment.starts_with(':') && segment.len() > 1)
}

/// Whether two paths are the same call: equal segment by segment, a `{{var}}` segment on either side matching
/// any one segment. Case-sensitive.
pub fn paths_match(a: &str, b: &str) -> bool {
    let (a, b): (Vec<&str>, Vec<&str>) = (a.split('/').collect(), b.split('/').collect());
    a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x == y || is_var(x) || is_var(y))
}

pub fn method_matches(mock: &str, request: &str) -> bool {
    mock == "*" || mock.eq_ignore_ascii_case(request)
}

/// The saved request a mock is paired with, or why there is none.
pub fn pair_of<'a>(mock: &Value, requests: &'a [Req]) -> std::result::Result<&'a Req, String> {
    if mock.get("useRegex").is_some_and(|v| v == &json!(true)) {
        // shortcut: a regex pattern is JavaScript's and pairing has no regex engine, so it is never paired;
        // to upgrade, evaluate it with the path of each request once this crate carries a regex dependency.
        return Err("regex pattern".into());
    }
    let (method, path) = (field(mock, "method"), path_of(&field(mock, "urlPattern")));
    let hits: Vec<&Req> =
        requests.iter().filter(|r| method_matches(&method, &r.method) && paths_match(&path, &r.path)).collect();
    match hits.as_slice() {
        [] => Err("no saved request".into()),
        [one] => Ok(one),
        many => {
            let exact: Vec<&&Req> = many.iter().filter(|r| r.path == path).collect();
            match exact.as_slice() {
                [one] => Ok(one),
                _ => Err(format!("ambiguous: {}", many.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "))),
            }
        }
    }
}

/// Whether `text` matches `glob`, where `*` stands for any run of characters.
pub fn glob_matches(glob: &str, text: &str) -> bool {
    let (g, t): (Vec<char>, Vec<char>) = (glob.chars().collect(), text.chars().collect());
    let (mut gi, mut ti, mut star) = (0, 0, None);
    while ti < t.len() {
        if gi < g.len() && g[gi] == '*' {
            star = Some((gi, ti));
            gi += 1;
        } else if gi < g.len() && g[gi] == t[ti] {
            gi += 1;
            ti += 1;
        } else if let Some((sg, st)) = star {
            gi = sg + 1;
            star = Some((sg, st + 1));
            ti = st + 1;
        } else {
            return false;
        }
    }
    g[gi..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(name: &str, method: &str, url: &str) -> Req {
        Req { id: name.into(), name: name.into(), method: method.into(), path: path_of(url) }
    }

    #[test]
    fn paths_lose_host_query_and_trailing_slash() {
        assert_eq!(path_of("https://api.test:8080/orders/?x=1#f"), "/orders");
        assert_eq!(path_of("{{baseUrl}}/users/{{id}}"), "/users/{{id}}");
        assert_eq!(path_of("https://{{host}}/a/b"), "/a/b");
        assert_eq!(path_of("api.localhost/orders"), "/orders");
        assert_eq!(path_of("http://api.test"), "/");
        assert_eq!(path_of("/orders"), "/orders");
    }

    #[test]
    fn variable_segments_match_one_segment_and_case_counts() {
        assert!(paths_match("/users/{{id}}", "/users/7"));
        assert!(paths_match("/users/7", "/users/{{id}}"));
        assert!(!paths_match("/users/{{id}}", "/users/7/posts"));
        assert!(!paths_match("/Users", "/users"));
    }

    #[test]
    fn pairing_prefers_an_exact_path_and_names_why_not() {
        let reqs = vec![
            req("Get orders", "GET", "{{baseUrl}}/orders"),
            req("Get order", "GET", "{{baseUrl}}/orders/{{id}}"),
            req("Get order 7", "GET", "{{baseUrl}}/orders/7"),
            req("Get order x", "GET", "{{baseUrl}}/orders/{{x}}"),
            req("Make order", "POST", "{{baseUrl}}/orders/"),
        ];
        let mock = |method: &str, url: &str| json!({"method": method, "urlPattern": url});
        let paired = |m: Value| pair_of(&m, &reqs).map(|r| r.name.clone());
        assert_eq!(paired(mock("get", "http://h/orders?a=1")), Ok("Get orders".into()));
        assert_eq!(paired(mock("POST", "http://h/orders")), Ok("Make order".into()));
        assert_eq!(paired(mock("GET", "http://h/orders/7")), Ok("Get order 7".into()));
        assert_eq!(paired(mock("GET", "http://h/orders/9")), Err("ambiguous: Get order, Get order x".into()));
        assert_eq!(paired(mock("DELETE", "http://h/orders")), Err("no saved request".into()));
        let mut regex = mock("GET", "/orders/\\d+");
        regex["useRegex"] = json!(true);
        assert_eq!(paired(regex), Err("regex pattern".into()));
    }

    #[test]
    fn globs_let_a_star_stand_for_any_run() {
        assert!(glob_matches(".meta.*", ".meta.page.total"));
        assert!(glob_matches(".items[].debug", ".items[].debug"));
        assert!(glob_matches("*.debug", ".items[].debug"));
        assert!(!glob_matches(".meta.*", ".meta"));
        assert!(!glob_matches(".items[].debug", ".items[].debugx"));
        assert!(glob_matches("*", ""));
    }
}
