//! `{{random.*}}` token resolution: the engine's one implementation of the generators.

use rand::Rng;

use crate::utils::id::now_ms;

const FIRST_NAMES: [&str; 26] = [
    "Alice", "Bob", "Carol", "David", "Eve", "Frank", "Grace", "Hank", "Iris", "Jack", "Karen", "Leo", "Maria", "Nate",
    "Olivia", "Paul", "Quinn", "Rachel", "Sam", "Tina", "Uma", "Victor", "Wendy", "Xander", "Yara", "Zoe",
];
const LAST_NAMES: [&str; 26] = [
    "Adams",
    "Baker",
    "Clark",
    "Davis",
    "Evans",
    "Foster",
    "Garcia",
    "Harris",
    "Ingram",
    "Jones",
    "Klein",
    "Lewis",
    "Moore",
    "Nelson",
    "Owen",
    "Parker",
    "Quinn",
    "Reed",
    "Smith",
    "Taylor",
    "Turner",
    "Underwood",
    "Vance",
    "Walker",
    "Young",
    "Zhang",
];
const CITIES: [&str; 20] = [
    "New York",
    "Los Angeles",
    "Chicago",
    "Houston",
    "Phoenix",
    "Portland",
    "Seattle",
    "Denver",
    "Austin",
    "Boston",
    "Miami",
    "Atlanta",
    "Detroit",
    "Nashville",
    "Orlando",
    "Minneapolis",
    "Dallas",
    "San Diego",
    "Oakland",
    "Raleigh",
];
const COUNTRIES: [&str; 20] = [
    "United States",
    "Germany",
    "France",
    "Canada",
    "Japan",
    "Brazil",
    "Australia",
    "India",
    "United Kingdom",
    "Mexico",
    "Spain",
    "Italy",
    "South Korea",
    "Netherlands",
    "Sweden",
    "Norway",
    "Finland",
    "Denmark",
    "New Zealand",
    "Argentina",
];
const COMPANIES: [&str; 14] = [
    "Acme Corp",
    "Globex",
    "Initech",
    "Umbrella Inc",
    "Dunder Mifflin",
    "Pied Piper",
    "Hooli",
    "Vandelay Industries",
    "Soylent Corp",
    "Wayne Enterprises",
    "Stark Industries",
    "Oscorp",
    "LexCorp",
    "Veridian Dynamics",
];
const JOB_TITLES: [&str; 12] = [
    "Software Engineer",
    "Product Manager",
    "Designer",
    "Data Scientist",
    "DevOps Engineer",
    "QA Engineer",
    "Solutions Architect",
    "Frontend Developer",
    "Backend Developer",
    "Full Stack Developer",
    "CTO",
    "Engineering Manager",
];
const WORDS: [&str; 18] = [
    "cerulean",
    "eloquent",
    "nomadic",
    "fleeting",
    "luminous",
    "serene",
    "resilient",
    "ephemeral",
    "jovial",
    "tenacious",
    "vibrant",
    "wistful",
    "fervent",
    "candid",
    "pensive",
    "effervescent",
    "quixotic",
    "serendipitous",
];
const CURRENCIES: [&str; 20] = [
    "USD", "EUR", "GBP", "JPY", "CAD", "AUD", "CHF", "CNY", "INR", "BRL", "MXN", "KRW", "SGD", "HKD", "NOK", "SEK",
    "DKK", "PLN", "CZK", "HUF",
];
const LOCALES: [&str; 15] = [
    "en-US", "en-GB", "de-DE", "fr-FR", "es-ES", "ja-JP", "zh-CN", "pt-BR", "ko-KR", "ru-RU", "it-IT", "nl-NL",
    "sv-SE", "pl-PL", "ar-SA",
];
const SENTENCES: [&str; 5] = [
    "The quick brown fox jumps over the lazy dog.",
    "Pack my box with five dozen liquor jugs.",
    "How vexingly quick daft zebras jump!",
    "The five boxing wizards jump quickly.",
    "Sphinx of black quartz, judge my vow.",
];
const PARAGRAPHS: [&str; 3] = [
    "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore \
     magna aliqua.",
    "Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.",
    "Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur.",
];
const URL_PATHS: [&str; 10] =
    ["/products", "/users", "/orders", "/items", "/posts", "/articles", "/categories", "/tags", "/search", "/settings"];
const URL_DOMAINS: [&str; 5] = ["example.com", "test.io", "api.dev", "sample.org", "demo.net"];
const ZIP_CODES: [&str; 15] = [
    "97201", "10001", "90210", "60601", "77001", "30301", "80201", "98101", "85001", "33101", "02101", "55401",
    "48201", "37201", "32801",
];
const STREET_SUFFIXES: [&str; 7] = ["St", "Ave", "Blvd", "Rd", "Ln", "Dr", "Way"];

