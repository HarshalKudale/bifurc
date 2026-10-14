//! The `bifurc` command of an installed Bifurc.
//!
//! The CLI is the Rust one (`packages/cli-rust`), shipped beside the engine it starts. A command on `PATH` has
//! to be an executable with a stable name — a Store package can only alias one — and this is that executable:
//! it finds the CLI the package laid out, runs it with the same arguments and streams, tells it where the
//! packaged engine is, and exits with its code. It decides nothing about the engine.
//!
//! `bifurc --mcp`, on its own, starts the MCP server instead of the CLI. The server is a binary of its own
//! (`packages/mcp-rust`) that a package keeps out of `PATH`: a Store package can alias one command, and a second
//! application only for an alias is refused as headless. So the one command is the way to it.
//!
//! An installer also uses it to put its own folder on the user's `PATH` (`--register-path`) and to take it
//! off again (`--unregister-path`), because that folder is the one thing this executable always knows.

use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

/// The Rust CLI inside an install, relative to the folder above `bin/`; the engine is at that folder's top.
const PACKAGED_CLI: &str = "resources/bifurc-cli";
const PACKAGED_MCP: &str = "resources/bifurc-mcp";
/// The flag that selects the MCP server. It stands alone: the server takes no options through this command.
const MCP_FLAG: &str = "--mcp";
const PACKAGED_ENGINE: &str = "bifurc-engine";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    // On a PATH symlink (macOS keeps the one it is called by) the install is where the link points.
    let exe = env::current_exe().and_then(|exe| if cfg!(unix) { exe.canonicalize() } else { Ok(exe) });
    let Ok(exe) = exe else {
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

    let mcp = args.first().is_some_and(|arg| arg == MCP_FLAG);
    if mcp && args.len() > 1 {
        eprintln!("bifurc: {MCP_FLAG} starts the MCP server and cannot be combined with other options or commands");
        return ExitCode::from(2);
    }
    // The MCP server takes no options through this command.
    let (packaged, built, package, args) =
        if mcp { (PACKAGED_MCP, "bifurc-mcp", "bifurc-mcp", Vec::new()) } else { (PACKAGED_CLI, "bifurc", "bifurc-cli", args) };

    let Some((cli, engine)) = locate(&bin_dir, &exe, packaged, built) else {
        eprintln!(
            "bifurc: could not find {packaged} beside this command, or a {built} build in a target/ above it. \
             Reinstall Bifurc, or build it with `cargo build -p {package}` at the repository root."
        );
        return ExitCode::FAILURE;
    };

    let mut command = Command::new(&cli);
    command.args(&args);
    if let Some(engine) = engine.filter(|_| env::var_os("BIFURC_ENGINE").is_none_or(|v| v.is_empty())) {
        command.env("BIFURC_ENGINE", engine);
    }
    run(command, &cli)
}

/// The CLI to run and, for an install, the engine it should start: the installed layout first (this executable
/// is in `bin/`, under the folder that holds `resources/`), then the newest `bifurc` build in a `target/` above
/// a checkout's launcher. That build finds its own engine; the launcher itself is skipped.
fn locate(bin_dir: &Path, this: &Path, packaged: &str, built: &str) -> Option<(PathBuf, Option<PathBuf>)> {
    let suffix = env::consts::EXE_SUFFIX;
    if let Some(root) = bin_dir.parent() {
        let cli = root.join(format!("{packaged}{suffix}"));
        if cli.is_file() {
            return Some((cli, Some(root.join(format!("{PACKAGED_ENGINE}{suffix}")))));
        }
    }
    let this = this.canonicalize().ok();
    bin_dir
        .ancestors()
        // Only the repository root: `packages/ui-rust/target` holds launchers of this very kind, not the CLI.
        .filter(|dir| dir.join("packages").join("cli-rust").is_dir())
        .flat_map(|dir| ["release", "debug"].map(|profile| dir.join("target").join(profile).join(format!("{built}{suffix}"))))
        .filter(|candidate| candidate.is_file() && candidate.canonicalize().ok() != this)
        .filter_map(|candidate| Some((std::fs::metadata(&candidate).ok()?.modified().ok()?, candidate)))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, cli)| (cli, None))
}

/// Replaces this process with the CLI, so signals and the exit status are the CLI's own.
#[cfg(unix)]
fn run(mut command: Command, program: &Path) -> ExitCode {
    use std::os::unix::process::CommandExt;
    let error = command.exec();
    eprintln!("bifurc: could not start {}: {error}", program.display());
    ExitCode::FAILURE
}

#[cfg(not(unix))]
fn run(mut command: Command, program: &Path) -> ExitCode {
    // Ctrl+C reaches every process on the console. The CLI decides what it means; this process only has to
    // still be there to pass the exit code on.
    #[cfg(windows)]
    unsafe {
        SetConsoleCtrlHandler(Some(ignore_ctrl), 1);
    }
    match command.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("bifurc: could not start {}: {error}", program.display());
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
