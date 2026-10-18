//! Every sentence the window shows comes from here, by key, in the language the user chose.
//!
//! A language is a folder `locales/<code>/` holding one flat JSON file per area of the app (`onboarding.json`,
//! `settings.json`, ...). A file's name is the first part of its keys: `"stage.terms.title"` in
//! `onboarding.json` is asked for as `t("onboarding.stage.terms.title")`. Text with a variable part keeps a
//! `{name}` placeholder and is asked for with [`t_with`]. A key a language lacks falls back to English; one
//! English lacks is shown as the key itself, so a gap is visible rather than blank.
//!
//! Adding a language: copy `locales/en/` to `locales/<code>/`, translate the values, and add one `language!`
//! line to [`LANGUAGES`]. Adding an area: add `<name>.json` to every language and the name to `AREAS`.

use std::{
    collections::HashMap,
    fmt::Display,
    sync::{OnceLock, RwLock},
};

/// The areas every language has a file for, in `locales/<code>/<area>.json`.
macro_rules! language {
    ($code:literal, $name:literal) => {
        Language {
            code: $code,
            name: $name,
            files: &[
                ("app", include_str!(concat!("../locales/", $code, "/app.json"))),
                ("audit", include_str!(concat!("../locales/", $code, "/audit.json"))),
                ("capture", include_str!(concat!("../locales/", $code, "/capture.json"))),
                ("components", include_str!(concat!("../locales/", $code, "/components.json"))),
                ("errors", include_str!(concat!("../locales/", $code, "/errors.json"))),
                ("environments", include_str!(concat!("../locales/", $code, "/environments.json"))),
                ("flows", include_str!(concat!("../locales/", $code, "/flows.json"))),
                ("healthbar", include_str!(concat!("../locales/", $code, "/healthbar.json"))),
                ("licenses", include_str!(concat!("../locales/", $code, "/licenses.json"))),
                ("mail", include_str!(concat!("../locales/", $code, "/mail.json"))),
                ("mappings", include_str!(concat!("../locales/", $code, "/mappings.json"))),
                ("mocks", include_str!(concat!("../locales/", $code, "/mocks.json"))),
                ("model", include_str!(concat!("../locales/", $code, "/model.json"))),
                ("onboarding", include_str!(concat!("../locales/", $code, "/onboarding.json"))),
                ("proxy_rules", include_str!(concat!("../locales/", $code, "/proxy_rules.json"))),
                ("requests", include_str!(concat!("../locales/", $code, "/requests.json"))),
                ("services", include_str!(concat!("../locales/", $code, "/services.json"))),
                ("session", include_str!(concat!("../locales/", $code, "/session.json"))),
                ("settings", include_str!(concat!("../locales/", $code, "/settings.json"))),
                ("shell", include_str!(concat!("../locales/", $code, "/shell.json"))),
                ("vaults", include_str!(concat!("../locales/", $code, "/vaults.json"))),
                ("workspace", include_str!(concat!("../locales/", $code, "/workspace.json"))),
                ("workspace_setup", include_str!(concat!("../locales/", $code, "/workspace_setup.json"))),
            ],
        }
    };
}

pub struct Language {
    /// What the preferences store, and the folder under `locales/`.
    pub code: &'static str,
    /// The language's name in itself, as the picker lists it.
    pub name: &'static str,
    files: &'static [(&'static str, &'static str)],
}

/// The first one is the fallback for keys another language lacks.
pub const LANGUAGES: &[Language] = &[language!("en", "English"), language!("de", "Deutsch"), language!("ja", "日本語")];

/// Values live for the whole run (leaked once when the bundles load), so `t` can hand out `&'static str` and
/// the helpers that already return one keep their signatures.
type Bundle = HashMap<String, &'static str>;

