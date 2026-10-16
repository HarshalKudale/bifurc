//! The `bifurc` binary: hands the command line to the library and exits with its code.

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // shortcut: a panic is reported to the default data directory even when `--data-dir` names another; parse
    // the flag here if crashes of such runs ever matter.
    let data_dir = std::env::var_os("BIFURC_DATA_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(bifurc_cli::discovery::platform_default_data_dir, Into::into);
    bifurc_protocol::crash::install("cli", bifurc_cli::cli::VERSION, data_dir);
    let code = bifurc_cli::cli::run(&argv, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
