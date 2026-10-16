//! Asking for a secret on the terminal without echoing it. The prompt and the answer go through the
//! controlling terminal itself, not stdin and stderr, so they work when the program's own streams are
//! redirected and they do not consume input meant for the program that is launched afterwards.

use std::io::{BufRead, BufReader, Write};

use crate::error::{CliError, Result};

/// Read one line from the terminal with echo off. An error when there is no terminal.
pub fn secret(prompt: &str) -> Result<String> {
    read_hidden(prompt).map_err(|e| {
        CliError::msg(format!(
            "Could not ask for the passphrase ({e}). Run this in a terminal, or set BIFURC_VAULT_PASSPHRASE, or \
             pipe it with --passphrase-stdin."
        ))
    })
}

/// One line from stdin (`--passphrase-stdin`).
pub fn stdin_line() -> Result<String> {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|e| CliError::msg(format!("cannot read stdin: {e}")))?;
    Ok(trim_newline(line))
}

fn trim_newline(mut line: String) -> String {
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    line
}

#[cfg(windows)]
fn read_hidden(prompt: &str) -> std::io::Result<String> {
    use std::{fs::OpenOptions, os::windows::io::AsRawHandle};

    use windows_sys::Win32::System::Console::{ENABLE_ECHO_INPUT, GetConsoleMode, SetConsoleMode};

    let input = OpenOptions::new().read(true).write(true).open("CONIN$")?;
    let mut output = OpenOptions::new().write(true).open("CONOUT$")?;
    let handle = input.as_raw_handle();
    let mut mode = 0u32;
    // SAFETY: `handle` is the console input handle opened above, and `mode` is a valid out pointer.
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: as above; the mode is put back before returning.
    unsafe { SetConsoleMode(handle, mode & !ENABLE_ECHO_INPUT) };
    write!(output, "{prompt}")?;
    output.flush()?;
    let mut line = String::new();
    let read = BufReader::new(&input).read_line(&mut line);
    // SAFETY: restores the mode read above.
    unsafe { SetConsoleMode(handle, mode) };
    writeln!(output)?;
    read?;
    Ok(trim_newline(line))
}

#[cfg(unix)]
fn read_hidden(prompt: &str) -> std::io::Result<String> {
    use std::{fs::OpenOptions, os::fd::AsRawFd};

    let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let fd = tty.as_raw_fd();
    // SAFETY: `fd` is the open terminal; `termios` is filled by tcgetattr before it is used.
    let mut original: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut silent = original;
    silent.c_lflag &= !libc::ECHO;
    // SAFETY: sets a valid termios on the open terminal; the original is restored below.
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &silent) };
    let mut output = &tty;
    write!(output, "{prompt}")?;
    output.flush()?;
    let mut line = String::new();
    let read = BufReader::new(&tty).read_line(&mut line);
    // SAFETY: restores the attributes read above.
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &original) };
    writeln!(output)?;
    read?;
    Ok(trim_newline(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_are_trimmed_but_spaces_are_kept() {
        assert_eq!(trim_newline("pass phrase \r\n".into()), "pass phrase ");
        assert_eq!(trim_newline("x".into()), "x");
    }
}
