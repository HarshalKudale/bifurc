//! What the CLI knows of the engine protocol: the handshake constants and the command names `call` accepts.
//! Both come from `bifurc-protocol` (`packages/protocol-rust`).

pub use bifurc_protocol::{PROTOCOL_VERSION, crash};
use bifurc_protocol::commands;

/// This client's name in the `hello` handshake.
pub const CLIENT_NAME: &str = "cli";

/// Every engine command, sorted.
pub fn command_names() -> Vec<&'static str> {
    commands::command_names()
}

/// Whether `name` is an engine command.
pub fn is_known_command(name: &str) -> bool {
    commands::is_known_command(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_are_sorted_and_unique() {
        let names = command_names();
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn known_commands_are_recognised() {
        assert!(is_known_command("config.get"));
        assert!(is_known_command("ws.status"));
        assert!(!is_known_command("config.nope"));
        assert!(!is_known_command(""));
    }

    #[test]
    fn the_client_name_is_a_protocol_client() {
        assert!(bifurc_protocol::CLIENT_NAMES.contains(&CLIENT_NAME));
    }
}
