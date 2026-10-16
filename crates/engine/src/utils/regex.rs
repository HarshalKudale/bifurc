//! Compiled user patterns (mock URLs, proxy rules, flow conditions), kept so a request never compiles one.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use parking_lot::Mutex;
use regress::Regex;

/// Patterns held before the cache starts over; a user's own patterns are few, this only bounds `{{var}}` churn.
const LIMIT: usize = 1024;

static CACHE: LazyLock<Mutex<HashMap<String, Option<Arc<Regex>>>>> = LazyLock::new(Default::default);

/// The compiled `pattern`, compiled once; `None` when it is not a valid regular expression (it then matches
/// nothing, as a stored pattern that no longer compiles always has).
pub fn compiled(pattern: &str) -> Option<Arc<Regex>> {
    let mut cache = CACHE.lock();
    if let Some(hit) = cache.get(pattern) {
        return hit.clone();
    }
    if cache.len() >= LIMIT {
        cache.clear();
    }
    let re = Regex::new(pattern).ok().map(Arc::new);
    cache.insert(pattern.to_string(), re.clone());
    re
}

/// Whether `pattern` is a regular expression that finds a match in `text`.
pub fn is_match(pattern: &str, text: &str) -> bool {
    compiled(pattern).is_some_and(|re| re.find(text).is_some())
}

/// The error for a pattern that does not compile, naming it; `Ok` for a valid one. A pattern with a `{{var}}`
/// in it is only known once the environment fills it in, so it is not checked here.
pub fn validate(label: &str, pattern: &str) -> crate::protocol::Result<()> {
    if pattern.contains("{{") || Regex::new(pattern).is_ok() {
        return Ok(());
    }
    Err(crate::protocol::EngineError::with_details(
        crate::protocol::ErrorCode::BadRequest,
        format!("{label} \"{pattern}\" is not a valid regular expression."),
        serde_json::json!({ "field": label }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_like_a_fresh_compile_and_an_invalid_pattern_matches_nothing() {
        for (pattern, text) in [(r"^http://a\.test/y/\d+$", "http://a.test/y/12"), ("^a$", "ab"), ("(", "("), ("", "x")]
        {
            let fresh = Regex::new(pattern).is_ok_and(|re| re.find(text).is_some());
            // Twice: the second answer comes from the cache.
            assert_eq!((is_match(pattern, text), is_match(pattern, text)), (fresh, fresh), "{pattern}");
        }
        assert!(compiled("(").is_none());
    }

    #[test]
    fn validation_names_the_pattern() {
        assert!(validate("pattern", "^a+$").is_ok());
        assert!(validate("pattern", "^{{host}}/(").is_ok());
        let err = validate("pattern", "(").unwrap_err();
        assert!(err.to_string().contains("\"(\""), "{err}");
    }
}
