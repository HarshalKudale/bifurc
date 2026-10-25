//! The sample library: predefined webhook payloads kept as files in `samples/`, one JSON file each. To add one,
//! drop a file there and list it in [`SAMPLES`].
//!
//! A sample is `{ name, about, headers, signatureHeader, payload }`: the headers the provider sends, the name of
//! the header it signs into (empty: unsigned) and the body. Inside the payload and the header values, a string
//! can hold tokens that are filled in afresh each time the sample is built: `{{unix}}` and `{{iso}}` (the time; a string that is
//! only `{{unix}}` becomes a number), `{{hex:N}}`, `{{alnum:N}}` and `{{digits:N}}` (N random characters). A
//! third part names the value, `{{hex:40:after}}`, and `{{after}}` then repeats it. Anything else in braces,
//! such as `{{baseUrl}}`, is left for the request's variables.

use std::{collections::HashMap, sync::LazyLock};

use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};

use crate::importexport::support::iso_from_ms;

/// The embedded sample files.
const SAMPLES: [&str; 8] = [
    include_str!("samples/stripe.checkout.session.completed.json"),
    include_str!("samples/stripe.payment_intent.succeeded.json"),
    include_str!("samples/stripe.invoice.paid.json"),
    include_str!("samples/github.push.json"),
    include_str!("samples/github.ping.json"),
    include_str!("samples/github.pull_request.opened.json"),
    include_str!("samples/discord.message.json"),
    include_str!("samples/slack.message.json"),
];

pub struct Template {
    pub name: String,
    pub about: String,
    /// The header the provider signs into; empty for a sender that does not sign.
    pub signature_header: String,
    headers: Value,
    payload: Value,
}

static LIBRARY: LazyLock<Vec<Template>> = LazyLock::new(|| {
    SAMPLES
        .iter()
        .map(|file| {
            let sample: Value = serde_json::from_str(file).expect("a webhook sample is valid JSON");
            let text = |key: &str| sample[key].as_str().unwrap_or_default().to_string();
            Template {
                name: text("name"),
                about: text("about"),
                signature_header: text("signatureHeader"),
                headers: sample["headers"].clone(),
                payload: sample["payload"].clone(),
            }
        })
        .collect()
});

const ALNUM: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const HEX: &str = "0123456789abcdef";
const DIGITS: &str = "0123456789";

/// Every sample, in the order [`SAMPLES`] lists them.
pub fn all() -> &'static [Template] {
    &LIBRARY
}

/// `len` random characters of `alphabet`.
pub fn random_chars(alphabet: &str, len: usize) -> String {
    let chars: Vec<char> = alphabet.chars().collect();
    let mut bytes = vec![0u8; len];
    // If the system RNG fails the ids repeat one character: still valid fake data.
    let _ = SystemRandom::new().fill(&mut bytes);
    bytes.iter().map(|b| chars[usize::from(*b) % chars.len()]).collect()
}

/// The sample `wanted` names: exactly, by the start of its name in any case (`stripe.invoice`), or by its
/// provider alone (`stripe`), which is that provider's first sample.
pub fn find(wanted: &str) -> Result<&'static Template, String> {
    let lower = wanted.to_lowercase();
    if let Some(first) = all().iter().find(|t| t.provider() == lower) {
        return Ok(first);
    }
    let hits: Vec<&Template> = all().iter().filter(|t| t.name.starts_with(&lower)).collect();
    match (all().iter().find(|t| t.name == wanted), hits.as_slice()) {
        (Some(template), _) => Ok(template),
        (None, [template]) => Ok(template),
        _ => {
            let names: Vec<&str> = all().iter().map(|t| t.name.as_str()).collect();
            Err(format!("no fixture \"{wanted}\"; fixtures: {}", names.join(", ")))
        }
    }
}

/// `2026-10-18T09:30:00Z` for unix seconds.
pub fn format_time(secs: u64) -> String {
    let iso = iso_from_ms(secs as i64 * 1000).unwrap_or_default();
    iso.replace(".000Z", "Z")
}

impl Template {
    /// Who sends it: the first part of its name.
    pub fn provider(&self) -> &str {
        self.name.split('.').next().unwrap_or_default()
    }

    /// The payload with its tokens filled in for `now` (unix seconds).
    pub fn payload(&self, now: u64) -> Value {
        let mut payload = self.payload.clone();
        fill(&mut payload, now, &mut HashMap::new());
        payload
    }

