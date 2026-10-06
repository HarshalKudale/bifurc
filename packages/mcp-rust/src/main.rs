//! The `bifurc-mcp` binary: reads its options and serves MCP on stdin and stdout until the client closes them.
//! Stdout carries the protocol and nothing else; anything said to a person goes to stderr.

use std::process::ExitCode;

use bifurc_mcp::{USAGE, invoke::Settings, server::serve};

/// The options of one start, or the text to print and the exit code instead of serving.
fn options(argv: &[String]) -> Result<(Settings, bool), (String, u8)> {
    let mut settings = Settings { env_data_dir: std::env::var_os("BIFURC_DATA_DIR"), ..Settings::default() };
    let mut all_tools = false;
    let mut rest = argv.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-h" | "--help" | "help" => return Err((USAGE.to_string(), 0)),
            "-V" | "--version" => return Err((format!("bifurc-mcp {}", env!("CARGO_PKG_VERSION")), 0)),
            "--no-start" => settings.no_start = true,
            "--all-tools" => all_tools = true,
            "--data-dir" => match rest.next() {
                Some(dir) => settings.data_dir = Some(dir.clone()),
                None => return Err(("bifurc-mcp: --data-dir needs a value".to_string(), 2)),
            },
            other => match other.strip_prefix("--data-dir=") {
                Some(dir) => settings.data_dir = Some(dir.to_string()),
                None => return Err((format!("bifurc-mcp: unknown option \"{other}\"\n\n{USAGE}"), 2)),
            },
        }
    }
    Ok((settings, all_tools))
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (settings, all_tools) = match options(&argv) {
        Ok(options) => options,
        Err((text, 0)) => {
            println!("{text}");
            return ExitCode::SUCCESS;
        }
        Err((text, code)) => {
            eprintln!("{text}");
            return ExitCode::from(code);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("bifurc-mcp: could not start: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve(settings, all_tools)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bifurc-mcp: {error}");
            ExitCode::FAILURE
        }
    }
}
