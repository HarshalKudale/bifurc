//! Trusting the engine's root certificate in this computer's trust store.
//!
//! The engine makes and keeps the certificate; whether this machine trusts it is a fact about the machine,
//! which the engine cannot reach, so the client installs it. Each platform has its own tool, and none of the
//! ones used here needs elevation: the certificate goes into the current user's store.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// What the platform's tool says when it cannot be run or refuses.
fn run(command: &mut Command) -> Result<(), String> {
    hide_console(command);
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("{}: {error}", command.get_program().to_string_lossy()))?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(if output.stderr.is_empty() { &output.stdout } else { &output.stderr });
    let last = text.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it failed without saying why");
    Err(format!("{}: {last}", command.get_program().to_string_lossy()))
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_: &mut Command) {}

fn is_installed(program: &str) -> bool {
    let finder = if cfg!(windows) { "where" } else { "which" };
    Command::new(finder)
        .arg(program)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn home() -> PathBuf {
    env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

/// What to tell the user to run when this platform has no tool that installs without elevation.
fn linux_instructions(certificate: &Path) -> String {
    format!(
        "Run these in a terminal:\nsudo cp \"{}\" /usr/local/share/ca-certificates/bifurc-ca.crt\nsudo update-ca-certificates",
        certificate.display()
    )
}

/// Puts `pem` in the current user's trust store. The message says where it went, or why it did not.
pub fn install(pem: &[u8]) -> Result<String, String> {
    let file = env::temp_dir().join(format!("bifurc-ca-{}.pem", std::process::id()));
    fs::write(&file, pem).map_err(|error| format!("Couldn't write the certificate to a file. {error}"))?;
    let outcome = install_file(&file);
    let _ = fs::remove_file(&file);
    outcome
}

fn install_file(file: &Path) -> Result<String, String> {
    if cfg!(windows) {
        run(Command::new("certutil").args(["-addstore", "-user", "Root"]).arg(file))?;
        Ok("Installed in the Windows user certificate store. Restart your browser to pick it up.".to_owned())
    } else if cfg!(target_os = "macos") {
        let keychain = home().join("Library").join("Keychains").join("login.keychain-db");
        run(Command::new("security")
            .args(["add-trusted-cert", "-d", "-r", "trustRoot", "-k"])
            .arg(keychain)
            .arg(file))?;
        Ok("Installed in your login keychain. Restart your browser to pick it up.".to_owned())
    } else if is_installed("trust") {
        run(Command::new("trust").arg("anchor").arg(file))?;
        Ok("Installed in the p11-kit trust store. Restart your browser to pick it up.".to_owned())
    } else {
        Err(format!("No tool that installs it without administrator rights was found. {}", linux_instructions(file)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_name_the_file_and_the_commands() {
        let text = linux_instructions(Path::new("/tmp/ca.pem"));
        assert!(text.contains("/tmp/ca.pem") && text.contains("update-ca-certificates"));
    }

    #[test]
    fn a_missing_program_is_reported_by_name() {
        let error = run(&mut Command::new("bifurc-no-such-program")).unwrap_err();
        assert!(error.contains("bifurc-no-such-program"), "{error}");
    }
}
