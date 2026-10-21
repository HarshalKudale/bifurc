//! How a webhook is signed: an HMAC-SHA256 of the exact body bytes, put in a header. The header's name says
//! which scheme the receiver expects.

use ring::hmac;

/// The header a signature goes in unless the request names another.
pub const DEFAULT_SIGNATURE_HEADER: &str = "x-webhook-signature";

pub fn hmac_hex(key: &[u8], data: &[u8]) -> String {
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data);
    tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The value of the signature header `header` for `body`, signed with `secret` at `now` (unix seconds).
/// `Stripe-Signature` carries `t=<time>,v1=<hex of HMAC over "<time>.<body>">`; every other header carries
/// `sha256=<hex of HMAC over the body>`, which is what GitHub's `X-Hub-Signature-256` expects.
pub fn signature(header: &str, body: &[u8], secret: &str, now: u64) -> String {
    if header.eq_ignore_ascii_case("stripe-signature") {
        let mut signed = format!("{now}.").into_bytes();
        signed.extend(body);
        format!("t={now},v1={}", hmac_hex(secret.as_bytes(), &signed))
    } else {
        format!("sha256={}", hmac_hex(secret.as_bytes(), body))
    }
}

/// The name of the header a received request signs into, if it has one: the first whose name says so.
pub fn signature_header_of<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    names.into_iter().find(|name| {
        let lower = name.to_ascii_lowercase();
        lower.contains("signature") || lower.contains("hmac") || lower.ends_with("-sig")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231() {
        assert_eq!(
            hmac_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hmac_hex(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First"),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn stripe_signs_the_timestamp_and_the_body() {
        let value = signature("Stripe-Signature", b"{\"a\":1}", "whsec_abc", 1_700_000_000);
        assert_eq!(value, format!("t=1700000000,v1={}", hmac_hex(b"whsec_abc", b"1700000000.{\"a\":1}")));
    }

    #[test]
    fn any_other_header_gets_the_prefixed_hex_of_the_body() {
        let expected = format!("sha256={}", hmac_hex(b"gh-s3cret", b"{}"));
        assert_eq!(signature("X-Hub-Signature-256", b"{}", "gh-s3cret", 0), expected);
        assert_eq!(signature("X-Anything", b"{}", "gh-s3cret", 99), expected);
    }

    #[test]
    fn the_signature_header_of_a_request_is_found_by_its_name() {
        let names = ["content-type", "x-github-event", "X-Hub-Signature-256", "user-agent"];
        assert_eq!(signature_header_of(names), Some("X-Hub-Signature-256"));
        assert_eq!(signature_header_of(["stripe-signature"]), Some("stripe-signature"));
        assert_eq!(signature_header_of(["x-shopify-hmac-sha256"]), Some("x-shopify-hmac-sha256"));
        assert_eq!(signature_header_of(["content-type", "user-agent"]), None);
    }
}