/// Every key `generate_random` implements.
pub const RANDOM_KEYS: [&str; 31] = [
    "random.name",
    "random.firstName",
    "random.lastName",
    "random.email",
    "random.username",
    "random.phone",
    "random.avatar",
    "random.int",
    "random.float",
    "random.boolean",
    "random.price",
    "random.uuid",
    "random.id",
    "random.url",
    "random.ip",
    "random.ipv6",
    "random.color",
    "random.date",
    "random.timestamp",
    "random.isoDateTime",
    "random.city",
    "random.country",
    "random.zipCode",
    "random.address",
    "random.word",
    "random.sentence",
    "random.paragraph",
    "random.company",
    "random.jobTitle",
    "random.currency",
    "random.locale",
];

fn pick<'a>(items: &[&'a str]) -> &'a str {
    items[rand::rng().random_range(0..items.len())]
}

fn rand_int(min: i64, max: i64) -> i64 {
    rand::rng().random_range(min..=max)
}

fn hex_str(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len).map(|_| char::from_digit(rng.random_range(0..16), 16).expect("a hex digit")).collect()
}

/// `Date#toISOString()` for a time in milliseconds since the epoch.
fn iso_string(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let in_day = ms.rem_euclid(86_400_000);
    // Civil date from a day count (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000
    )
}

fn random_past_iso() -> String {
    iso_string(now_ms() - rand_int(0, 365 * 24 * 60 * 60 * 1000))
}

/// One generated value for `key` (`random.name`, …). An unknown key comes back as the literal `{{key}}`.
pub fn generate_random(key: &str) -> String {
    match key {
        "random.name" => format!("{} {}", pick(&FIRST_NAMES), pick(&LAST_NAMES)),
        "random.firstName" => pick(&FIRST_NAMES).to_string(),
        "random.lastName" => pick(&LAST_NAMES).to_string(),
        "random.email" => {
            let first = pick(&FIRST_NAMES).to_lowercase();
            let last = pick(&LAST_NAMES).to_lowercase();
            format!("{first}.{last}{}@example.com", rand_int(1, 99))
        }
        "random.username" => format!("{}_{}", pick(&FIRST_NAMES).to_lowercase(), rand_int(1, 999)),
        "random.phone" => format!("+1-{}-{}-{}", rand_int(200, 999), rand_int(200, 999), rand_int(1000, 9999)),
        "random.avatar" => format!("https://i.pravatar.cc/150?u={}", hex_str(8)),
        "random.int" => rand_int(0, 9999).to_string(),
        "random.float" => format!("{:.4}", rand::rng().random::<f64>()),
        "random.boolean" => (if rand::rng().random::<f64>() < 0.5 { "true" } else { "false" }).to_string(),
        "random.price" => format!("{}.{:02}", rand_int(1, 999), rand_int(0, 99)),
        "random.uuid" => {
            let variant = char::from_digit(rand_int(8, 11) as u32, 16).expect("a hex digit");
            format!("{}-{}-4{}-{variant}{}-{}", hex_str(8), hex_str(4), hex_str(3), hex_str(3), hex_str(12))
        }
        "random.id" => hex_str(8),
        "random.url" => format!("https://{}{}", pick(&URL_DOMAINS), pick(&URL_PATHS)),
        "random.ip" => format!("{}.{}.{}.{}", rand_int(1, 254), rand_int(0, 255), rand_int(0, 255), rand_int(1, 254)),
        "random.ipv6" => (0..8).map(|_| hex_str(4)).collect::<Vec<_>>().join(":"),
        "random.color" => format!("#{}", hex_str(6)),
        "random.date" => random_past_iso()[..10].to_string(),
        "random.timestamp" => (now_ms() / 1000 - rand_int(0, 86_400 * 365)).to_string(),
        "random.isoDateTime" => random_past_iso(),
        "random.city" => pick(&CITIES).to_string(),
        "random.country" => pick(&COUNTRIES).to_string(),
        "random.zipCode" => pick(&ZIP_CODES).to_string(),
        "random.address" => {
            let initial = pick(&WORDS).chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or_default();
            format!("{} {initial}{} {}", rand_int(1, 9999), &pick(&WORDS)[1..], pick(&STREET_SUFFIXES))
        }
        "random.word" => pick(&WORDS).to_string(),
        "random.sentence" => pick(&SENTENCES).to_string(),
        "random.paragraph" => pick(&PARAGRAPHS).to_string(),
        "random.company" => pick(&COMPANIES).to_string(),
        "random.jobTitle" => pick(&JOB_TITLES).to_string(),
        "random.currency" => pick(&CURRENCIES).to_string(),
        "random.locale" => pick(&LOCALES).to_string(),
        _ => format!("{{{{{key}}}}}"),
    }
}