fn bundle(language: &Language) -> Bundle {
    let mut keys = Bundle::new();
    for (area, text) in language.files {
        let entries: HashMap<String, String> = serde_json::from_str(text)
            .unwrap_or_else(|error| panic!("locales/{}/{area}.json is not a flat string map: {error}", language.code));
        keys.extend(
            entries.into_iter().map(|(key, value)| (format!("{area}.{key}"), &*Box::leak(value.into_boxed_str()))),
        );
    }
    keys
}

fn bundles() -> &'static Vec<Bundle> {
    static BUNDLES: OnceLock<Vec<Bundle>> = OnceLock::new();
    BUNDLES.get_or_init(|| LANGUAGES.iter().map(bundle).collect())
}

fn current_index() -> &'static RwLock<usize> {
    static CURRENT: OnceLock<RwLock<usize>> = OnceLock::new();
    CURRENT.get_or_init(|| RwLock::new(0))
}

/// Switches the language the next frame is drawn in. An unknown code leaves it as it was.
pub fn set_language(code: &str) {
    if let Some(index) = LANGUAGES.iter().position(|language| language.code == code) {
        *current_index().write().unwrap() = index;
    }
}

pub fn language() -> &'static Language {
    &LANGUAGES[*current_index().read().unwrap()]
}

/// The text for `key` in the current language.
pub fn t(key: &str) -> &'static str {
    let bundles = bundles();
    bundles[*current_index().read().unwrap()].get(key).or_else(|| bundles[0].get(key)).copied().unwrap_or_else(|| {
        // Kept so a key that is asked for every frame leaks once, not every time.
        static MISSING: OnceLock<RwLock<HashMap<String, &'static str>>> = OnceLock::new();
        let missing = MISSING.get_or_init(Default::default);
        if let Some(shown) = missing.read().unwrap().get(key) {
            return shown;
        }
        let shown: &'static str = Box::leak(key.to_owned().into_boxed_str());
        missing.write().unwrap().insert(key.to_owned(), shown);
        shown
    })
}

/// Whether the current language, or English, has text for `key`.
pub fn has(key: &str) -> bool {
    let bundles = bundles();
    bundles[*current_index().read().unwrap()].contains_key(key) || bundles[0].contains_key(key)
}

/// The text for `key` with each `{name}` replaced by its value.
pub fn t_with(key: &str, values: &[(&str, &dyn Display)]) -> String {
    values
        .iter()
        .fold(t(key).to_owned(), |text, (name, value)| text.replace(&format!("{{{name}}}"), &value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_parses_and_has_every_key_english_has() {
        let bundles = bundles();
        for (language, keys) in LANGUAGES.iter().zip(bundles) {
            for key in bundles[0].keys() {
                assert!(keys.contains_key(key), "{} lacks {key}", language.code);
            }
        }
    }

    fn rust_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    /// Any quoted `area.some.key` in the source must be a key English has: a typo shows as the key on screen.
    #[test]
    fn every_key_the_source_asks_for_exists() {
        let areas: Vec<&str> = LANGUAGES[0].files.iter().map(|(area, _)| *area).collect();
        let mut files = Vec::new();
        rust_files(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        let mut missing = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for piece in text.split('"') {
                let segments: Vec<&str> = piece.split('.').collect();
                let is_key = segments.len() >= 3
                    && areas.contains(&segments[0])
                    && segments.iter().all(|s| {
                        !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    });
                if is_key && !bundles()[0].contains_key(piece) {
                    missing.push(format!("{} in {}", piece, file.display()));
                }
            }
        }
        assert!(missing.is_empty(), "keys with no English text:\n{}", missing.join("\n"));
    }

    #[test]
    fn has_tells_a_known_key_from_an_unknown_one() {
        assert!(has("errors.request.send.refused"));
        assert!(!has(&format!("errors.{}", "no.such.key")));
    }

    #[test]
    fn a_missing_key_shows_as_itself_and_placeholders_are_filled() {
        assert_eq!(t("no.such.key"), "no.such.key");
        assert_eq!(t_with("no.such.{name}", &[("name", &"key")]), "no.such.key");
    }
}
