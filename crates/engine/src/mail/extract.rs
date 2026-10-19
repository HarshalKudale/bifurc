//! What an agent wants out of a verification mail: the one-time code and the link.

/// Words that make a number right after them the code.
const KEYWORDS: [&str; 8] = ["code", "otp", "pin", "passcode", "password", "verification", "verify", "token"];

/// The text of an HTML mail: tags (and style and script blocks) dropped, common entities decoded.
pub fn strip_tags(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let (mut out, mut i) = (String::new(), 0);
    while i < html.len() {
        if html.as_bytes()[i] == b'<' {
            if let Some(tag) = ["style", "script"].iter().find(|tag| lower[i + 1..].starts_with(**tag)) {
                i = lower[i..].find(&format!("</{tag}")).map_or(html.len(), |end| i + end);
            }
            i = html[i..].find('>').map_or(html.len(), |end| i + end + 1);
            out.push(' ');
        } else {
            let ch = html[i..].chars().next().unwrap_or_default();
            out.push(ch);
            i += ch.len_utf8().max(1);
        }
    }
    [("&nbsp;", " "), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&amp;", "&")]
        .iter()
        .fold(out, |text, (entity, plain)| text.replace(entity, plain))
}

/// Whether the digits `chars[start..end]` stand alone: not inside a word, a date, a price, a phone number.
fn standalone(chars: &[char], start: usize, end: usize) -> bool {
    let (prev, next) = (start.checked_sub(1).map(|at| chars[at]), chars.get(end).copied());
    let glued = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || "_#$€£+".contains(c));
    let joins = |sep: Option<char>, beyond: Option<char>| {
        sep.is_some_and(|c| "-.,/:".contains(c)) && beyond.is_some_and(|c| c.is_ascii_digit())
    };
    let before = start.checked_sub(2).map(|at| chars[at]);
    !glued(prev) && !glued(next) && !joins(prev, before) && !joins(next, chars.get(end + 1).copied())
}

/// The code in `text`: a 4 to 8 digit number on its own. One after a word like "code" wins over an earlier
/// one; else the first that is not a year (a year with no such word is a date, not a code).
fn code_in(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let (mut plain, mut i) = (None, 0);
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let run: String = chars[start..i].iter().collect();
        if !(4..=8).contains(&run.len()) || !standalone(&chars, start, i) {
            continue;
        }
        let before: String = chars[start.saturating_sub(40)..start].iter().collect::<String>().to_lowercase();
        if KEYWORDS.iter().any(|word| before.contains(word)) {
            return Some(run);
        }
        let year = run.len() == 4 && (1900..=2100).contains(&run.parse::<u32>().unwrap_or(0));
        if plain.is_none() && !year {
            plain = Some(run);
        }
    }
    plain
}

/// The code of a mail: from the subject, else the text, else the html without its tags.
/// shortcut: a phone number written with spaces (`+49 1512 3456789`) can still read as a code.
pub fn otp(subject: &str, text: &str, html: &str) -> Option<String> {
    code_in(subject).or_else(|| code_in(text)).or_else(|| code_in(&strip_tags(html)))
}

/// Every http(s) link in `text`, in order, without trailing punctuation.
fn links_in(text: &str) -> Vec<String> {
    let (mut out, mut rest) = (Vec::new(), text);
    while let Some(at) = rest.find("http") {
        let tail = &rest[at..];
        if !(tail.starts_with("http://") || tail.starts_with("https://")) {
            rest = &tail[4..];
            continue;
        }
        let end = tail.find(|c: char| c.is_whitespace() || "\"'<>".contains(c)).unwrap_or(tail.len());
        let link = tail[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']']).replace("&amp;", "&");
        if link.len() > "https://".len() {
            out.push(link);
        }
        rest = &tail[end..];
    }
    out
}

/// The first link of the text part, else of the html (where it is an `href`); the first that contains `matching`
/// when that is given.
pub fn link(text: &str, html: &str, matching: Option<&str>) -> Option<String> {
    let mut all = links_in(text);
    all.extend(links_in(html));
    all.into_iter().find(|link| matching.is_none_or(|part| link.contains(part)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_is_the_first_standalone_number_of_four_to_eight_digits() {
        assert_eq!(otp("Your code is 483920", "", ""), Some("483920".into()));
        assert_eq!(otp("Welcome", "Hi!\nUse 7391 to sign in.", ""), Some("7391".into()));
        assert_eq!(otp("Welcome", "", "<p>Code:<b>55 </b><b>918273</b></p>"), Some("918273".into()));
        assert_eq!(otp("Hello", "", "<style>.a{width:1234}</style><p>PIN 6512</p>"), Some("6512".into()));
    }

    #[test]
    fn numbers_that_are_not_codes_are_not_taken() {
        assert_eq!(otp("", "Sent on October 17, 2026. Your verification code is 483920.", ""), Some("483920".into()));
        assert_eq!(otp("", "Sent on October 17, 2026.", ""), None);
        assert_eq!(otp("", "On 2026-10-17 and 17/10/2026 and 12:30", ""), None);
        assert_eq!(otp("", "Call 555-123-4567 or 1-800-555-0199", ""), None);
        assert_eq!(otp("", "Order 123456789012, ref AB123456, #123456, $1234.50, 1,234,567", ""), None);
        assert_eq!(otp("", "tracking ab-123456x and 12 345", ""), None);
        assert_eq!(otp("", "Your code is 2026", ""), Some("2026".into()));
    }

    #[test]
    fn the_link_is_the_first_one_or_the_first_that_contains_the_filter() {
        let text = "See https://shop.test/terms. Verify: https://shop.test/verify?t=abc&u=1, thanks";
        assert_eq!(link(text, "", None), Some("https://shop.test/terms".into()));
        assert_eq!(link(text, "", Some("/verify")), Some("https://shop.test/verify?t=abc&u=1".into()));
        assert_eq!(link(text, "", Some("/nope")), None);
        let html = "<a href=\"http://x.test/v?a=1&amp;b=2\">go</a>";
        assert_eq!(link("", html, None), Some("http://x.test/v?a=1&b=2".into()));
        assert_eq!(link("no links, just httpx and http", "", None), None);
    }

    #[test]
    fn tags_are_dropped_with_style_blocks_and_entities_decoded() {
        let text = strip_tags("<html><style>p{}</style><p>a&nbsp;&amp;<br/>b</p><script>x()</script>");
        assert_eq!(text.split_whitespace().collect::<Vec<_>>(), ["a", "&", "b"]);
    }
}
