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

use crate::i18n::{t, t_with};

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
    let last = text.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or(t("model.trust.no_reason"));
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

fn home() -> PathBuf {
    env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

/// Where each Linux family keeps the roots an administrator adds, and what rebuilds its bundle from them.
const LINUX_STORES: [(&str, &str); 4] = [
    ("/etc/pki/ca-trust/source/anchors/bifurc-ca.pem", "update-ca-trust extract"),
    ("/usr/local/share/ca-certificates/bifurc-ca.crt", "update-ca-certificates"),
    ("/etc/ca-certificates/trust-source/anchors/bifurc-ca.crt", "trust extract-compat"),
    ("/usr/share/pki/trust/anchors/bifurc-ca.pem", "update-ca-certificates"),
];

/// The shell line that puts the certificate (`$1`) in a store and rebuilds the bundle. It needs root.
fn linux_script(target: &str, rebuild: &str) -> String {
    format!("install -m 644 \"$1\" {target} && {rebuild}")
}

/// What to tell the user to run when the certificate could not be installed for them.
fn linux_instructions(certificate: &Path, target: &str, rebuild: &str) -> String {
    let command = format!("sudo sh -c '{}' sh \"{}\"", linux_script(target, rebuild), certificate.display());
    t_with("model.trust.run_in_terminal", &[("command", &command)])
}

fn install_linux(file: &Path) -> Result<String, String> {
    let (target, rebuild) = LINUX_STORES
        .into_iter()
        .find(|(target, _)| Path::new(target).parent().is_some_and(Path::is_dir))
        .ok_or_else(|| t_with("model.trust.no_store", &[("file", &file.display())]))?;
    // The stores belong to root on every distribution, so polkit asks for the password.
    run(Command::new("pkexec").args(["sh", "-c", &linux_script(target, rebuild), "sh"]).arg(file)).map_err(
        |error| {
            t_with(
                "model.trust.failed_with_instructions",
                &[("error", &error), ("instructions", &linux_instructions(file, target, rebuild))],
            )
        },
    )?;
    Ok(t("model.trust.installed_linux").to_owned())
}

/// Puts `pem` in this computer's trust store. The message says where it went, or why it did not.
pub fn install(pem: &[u8]) -> Result<String, String> {
    let file = env::temp_dir().join(format!("bifurc-ca-{}.pem", std::process::id()));
    fs::write(&file, pem).map_err(|error| t_with("model.trust.write_failed", &[("error", &error)]))?;
    let outcome = install_file(&file);
    // A failure can name the file in what it tells the user to run, so it is only removed on success.
    if outcome.is_ok() {
        let _ = fs::remove_file(&file);
    }
    outcome
}

fn install_file(file: &Path) -> Result<String, String> {
    if cfg!(windows) {
        run(Command::new("certutil").args(["-addstore", "-user", "Root"]).arg(file))?;
        Ok(t("model.trust.installed_windows").to_owned())
    } else if cfg!(target_os = "macos") {
        let keychain = home().join("Library").join("Keychains").join("login.keychain-db");
        run(Command::new("security")
            .args(["add-trusted-cert", "-d", "-r", "trustRoot", "-k"])
            .arg(keychain)
            .arg(file))?;
        Ok(t("model.trust.installed_macos").to_owned())
    } else {
        install_linux(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_name_the_file_and_the_commands() {
        let (target, rebuild) = LINUX_STORES[1];
        let text = linux_instructions(Path::new("/tmp/ca.pem"), target, rebuild);
        assert!(text.contains("/tmp/ca.pem") && text.contains(target) && text.contains("update-ca-certificates"));
    }

    #[test]
    fn a_missing_program_is_reported_by_name() {
        let error = run(&mut Command::new("bifurc-no-such-program")).unwrap_err();
        assert!(error.contains("bifurc-no-such-program"), "{error}");
    }
}
