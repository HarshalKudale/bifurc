//! The version handshake: major mismatch refuses, minor mismatch connects and degrades.

/// Bumped on any wire-incompatible change.
pub const PROTOCOL_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionCompatibility {
    Exact,
    Compatible,
    Incompatible,
}

/// `major.minor.patch`; a missing or non-numeric part is `0`, a numeric prefix (`1-beta`) counts.
pub fn parse_version(version: &str) -> ParsedVersion {
    let mut parts = version.split('.').map(|part| {
        let digits: String = part.trim().chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u64>().unwrap_or(0)
    });
    ParsedVersion {
        major: parts.next().unwrap_or(0),
        minor: parts.next().unwrap_or(0),
        patch: parts.next().unwrap_or(0),
    }
}

pub fn check_version_compatibility(client: &str, engine: &str) -> VersionCompatibility {
    let (client, engine) = (parse_version(client), parse_version(engine));
    if client.major != engine.major {
        VersionCompatibility::Incompatible
    } else if client.minor != engine.minor {
        VersionCompatibility::Compatible
    } else {
        VersionCompatibility::Exact
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_mismatch_is_incompatible() {
        assert_eq!(check_version_compatibility("1.0.0", "1.0.0"), VersionCompatibility::Exact);
        assert_eq!(check_version_compatibility("1.2.0", "1.0.0"), VersionCompatibility::Compatible);
        assert_eq!(check_version_compatibility("2.0.0", "1.0.0"), VersionCompatibility::Incompatible);
    }

    #[test]
    fn parts_default_to_zero() {
        assert_eq!(parse_version("1.x"), ParsedVersion { major: 1, minor: 0, patch: 0 });
        assert_eq!(parse_version("3.4.5-beta"), ParsedVersion { major: 3, minor: 4, patch: 5 });
    }
}
