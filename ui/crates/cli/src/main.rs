//! The `bifurc` command of an installed Bifurc.
//!
//! The CLI itself is JavaScript (`packages/cli`) and an installed app ships its own Node rather than asking
//! for one on `PATH`. A command on `PATH` has to be an executable, though — a Store package can only alias
//! one, and a script would put the bundled `node` on `PATH` beside it — so this is that executable: it finds
//! the Node and the CLI the package laid out beside it, runs one on the other with the same arguments and
//! streams, and exits with its code. It decides nothing about the engine.
//!
//! An installer also uses it to put its own folder on the user's `PATH` (`--register-path`) and to take it
//! off again (`--unregister-path`), because that folder is the one thing this executable always knows.

use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const PACKAGED_CLI: &str = "resources/node_modules/@bifurc/cli/dist/index.js";
const CHECKOUT_CLI: &str = "packages/cli/dist/index.js";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let Ok(exe) = env::current_exe() else {
        eprintln!("bifurc: could not tell where this executable is");
        return ExitCode::FAILURE;
    };
    let bin_dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();

    #[cfg(windows)]
    if let [only] = args.as_slice() {
        match only.to_str() {
            Some("--register-path") => return report(user_path::add(&bin_dir)),
            Some("--unregister-path") => return report(user_path::remove(&bin_dir)),
            _ => {}
        }
    }

    let Some((node, cli)) = locate(&bin_dir) else {
        eprintln!(
            "bifurc: could not find the CLI ({PACKAGED_CLI} beside this command, or {CHECKOUT_CLI} above it). \
             Reinstall Bifurc, or build it with `npm run build:packages`."
        );
        return ExitCode::FAILURE;
    };

    let mut command = Command::new(&node);
    command.arg(&cli).args(&args);
    run(command, &node)
}

/// The Node to run and the CLI's entry script: the installed layout first (this executable is in `bin/`,
/// under the folder that holds `runtime/` and `resources/`), then a checkout, for a launcher run from
/// `ui/target`.
fn locate(bin_dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let node_name = format!("node{}", env::consts::EXE_SUFFIX);
    if let Some(root) = bin_dir.parent() {
        let cli = root.join(PACKAGED_CLI);
        let node = root.join("runtime").join(&node_name);
        if cli.is_file() && node.is_file() {
            return Some((node, cli));
        }
    }
    let node = env::var_os("BIFURC_NODE").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| "node".into());
    bin_dir.ancestors().map(|dir| dir.join(CHECKOUT_CLI)).find(|cli| cli.is_file()).map(|cli| (node, cli))
}

/// Replaces this process with the CLI, so signals and the exit status are the CLI's own.
#[cfg(unix)]
fn run(mut command: Command, node: &Path) -> ExitCode {
    use std::os::unix::process::CommandExt;
    let error = command.exec();
    eprintln!("bifurc: could not start {}: {error}", node.display());
    ExitCode::FAILURE
}

#[cfg(not(unix))]
fn run(mut command: Command, node: &Path) -> ExitCode {
    // Ctrl+C reaches every process on the console. The CLI decides what it means; this process only has to
    // still be there to pass the exit code on.
    #[cfg(windows)]
    unsafe {
        SetConsoleCtrlHandler(Some(ignore_ctrl), 1);
    }
    match command.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("bifurc: could not start {}: {error}", node.display());
            ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
}

#[cfg(windows)]
unsafe extern "system" fn ignore_ctrl(_: u32) -> i32 {
    1
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
