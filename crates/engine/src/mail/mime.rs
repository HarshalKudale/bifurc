//! Just enough MIME for a verification mail: the subject (RFC 2047 words decoded), who it is from and to, and
//! the text and html bodies (multipart nesting, base64 and quoted-printable). Attachments are skipped.

use base64::Engine as _;

#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub from: String,
    /// The addresses of the `To` header, lower case.
    pub to: Vec<String>,
    pub subject: String,
    pub text: String,
    pub html: String,
}

type Headers = Vec<(String, String)>;

/// Parts nested deeper than this are ignored.
const MAX_DEPTH: usize = 8;

/// `message` split at its first empty line.
fn split(message: &str) -> (&str, &str) {
    match message.strip_prefix('\n') {
        Some(body) => ("", body),
        None => message.split_once("\n\n").unwrap_or((message, "")),
    }
}

/// The header lines as (lower case name, unfolded value).
fn headers(head: &str) -> Headers {
    let mut out: Headers = Vec::new();
    for line in head.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = out.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
        } else if let Some((name, value)) = line.split_once(':') {
            out.push((name.trim().to_lowercase(), value.trim().to_string()));
        }
    }
    out
}

fn header<'a>(all: &'a Headers, name: &str) -> Option<&'a str> {
    all.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
}

/// A parameter of a header value like `text/html; charset="utf-8"`.
fn param(value: &str, key: &str) -> Option<String> {
    value.split(';').skip(1).find_map(|part| {
        let (name, value) = part.split_once('=')?;
        name.trim().eq_ignore_ascii_case(key).then(|| value.trim().trim_matches('"').to_string())
    })
}

/// The addresses in a header like `A <a@x>, b@y`, lower case. shortcut: a comma inside a quoted display name
/// splits the entry, which only costs the (address-less) fragment, not the address.
pub fn addresses(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            let address = match (part.rfind('<'), part.rfind('>')) {
                (Some(open), Some(close)) if open < close => &part[open + 1..close],
                _ => part,
            };
            address.trim().contains('@').then(|| address.trim().to_lowercase())
        })
        .collect()
}

