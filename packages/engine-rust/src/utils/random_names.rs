//! Default names for things created without one.

use rand::Rng;

const ADJECTIVES: [&str; 48] = [
    "amber", "blue", "bright", "calm", "clear", "cool", "crisp", "dark", "deep", "dry", "fair", "fast", "fine",
    "fresh", "gold", "green", "high", "keen", "kind", "late", "lean", "light", "long", "loud", "mild", "neat", "new",
    "old", "pale", "plain", "pure", "quick", "rare", "red", "rich", "sharp", "shy", "slim", "slow", "soft", "still",
    "sweet", "tall", "thin", "warm", "wide", "wild", "wise",
];

const NOUNS: [&str; 56] = [
    "bay", "brook", "cave", "cliff", "cloud", "coast", "creek", "crest", "dale", "dawn", "dew", "dune", "elm", "fern",
    "field", "flame", "flint", "fog", "forge", "frost", "gale", "glen", "grove", "hawk", "hill", "lake", "leaf",
    "marsh", "mist", "moon", "oak", "path", "peak", "pine", "pond", "rain", "reed", "ridge", "rock", "sage", "sand",
    "shade", "shore", "sky", "snow", "star", "stone", "storm", "sun", "tide", "vale", "wave", "well", "wind", "wood",
    "wren",
];

/// A two-word name for a workspace created without one (`amber-brook`).
pub fn generate_random_workspace_name() -> String {
    let mut rng = rand::rng();
    format!("{}-{}", ADJECTIVES[rng.random_range(0..ADJECTIVES.len())], NOUNS[rng.random_range(0..NOUNS.len())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_adjective_dash_noun() {
        for _ in 0..50 {
            let name = generate_random_workspace_name();
            let (adjective, noun) = name.split_once('-').unwrap();
            assert!(ADJECTIVES.contains(&adjective) && NOUNS.contains(&noun), "{name}");
        }
    }
}
