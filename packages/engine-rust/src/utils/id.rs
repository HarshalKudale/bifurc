//! Ids and clocks, in the formats the store already holds.

use std::time::{SystemTime, UNIX_EPOCH};

use rand::Rng;

/// Milliseconds since the epoch (`Date.now()`).
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

const BASE36: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

pub fn to_base36(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    while n > 0 {
        digits.push(BASE36[(n % 36) as usize]);
        n /= 36;
    }
    digits.reverse();
    String::from_utf8(digits).expect("base36 digits are ASCII")
}

fn random_base36(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len).map(|_| BASE36[rng.random_range(0..36)] as char).collect()
}

/// An entity id: the time in base 36 followed by five random characters.
pub fn generate_id() -> String {
    format!("{}{}", to_base36(now_ms() as u64), random_base36(5))
}

/// A log entry id: the time in base 36 followed by four random characters.
pub fn mk_id() -> String {
    format!("{}{}", to_base36(now_ms() as u64), random_base36(4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base36_matches_javascript() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(1_700_000_000_000), "loyw3v28");
    }

    #[test]
    fn ids_are_time_then_random() {
        let id = generate_id();
        assert!(id.len() >= 13);
        assert!(id.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()));
        assert_eq!(mk_id().len(), id.len() - 1);
    }
}
