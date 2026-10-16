//! The `bifurc` command: the Bifurc CLI, one executable for a checkout and an install alike.
//!
//! `bifurc --mcp` starts the MCP server in this process instead (the options after it are the server's own).
//! An installer also uses it to put its own folder on the user's `PATH` (`--register-path`) and to take it
//! off again (`--unregister-path`), because that folder is the one thing this executable always knows.
//! The engine is found by the CLI itself (`bifurc_cli::locate`): beside this executable, above its `bin/`,
//! or in a `target/` above it.

use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    #[cfg(windows)]
    if let [only] = argv.as_slice() {
        let bin_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
        let change = match (only.as_str(), bin_dir) {
            ("--register-path", Some(dir)) => Some(user_path::add(&dir)),
            ("--unregister-path", Some(dir)) => Some(user_path::remove(&dir)),
            _ => None,
        };
        if let Some(outcome) = change {
            return report(outcome);
        }
    }

    if argv.first().is_some_and(|arg| arg == "--mcp") {
        return bifurc_mcp::run::run(&argv[1..]);
    }

    // shortcut: a panic is reported to the default data directory even when `--data-dir` names another; parse
    // the flag here if crashes of such runs ever matter.
    let data_dir = std::env::var_os("BIFURC_DATA_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(bifurc_cli::discovery::platform_default_data_dir, Into::into);
    bifurc_protocol::crash::install("cli", bifurc_cli::cli::VERSION, data_dir);
    let code = bifurc_cli::cli::run(&argv, &mut std::io::stdout(), &mut std::io::stderr());
    // The CLI's code is an i32 it has already limited to what a process can return.
    std::process::exit(code);
}

#[cfg(windows)]
fn report(outcome: std::io::Result<()>) -> ExitCode {
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bifurc: could not update the user's PATH: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The current user's `PATH` (`HKCU\Environment`), read and written unexpanded so entries such as
/// `%USERPROFILE%\bin` stay as their owner wrote them.
#[cfg(windows)]
mod user_path {
    use std::{io, path::Path};

    use winreg::{
        RegKey, RegValue,
        enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ},
    };

    const VALUE: &str = "Path";

    pub fn add(dir: &Path) -> io::Result<()> {
        rewrite(dir, true)
    }

    pub fn remove(dir: &Path) -> io::Result<()> {
        rewrite(dir, false)
    }

    fn rewrite(dir: &Path, present: bool) -> io::Result<()> {
        let dir = dir.to_string_lossy().trim_end_matches('\\').to_owned();
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)?;
        let (current, kind) = match key.get_raw_value(VALUE) {
            Ok(raw) if matches!(raw.vtype, REG_SZ | REG_EXPAND_SZ) => (wide_to_string(&raw.bytes), raw.vtype),
            Ok(_) => return Err(io::Error::other("the Path value is not text")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (String::new(), REG_EXPAND_SZ),
            Err(error) => return Err(error),
        };

        let same = |entry: &str| entry.trim().trim_end_matches('\\').eq_ignore_ascii_case(&dir);
        let mut entries: Vec<&str> = current.split(';').filter(|entry| !entry.trim().is_empty()).collect();
        let had = entries.iter().any(|entry| same(entry));
        if had == present {
            return Ok(());
        }
        if present {
            entries.push(&dir);
        } else {
            entries.retain(|entry| !same(entry));
        }

        let bytes = entries.join(";").encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        key.set_raw_value(VALUE, &RegValue { bytes, vtype: kind })?;
        announce();
        Ok(())
    }

    fn wide_to_string(bytes: &[u8]) -> String {
        let wide: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        String::from_utf16_lossy(&wide).trim_end_matches('\0').to_owned()
    }

    /// Tells running programs (Explorer, and so every terminal opened after this) that the environment
    /// changed. Without it the new `PATH` only shows after signing out.
    fn announce() {
        const HWND_BROADCAST: isize = 0xffff;
        const WM_SETTINGCHANGE: u32 = 0x001A;
        const SMTO_ABORTIFHUNG: u32 = 0x0002;
        let what: Vec<u16> = "Environment".encode_utf16().chain([0]).collect();
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                what.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                2000,
                std::ptr::null_mut(),
            );
        }
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            message: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }
}
