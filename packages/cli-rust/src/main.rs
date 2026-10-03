//! The `bifurc` binary: hands the command line to the library and exits with its code.

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let code = bifurc_cli::cli::run(&argv, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