/// `bytes` in `charset`. shortcut: Latin-1 and UTF-8 only, anything else is read as UTF-8 and may show `?`;
/// upgrade with encoding_rs when a mail in another charset matters.
fn decode_charset(bytes: &[u8], charset: Option<&str>) -> String {
    match charset.map(str::to_lowercase).as_deref() {
        Some("iso-8859-1" | "latin1" | "windows-1252") => bytes.iter().map(|b| *b as char).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// `=XX` escapes decoded; a malformed one stays as it is.
fn qp_bytes(text: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16);
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let pair = text.get(i + 1).zip(text.get(i + 2)).and_then(|(a, b)| Some(hex(*a)? * 16 + hex(*b)?));
        match (text[i], pair) {
            (b'=', Some(byte)) => {
                out.push(byte as u8);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    out
}

/// A quoted-printable body: soft line breaks (`=` at the end of a line) joined, escapes decoded.
fn quoted_printable(body: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in body.split('\n') {
        let line = line.trim_end_matches([' ', '\t']);
        match line.strip_suffix('=') {
            Some(soft) => out.extend(qp_bytes(soft.as_bytes())),
            None => {
                out.extend(qp_bytes(line.as_bytes()));
                out.push(b'\n');
            }
        }
    }
    out
}

fn base64_bytes(text: &str) -> Option<Vec<u8>> {
    let clean: String = text.chars().filter(|c| !c.is_whitespace() && *c != '=').collect();
    base64::engine::general_purpose::STANDARD_NO_PAD.decode(clean).ok()
}

/// The text of one RFC 2047 word `=?charset?B|Q?text?=`.
fn word(charset: &str, encoding: &str, text: &str) -> Option<String> {
    let bytes = match encoding.to_ascii_lowercase().as_str() {
        "b" => base64_bytes(text)?,
        "q" => qp_bytes(text.replace('_', " ").as_bytes()),
        _ => return None,
    };
    Some(decode_charset(&bytes, Some(charset)))
}

/// A header value with its RFC 2047 encoded words decoded; whitespace between two of them is dropped.
pub fn decode_words(value: &str) -> String {
    let (mut out, mut rest, mut after_word) = (String::new(), value, false);
    while let Some(start) = rest.find("=?") {
        let (before, inner) = (&rest[..start], &rest[start + 2..]);
        let parsed = inner.split_once('?').and_then(|(charset, tail)| {
            let (encoding, tail) = tail.split_once('?')?;
            let (text, tail) = tail.split_once("?=")?;
            Some((word(charset, encoding, text)?, tail))
        });
        match parsed {
            Some((decoded, tail)) => {
                if !(after_word && before.trim().is_empty()) {
                    out.push_str(before);
                }
                out.push_str(&decoded);
                (rest, after_word) = (tail, true);
            }
            None => {
                out.push_str(&rest[..start + 2]);
                (rest, after_word) = (inner, false);
            }
        }
    }
    out + rest
}

/// The parts between `--boundary` lines.
fn parts<'a>(body: &'a str, boundary: &str) -> Vec<&'a str> {
    let marker = format!("--{boundary}");
    let mut out = Vec::new();
    let Some(first) = body.find(&marker) else { return out };
    let mut rest = &body[first + marker.len()..];
    while !rest.starts_with("--") {
        let part = rest.split_once('\n').map_or("", |(_, part)| part);
        match part.find(&format!("\n{marker}")) {
            Some(end) => {
                out.push(&part[..end]);
                rest = &part[end + 1 + marker.len()..];
            }
            None => {
                out.push(part);
                break;
            }
        }
    }
    out
}

fn walk(all: &Headers, body: &str, out: &mut Parsed, depth: usize) {
    let content_type = header(all, "content-type").unwrap_or("text/plain");
    let kind = content_type.split(';').next().unwrap_or_default().trim().to_lowercase();
    if kind.starts_with("multipart/") {
        if let Some(boundary) = param(content_type, "boundary").filter(|_| depth < MAX_DEPTH) {
            for part in parts(body, &boundary) {
                let (head, inner) = split(part);
                walk(&headers(head), inner, out, depth + 1);
            }
        }
        return;
    }
    if header(all, "content-disposition").is_some_and(|d| d.trim_start().to_lowercase().starts_with("attachment")) {
        return;
    }
    let encoding = header(all, "content-transfer-encoding").map(|v| v.trim().to_lowercase());
    let bytes = match encoding.as_deref() {
        Some("base64") => base64_bytes(body).unwrap_or_else(|| body.as_bytes().to_vec()),
        Some("quoted-printable") => quoted_printable(body),
        _ => body.as_bytes().to_vec(),
    };
    let text = decode_charset(&bytes, param(content_type, "charset").as_deref()).trim_end().to_string();
    match kind.as_str() {
        "text/plain" if out.text.is_empty() => out.text = text,
        "text/html" if out.html.is_empty() => out.html = text,
        _ => {}
    }
}

/// The mail in `raw` (the message as it came over SMTP, after dot-unstuffing).
/// shortcut: the message is read as lossy UTF-8 first, so a raw 8-bit Latin-1 body shows `�`; encoded bodies
/// (the usual case) are not affected.
pub fn parse(raw: &[u8]) -> Parsed {
    let message = String::from_utf8_lossy(raw).replace("\r\n", "\n");
    let (head, body) = split(&message);
    let all = headers(head);
    let mut parsed = Parsed {
        from: header(&all, "from").map(addresses).and_then(|a| a.into_iter().next()).unwrap_or_default(),
        to: header(&all, "to").map(addresses).unwrap_or_default(),
        subject: decode_words(header(&all, "subject").unwrap_or_default()),
        ..Parsed::default()
    };
    walk(&all, body, &mut parsed, 0);
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_mail_has_its_headers_and_text() {
        let mail = parse(b"From: Shop <noreply@shop.test>\r\nTo: A <A@x.test>, b@x.test\r\nSubject: Hi\r\n there\r\n\r\nBody line\r\n");
        assert_eq!(
            mail,
            Parsed {
                from: "noreply@shop.test".into(),
                to: vec!["a@x.test".into(), "b@x.test".into()],
                subject: "Hi there".into(),
                text: "Body line".into(),
                html: String::new(),
            }
        );
    }

    #[test]
    fn quoted_printable_joins_soft_breaks_and_decodes_charsets() {
        let raw = "Content-Type: text/plain; charset=iso-8859-1\nContent-Transfer-Encoding: quoted-printable\n\nCaf=E9 code =\n123456 = ok=\n";
        assert_eq!(parse(raw.as_bytes()).text, "Café code 123456 = ok");
        let utf8 = "Content-Transfer-Encoding: quoted-printable\n\nna=C3=AFve=3D\n";
        assert_eq!(parse(utf8.as_bytes()).text, "naïve=");
    }

    #[test]
    fn base64_bodies_are_decoded_with_or_without_padding() {
        let raw = "Content-Type: text/html; charset=utf-8\nContent-Transfer-Encoding: base64\n\nPGI+aGk8L2I+\n";
        assert_eq!(parse(raw.as_bytes()).html, "<b>hi</b>");
        assert_eq!(parse(b"Content-Transfer-Encoding: base64\n\naGk\n").text, "hi");
    }

    #[test]
    fn multipart_alternative_gives_both_bodies_and_attachments_are_skipped() {
        let raw = "Subject: x\nContent-Type: multipart/mixed; boundary=\"outer\"\n\nignored\n--outer\n\
            Content-Type: multipart/alternative; boundary=inner\n\n--inner\nContent-Type: text/plain\n\nplain code 1234\n\
            --inner\nContent-Type: text/html\nContent-Transfer-Encoding: quoted-printable\n\n<a href=3D\"https://x.test/v\">go</a>\n\
            --inner--\n--outer\nContent-Type: text/plain\nContent-Disposition: attachment; filename=a.txt\n\nsecret\n--outer--\n";
        let mail = parse(raw.as_bytes());
        assert_eq!(mail.text, "plain code 1234");
        assert_eq!(mail.html, "<a href=\"https://x.test/v\">go</a>");
    }

    #[test]
    fn encoded_words_in_a_subject_are_decoded_and_joined() {
        assert_eq!(decode_words("=?UTF-8?B?Q2Fmw6k=?= =?UTF-8?Q?_code_=31=32?= tail"), "Café code 12 tail");
        assert_eq!(decode_words("=?iso-8859-1?Q?Caf=E9?="), "Café");
        assert_eq!(decode_words("plain =? broken"), "plain =? broken");
        assert_eq!(parse(b"Subject: =?utf-8?q?Your_code?=\n\nx").subject, "Your code");
    }
}
