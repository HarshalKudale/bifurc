//! The cookies a response sets, read from its `Set-Cookie` headers, and the JWTs among their values.
//!
//! The engine hands the response headers over as a map, joining repeated headers with ", ", so several
//! `Set-Cookie` lines arrive as one string that [`split_set_cookie`] takes apart again.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;

use super::{git::date_time_utc, http::pretty_json};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResponseCookie {
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub expires: Option<String>,
    pub max_age: Option<String>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
}

/// Every cookie the `Set-Cookie` headers of a response set, in order.
pub fn from_headers(headers: &[(String, String)]) -> Vec<ResponseCookie> {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
        .flat_map(|(_, joined)| split_set_cookie(joined))
        .filter_map(|line| parse(&line))
        .collect()
}

/// Splits headers joined with ", ". A comma inside `Expires=Wed, 21 Oct ...` does not separate cookies.
pub fn split_set_cookie(joined: &str) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut rest = joined;
    while let Some(at) = rest.find(',') {
        let (before, after) = (&rest[..at], &rest[at + 1..]);
        let current = lines.last_mut().expect("one line always exists");
        current.push_str(before);
        let attribute = current.rsplit(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if attribute.starts_with("expires=") && !attribute.contains(',') {
            current.push(',');
        } else {
            lines.push(String::new());
        }
        rest = after;
    }
    lines.last_mut().expect("one line always exists").push_str(rest);
    lines.into_iter().map(|line| line.trim().to_owned()).filter(|line| !line.is_empty()).collect()
}

fn parse(line: &str) -> Option<ResponseCookie> {
    let mut parts = line.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let mut cookie =
        ResponseCookie { name: name.trim().to_owned(), value: value.trim().to_owned(), ..Default::default() };
    if cookie.name.is_empty() {
        return None;
    }
    for attribute in parts {
        let (key, value) = attribute.split_once('=').unwrap_or((attribute, ""));
        let value = Some(value.trim().to_owned()).filter(|v| !v.is_empty());
        match key.trim().to_ascii_lowercase().as_str() {
            "domain" => cookie.domain = value,
            "path" => cookie.path = value,
            "expires" => cookie.expires = value,
            "max-age" => cookie.max_age = value,
            "samesite" => cookie.same_site = value,
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            _ => {}
        }
    }
    Some(cookie)
}

/// A decoded JSON Web Token. The signature is not checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jwt {
    pub header: String,
    pub payload: String,
    /// `exp`, `iat` and `nbf` as `(claim, UTC time)` when the payload has them.
    pub times: Vec<(&'static str, String)>,
    /// `exp` in epoch seconds, for telling whether it has passed.
    pub expires_at: Option<u64>,
}

impl Jwt {
    pub fn is_expired(&self, now_secs: u64) -> bool {
        self.expires_at.is_some_and(|exp| exp <= now_secs)
    }
}

/// `Some` when `value` is three base64url segments whose header is a JSON object with `alg`.
pub fn decode_jwt(value: &str) -> Option<Jwt> {
    let mut segments = value.split('.');
    let (header, payload, _signature) = (segments.next()?, segments.next()?, segments.next()?);
    if segments.next().is_some() {
        return None;
    }
    let part = |segment: &str| String::from_utf8(URL_SAFE_NO_PAD.decode(segment.trim_end_matches('=')).ok()?).ok();
    let (header, payload) = (part(header)?, part(payload)?);
    let head: Value = serde_json::from_str(&header).ok()?;
    head.as_object()?.get("alg")?;
    let claims: Value = serde_json::from_str(&payload).ok()?;
    let seconds = |claim: &str| claims.get(claim).and_then(Value::as_f64).filter(|s| *s >= 0.0).map(|s| s as u64);
    let times = ["exp", "iat", "nbf"]
        .into_iter()
        .filter_map(|claim| {
            let at = seconds(claim)?;
            Some((claim, format!("{}:{:02} UTC", date_time_utc(at * 1000), at % 60)))
        })
        .collect();
    Some(Jwt {
        header: pretty_json(&header).unwrap_or(header),
        payload: pretty_json(&payload).unwrap_or(payload),
        times,
        expires_at: seconds("exp"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Header `{"alg":"HS256","typ":"JWT"}`, payload `{"sub":"1","exp":1700000000,"iat":1699990000}`.
    pub const TOKEN: &str =
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxIiwiZXhwIjoxNzAwMDAwMDAwLCJpYXQiOjE2OTk5OTAwMDB9.c2ln";

    #[test]
    fn repeated_headers_split_but_a_date_keeps_its_comma() {
        let joined = "a=1; Expires=Wed, 21 Oct 2026 07:28:00 GMT; Path=/, b=2; Secure, c=3";
        assert_eq!(
            split_set_cookie(joined),
            ["a=1; Expires=Wed, 21 Oct 2026 07:28:00 GMT; Path=/", "b=2; Secure", "c=3"]
        );
    }

    #[test]
    fn attributes_are_read_whatever_their_case() {
        let line = "sid=abc=; DOMAIN=x.io; path=/a; Max-Age=60; secure; HttpOnly; SameSite=Lax";
        let headers =
            vec![("content-type".to_owned(), "text/plain".to_owned()), ("set-cookie".to_owned(), line.to_owned())];
        let cookies = from_headers(&headers);
        assert_eq!(cookies.len(), 1);
        let c = &cookies[0];
        assert_eq!((c.name.as_str(), c.value.as_str()), ("sid", "abc="));
        assert_eq!(
            (c.domain.as_deref(), c.path.as_deref(), c.max_age.as_deref()),
            (Some("x.io"), Some("/a"), Some("60"))
        );
        assert!(c.secure && c.http_only);
        assert_eq!(c.same_site.as_deref(), Some("Lax"));
        assert!(from_headers(&[("set-cookie".into(), "garbage".into())]).is_empty());
    }

    #[test]
    fn a_jwt_is_decoded_with_its_times() {
        let jwt = decode_jwt(TOKEN).expect("a token");
        assert!(jwt.header.contains("\"alg\": \"HS256\""));
        assert!(jwt.payload.contains("\"sub\": \"1\""));
        assert_eq!(jwt.times[0], ("exp", "2023-11-14 22:13:20 UTC".to_owned()));
        assert_eq!(jwt.times[1].0, "iat");
        assert!(jwt.is_expired(1_700_000_000) && !jwt.is_expired(1_699_999_999));
    }

    #[test]
    fn other_values_are_not_jwts() {
        for value in ["abc", "a.b.c", "a.b", &format!("{TOKEN}.x"), "eyJ4IjoxfQ.eyJ4IjoxfQ.c"] {
            assert!(decode_jwt(value).is_none(), "{value}");
        }
    }
}
