//! The features that are switched on or off as a whole. Each is asked about through one function here, so
//! that what decides it (a constant today, what an account is entitled to later) changes in this file and
//! nowhere else.

/// Team sync as this build ships it.
const TEAM_SYNC: bool = true;

#[cfg(test)]
static TEST_TEAM_SYNC: parking_lot::Mutex<Option<bool>> = parking_lot::Mutex::new(None);

/// Switch team sync for the tests that follow, or back to the build's own with `None`. Tests take turns
/// through the scratch data root.
#[cfg(test)]
pub fn set_test_team_sync(on: Option<bool>) {
    *TEST_TEAM_SYNC.lock() = on;
}

/// What an environment variable makes of a switch: a word for off or for on, else the default.
fn switched(value: Option<&str>, default: bool) -> bool {
    match value.map(|value| value.trim().to_ascii_lowercase()).as_deref() {
        Some("0" | "false" | "off" | "no") => false,
        Some("1" | "true" | "on" | "yes") => true,
        _ => default,
    }
}

/// Team sync: a workspace is used by everyone GitHub gives access to its repository, each with the access
/// they were given there (someone who may not push has it read-only). Off, a workspace is imported and
/// synced by the owner of its repository alone. `BIFURC_TEAM_SYNC` switches it for one run of the engine.
pub fn team_sync() -> bool {
    #[cfg(test)]
    if let Some(on) = *TEST_TEAM_SYNC.lock() {
        return on;
    }
    switched(std::env::var("BIFURC_TEAM_SYNC").ok().as_deref(), TEAM_SYNC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_follows_the_environment_and_else_the_default() {
        assert!(!switched(Some("0"), true));
        assert!(!switched(Some(" Off "), true));
        assert!(switched(Some("yes"), false));
        assert!(switched(None, true) && !switched(None, false));
        assert!(switched(Some("maybe"), true) && !switched(Some(""), false));
    }
}