/// Expand every `{{random.*}}` generator in `text`.
pub fn resolve_randomizers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{random.") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let key_len = after
            .char_indices()
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '.'))
            .map_or(after.len(), |(i, _)| i);
        // The key is `random.` followed by word characters only (`\w+`), then `}}`.
        let key = &after[..key_len];
        let word = &key["random.".len()..];
        if !word.is_empty() && !word.contains('.') && after[key_len..].starts_with("}}") {
            out.push_str(&generate_random(key));
            rest = &after[key_len + 2..];
        } else {
            out.push_str("{{");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_digits(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
    }

    #[test]
    fn every_key_produces_a_value_of_the_right_shape() {
        for key in RANDOM_KEYS {
            for _ in 0..20 {
                let value = generate_random(key);
                assert!(!value.is_empty(), "{key}");
                assert_ne!(value, format!("{{{{{key}}}}}"), "{key} is not implemented");
                match key {
                    "random.email" => {
                        assert!(value.ends_with("@example.com") && value.contains('.'), "{value}");
                    }
                    "random.username" => {
                        let (name, n) = value.split_once('_').unwrap();
                        assert!(name.chars().all(|c| c.is_ascii_lowercase()));
                        assert!((1..=999).contains(&n.parse::<i64>().unwrap()));
                    }
                    "random.phone" => {
                        let parts: Vec<&str> = value.split('-').collect();
                        assert_eq!(parts.len(), 4);
                        assert_eq!(parts[0], "+1");
                        assert!(parts[1..].iter().all(|p| all_digits(p)));
                    }
                    "random.int" => assert!((0..=9999).contains(&value.parse::<i64>().unwrap())),
                    "random.float" => {
                        let (whole, frac) = value.split_once('.').unwrap();
                        assert_eq!(whole, "0");
                        assert_eq!(frac.len(), 4);
                    }
                    "random.boolean" => assert!(value == "true" || value == "false"),
                    "random.price" => {
                        let (whole, cents) = value.split_once('.').unwrap();
                        assert!(all_digits(whole));
                        assert_eq!(cents.len(), 2);
                    }
                    "random.uuid" => {
                        let parts: Vec<&str> = value.split('-').collect();
                        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
                        assert!(parts[2].starts_with('4'));
                        assert!("89ab".contains(parts[3].chars().next().unwrap()));
                    }
                    "random.id" => assert_eq!(value.len(), 8),
                    "random.url" => assert!(value.starts_with("https://")),
                    "random.ip" => assert_eq!(value.split('.').count(), 4),
                    "random.ipv6" => assert_eq!(value.split(':').count(), 8),
                    "random.color" => assert!(value.len() == 7 && value.starts_with('#')),
                    "random.date" => {
                        assert_eq!(value.len(), 10);
                        assert_eq!(&value[4..5], "-");
                    }
                    "random.timestamp" => assert!(all_digits(&value)),
                    "random.isoDateTime" => {
                        assert_eq!(value.len(), 24);
                        assert!(value.ends_with('Z') && &value[10..11] == "T");
                    }
                    "random.address" => {
                        let words: Vec<&str> = value.split(' ').collect();
                        assert_eq!(words.len(), 3);
                        assert!(all_digits(words[0]));
                        assert!(words[1].chars().next().unwrap().is_ascii_uppercase());
                        assert!(STREET_SUFFIXES.contains(&words[2]));
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn iso_string_matches_javascript() {
        assert_eq!(iso_string(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_string(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(iso_string(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
        assert_eq!(iso_string(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn resolve_replaces_tokens_and_leaves_the_rest() {
        let out = resolve_randomizers("id={{random.int}}&x={{random.nope}}&y={{ random.int }}&z={{random.int");
        assert!(out.starts_with("id="));
        assert!(out.contains("&x={{random.nope}}&y={{ random.int }}&z={{random.int"));
        let id = out["id=".len()..out.find('&').unwrap()].to_string();
        assert!(all_digits(&id));
        assert_eq!(resolve_randomizers(""), "");
        assert_eq!(resolve_randomizers("{{random.}} {{random.a.b}}"), "{{random.}} {{random.a.b}}");
        assert_eq!(resolve_randomizers("{{{random.id}}}").len(), 1 + 8 + 1);
    }
}