    /// The headers with their tokens filled in, in the order the sample lists them.
    pub fn headers(&self, now: u64) -> Vec<(String, String)> {
        let mut names = HashMap::new();
        let mut headers = self.headers.clone();
        fill(&mut headers, now, &mut names);
        headers
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(name, value)| value.as_str().map(|value| (name.clone(), value.to_string())))
            .collect()
    }
}

fn fill(value: &mut Value, now: u64, names: &mut HashMap<String, String>) {
    match value {
        Value::String(text) if text == "{{unix}}" => *value = json!(now),
        Value::String(text) => *text = expand(text, now, names),
        Value::Array(items) => items.iter_mut().for_each(|item| fill(item, now, names)),
        Value::Object(map) => map.values_mut().for_each(|item| fill(item, now, names)),
        _ => {}
    }
}

fn expand(text: &str, now: u64, names: &mut HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(length) = rest[start..].find("}}") else { break };
        out.push_str(&rest[..start]);
        let token = &rest[start + 2..start + length];
        match value_of(token, now, names) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[start..start + length + 2]),
        }
        rest = &rest[start + length + 2..];
    }
    out.push_str(rest);
    out
}

/// What a token stands for, or `None` for one that is not ours.
fn value_of(token: &str, now: u64, names: &mut HashMap<String, String>) -> Option<String> {
    let mut parts = token.split(':');
    let (kind, len, name) = (parts.next()?, parts.next().and_then(|n| n.parse().ok()), parts.next());
    if len.is_none()
        && let Some(known) = names.get(kind)
    {
        return Some(known.clone());
    }
    if let Some(known) = name.and_then(|name| names.get(name)) {
        return Some(known.clone());
    }
    let value = match (kind, len) {
        ("unix", _) => now.to_string(),
        ("iso", _) => format_time(now),
        ("hex", Some(n)) => random_chars(HEX, n),
        ("alnum", Some(n)) => random_chars(ALNUM, n),
        ("digits", Some(n)) => random_chars(DIGITS, n),
        _ => return None,
    };
    if let Some(name) = name {
        names.insert(name.to_string(), value.clone());
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sample_is_valid_and_builds_with_no_token_left_over() {
        let names: Vec<&str> = all().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names.len(), SAMPLES.len());
        for template in all() {
            assert!(!template.about.is_empty() && template.headers.is_object(), "{}", template.name);
            let built = template.payload(1_760_779_800).to_string() + &format!("{:?}", template.headers(0));
            assert!(!built.contains("{{"), "{}: {built}", template.name);
        }
        assert!(names.contains(&"discord.message") && names.contains(&"slack.message"));
    }

    #[test]
    fn tokens_become_fresh_values_and_a_name_repeats_one() {
        let push = find("github.push").unwrap().payload(1_760_779_800);
        let after = push["after"].as_str().unwrap();
        assert_eq!(after.len(), 40);
        assert_eq!(push["head_commit"]["id"], after);
        assert_eq!(push["commits"][0]["timestamp"], "2025-10-18T09:30:00Z");
        assert!(push["compare"].as_str().unwrap().ends_with(&format!("...{after}")));
        assert_ne!(push["before"], push["after"]);
        let invoice = find("stripe.invoice.paid").unwrap();
        assert_eq!(invoice.payload(1_760_779_800)["created"], 1_760_779_800u64);
        assert_ne!(invoice.payload(1_760_779_800), invoice.payload(1_760_779_800));
    }

    #[test]
    fn a_sample_names_the_headers_its_sender_sends_and_the_one_it_signs_into() {
        let push = find("github").unwrap();
        assert_eq!((push.name.as_str(), push.provider()), ("github.push", "github"));
        let headers = push.headers(0);
        assert!(headers.contains(&("X-GitHub-Event".to_string(), "push".to_string())));
        assert_eq!(headers.iter().find(|(name, _)| name == "X-GitHub-Delivery").unwrap().1.len(), 36);
        assert_eq!(push.signature_header, "X-Hub-Signature-256");
        assert_eq!(find("stripe.invoice").unwrap().signature_header, "Stripe-Signature");
        assert_eq!(find("discord").unwrap().signature_header, "");
    }

    #[test]
    fn a_token_that_is_not_ours_is_left_for_the_request_variables() {
        let mut names = HashMap::new();
        assert_eq!(expand("{{baseUrl}}/x/{{digits:3}}", 0, &mut names).len(), "{{baseUrl}}/x/".len() + 3);
        assert!(expand("{{baseUrl}}/x", 0, &mut names).starts_with("{{baseUrl}}"));
    }

    #[test]
    fn times_read_like_the_cli_always_printed_them() {
        assert_eq!(format_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_time(1_760_779_800), "2025-10-18T09:30:00Z");
    }
}
