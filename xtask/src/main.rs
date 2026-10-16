//! Packages the desktop app — the Rust UI, the Rust engine, the Rust CLI and the MCP server — as one install.
//!
//! ```text
//! cargo xtask package                       # everything this platform has: packages and store manifests
//! cargo xtask package --target nsis,msix    # windows: installer exe, Store package
//! cargo xtask package --target appimage,deb # linux
//! cargo xtask package --target dmg          # macos: the whole thing is the default there
//! cargo xtask package --target dir          # the staged folder only
//! cargo xtask package --skip-build          # package what is already built
//! cargo xtask package --release-tag v1.2.3  # the release the manifests download from (default: v<version>)
//! ```
//!
//! | target     | platform | output in `release/desktop`                                               |
//! | ---------- | -------- | ------------------------------------------------------------------------- |
//! | `nsis`     | windows  | `Bifurc-<version>-setup.exe`                                              |
//! | `msix`     | windows  | `Bifurc-<version>.msix`                                                   |
//! | `zip`      | windows  | `Bifurc-<version>-win-<arch>.zip`, the staged folder as it is             |
//! | `winget`   | windows  | `manifests/winget/…`, the three files a winget-pkgs pull request holds    |
//! | `scoop`    | windows  | `manifests/scoop/bifurc.json`                                             |
//! | `appimage` | linux    | `Bifurc-<version>-<arch>.AppImage`                                        |
//! | `deb`      | linux    | `bifurc_<version>_<arch>.deb`                                             |
//! | `tarball`  | linux    | `Bifurc-<version>-linux-<arch>.tar.gz`, the staged folder as it is        |
//! | `aur`      | linux    | `manifests/aur/…`: `PKGBUILD`, `.SRCINFO`, `bifurc.install`               |
//! | `dmg`      | macos    | `Bifurc-<version>-mac-<arch>.dmg`: `Bifurc.app` and an Applications link  |
//!
//! A manifest names the file a package manager downloads and its hash, so its target builds that file too:
//! `winget` the installer, `scoop` the zip, `aur` the tarball. The download is the file of that
//! name on the GitHub release `--release-tag` names, in `GITHUB_REPOSITORY` (default: this repository).
//!
//! Every package is the same folder (`release/desktop/<platform>-unpacked`) in a different wrapper:
//!
//! ```text
//! bifurc-ui[.exe]      the interface
//! bifurc-engine[.exe]  the engine
//! bifurc-tray[.exe]    the engine's tray icon (Windows), menu-bar icon (macOS) or StatusNotifierItem (Linux)
//! bin/bifurc[.exe]     the `bifurc` command — the only folder that goes on PATH; runs resources/bifurc-cli
//! resources/bifurc-mcp[.exe]  the MCP server (`crates/mcp`), started by `bifurc --mcp`; not on PATH
//! resources/bifurc-cli[.exe]   the CLI (`crates/cli`)
//! ```
//!
//! `crates/engine-client/src/sidecar.rs` and `crates/launcher/src/main.rs` (`bifurc-launcher`) look for exactly these paths
//! beside their own executable, and the engine finds the tray helper beside itself, so the folder works from
//! wherever it is put and no installer has to write a configuration.
//!
//! The one thing a wrapper adds is `resources/install.json`, which names it (`{"package": "nsis"}`): how the
//! engine knows how this copy is updated (`crates/engine/src/updates/channel.rs`). The staged folder
//! itself has none, and is a build.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

type Result<T> = std::result::Result<T, String>;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const WINDOWS: bool = cfg!(windows);
const EXE: &str = if WINDOWS { ".exe" } else { "" };

/// What the Windows SDK calls the architecture, and what the manifest wants.
fn sdk_arch() -> &'static str {
    if env::consts::ARCH == "aarch64" { "arm64" } else { "x64" }
}

/// What AppImage calls it.
fn appimage_arch() -> &'static str {
    if env::consts::ARCH == "aarch64" { "aarch64" } else { "x86_64" }
}

/// What Debian calls it.
fn deb_arch() -> &'static str {
    if env::consts::ARCH == "aarch64" { "arm64" } else { "amd64" }
}

/// What the disk image's name calls it.
fn mac_arch() -> &'static str {
    if env::consts::ARCH == "aarch64" { "arm64" } else { "x64" }
}

/// What Scoop calls it.
fn scoop_arch() -> &'static str {
    if env::consts::ARCH == "aarch64" { "arm64" } else { "64bit" }
}

/// Every target of each platform, in the order they are made: a manifest after the file it describes.
const WINDOWS_TARGETS: [&str; 5] = ["nsis", "msix", "zip", "winget", "scoop"];
const LINUX_TARGETS: [&str; 4] = ["appimage", "deb", "tarball", "aur"];
const MACOS_TARGETS: [&str; 1] = ["dmg"];
const MACOS: bool = cfg!(target_os = "macos");

/// The package a manifest's target describes, which is built with it.
fn needs(target: &str) -> Option<&'static str> {
    match target {
        "winget" => Some("nsis"),
        "scoop" => Some("zip"),
        "aur" => Some("tarball"),
        _ => None,
    }
}

const DEFAULT_REPOSITORY: &str = "HarshalKudale/bifurc";
const HOMEPAGE: &str = "https://bifurc.app";
const PUBLISHER: &str = "Harshal Kudale";
/// A package's maintainer field wants an address; this one reaches the maintainer through GitHub.
const MAINTAINER_EMAIL: &str = "HarshalKudale@users.noreply.github.com";
const WINGET_PUBLISHER: &str = "HarshalKudale";
const WINGET_NAME: &str = "Bifurc";
/// One line and one paragraph, in words that are safe as they are in YAML, XML, JSON and a shell string.
const SUMMARY: &str = "Desktop API development workspace for localhost routing, capture, mocking and protocol testing";
const DESCRIPTION: &str = "Bifurc brings together .localhost mappings, proxy rules, traffic capture, mocks, and REST, \
                           GraphQL, SOAP, gRPC and WebSocket requests, kept as files in git-backed workspaces.";

struct Ctx {
    repo: PathBuf,
    out_dir: PathBuf,
    /// `owner/name` on GitHub: where the release the manifests download from is.
    repository: String,
    /// The tag of that release.
    release_tag: String,
}

impl Ctx {
    fn packaging(&self) -> PathBuf {
        self.repo.join("packaging")
    }

    fn manifests(&self) -> PathBuf {
        self.out_dir.join("manifests")
    }

    fn repo_url(&self) -> String {
        format!("https://github.com/{}", self.repository)
    }

    /// Where `file` is downloaded from once it is attached to the release.
    fn asset_url(&self, file: &Path) -> String {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        format!("{}/releases/download/{}/{name}", self.repo_url(), self.release_tag)
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("package") => {}
        _ => {
            return Err(format!(
                "usage: cargo xtask package [--target dir,{},{},{}] [--skip-build] [--release-tag <tag>]",
                WINDOWS_TARGETS.join(","),
                LINUX_TARGETS.join(","),
                MACOS_TARGETS.join(",")
            ));
        }
    }
    let args: Vec<String> = args.collect();
    let skip_build = args.iter().any(|a| a == "--skip-build");
    let value_of = |flag: &str| -> Result<Option<&str>> {
        match args.iter().position(|a| a == flag) {
            Some(at) => Ok(Some(args.get(at + 1).ok_or(format!("{flag} needs a value"))?.as_str())),
            None => Ok(None),
        }
    };
    let platform_targets: &[&str] = if WINDOWS {
        &WINDOWS_TARGETS
    } else if MACOS {
        &MACOS_TARGETS
    } else {
        &LINUX_TARGETS
    };
    let mut targets: Vec<&str> = match value_of("--target")? {
        Some(spec) => spec.split(',').filter(|t| !t.is_empty()).collect(),
        None => platform_targets.to_vec(),
    };
    for target in &targets {
        if *target == "dir" || platform_targets.contains(target) {
            continue;
        }
        let built_on = if WINDOWS_TARGETS.contains(target) {
            Some("windows")
        } else if LINUX_TARGETS.contains(target) {
            Some("linux")
        } else if MACOS_TARGETS.contains(target) {
            Some("macos")
        } else {
            None
        };
        if let Some(built_on) = built_on {
            return Err(format!("{target} is built on {built_on}; this is {}", env::consts::OS));
        }
        return Err(format!(
            "unknown target \"{target}\" (dir, {}, {}, {})",
            WINDOWS_TARGETS.join(", "),
            LINUX_TARGETS.join(", "),
            MACOS_TARGETS.join(", ")
        ));
    }
    for needed in targets.clone().into_iter().filter_map(needs) {
        if !targets.contains(&needed) {
            targets.push(needed);
        }
    }
    let wanted = |target: &str| targets.contains(&target);

    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("no repository above xtask")?.to_path_buf();
    let ctx = Ctx {
        out_dir: repo.join("release").join("desktop"),
        repo,
        repository: env::var("GITHUB_REPOSITORY").ok().filter(|r| !r.is_empty()).unwrap_or_else(|| DEFAULT_REPOSITORY.to_owned()),
        release_tag: value_of("--release-tag")?.filter(|tag| !tag.is_empty()).map_or_else(|| format!("v{VERSION}"), str::to_owned),
    };

    if !skip_build {
        build(&ctx)?;
    }
    let staged = stage(&ctx)?;
    let mut made = vec![staged.clone()];
    if wanted("nsis") {
        mark(&staged, "nsis")?;
        let installer = package_nsis(&ctx, &staged)?;
        made.push(installer.clone());
        if wanted("winget") {
            made.push(manifest_winget(&ctx, &installer)?);
        }
    }
    if wanted("msix") {
        made.push(package_msix(&ctx, &staged)?);
    }
    if wanted("zip") {
        mark(&staged, "zip")?;
        let zip = package_zip(&ctx, &staged)?;
        made.push(zip.clone());
        if wanted("scoop") {
            made.push(manifest_scoop(&ctx, &zip)?);
        }
    }
    if wanted("dmg") {
        made.push(package_dmg(&ctx, &staged)?);
    }
    if wanted("appimage") {
        made.push(package_appimage(&ctx, &staged)?);
    }
    if wanted("deb") {
        made.push(package_deb(&ctx, &staged)?);
    }
    if wanted("tarball") {
        let tarball = package_tarball(&ctx, &staged)?;
        made.push(tarball.clone());
        if wanted("aur") {
            made.push(manifest_aur(&ctx, &tarball)?);
        }
    }
    let _ = fs::remove_file(install_marker(&staged));
    println!("\nBifurc {VERSION} (manifests download from {}):", ctx.asset_url(Path::new("")).trim_end_matches('/'));
    for file in made {
        println!("  {}", file.strip_prefix(&ctx.repo).unwrap_or(&file).display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------------------

/// Runs `command`. `quiet` keeps a tool that lists every file it touches off the console unless it fails.
fn exec(mut command: Command, quiet: bool) -> Result<()> {
    println!("> {command:?}");
    if quiet {
        let output = command.output().map_err(|e| format!("could not run {:?}: {e}", command.get_program()))?;
        if !output.status.success() {
            return Err(format!(
                "{:?} exited with {}\n{}{}",
                command.get_program(),
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    } else {
        let status = command.status().map_err(|e| format!("could not run {:?}: {e}", command.get_program()))?;
        if !status.success() {
            return Err(format!("{:?} exited with {status}", command.get_program()));
        }
    }
    Ok(())
}

fn empty_dir(dir: &Path) -> Result<()> {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

fn copy_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::copy(from, to).map(|_| ()).map_err(|e| format!("copying {} to {}: {e}", from.display(), to.display()))
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

fn write_file(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn install_marker(app: &Path) -> PathBuf {
    app.join("resources").join("install.json")
}

/// Says in the app folder `app` which package it is in.
fn mark(app: &Path, package: &str) -> Result<()> {
    write_file(&install_marker(app), &format!("{{\"package\": \"{package}\"}}\n"))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

/// A template from `packaging/`, with `{{NAME}}` filled in. Always LF: two of them are shell and desktop files.
fn template(ctx: &Ctx, file: &str, values: &[(&str, &str)]) -> Result<String> {
    let path = ctx.packaging().join(file);
    let mut text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?.replace("\r\n", "\n");
    for (name, value) in values {
        text = text.replace(&format!("{{{{{name}}}}}"), value);
    }
    Ok(text)
}

/// A template with what every manifest says about the app filled in, besides its own `values`.
fn manifest(ctx: &Ctx, file: &str, values: &[(&str, &str)]) -> Result<String> {
    let repo_url = ctx.repo_url();
    let license_url = format!("{repo_url}/blob/master/LICENSE");
    let icon_url = format!("https://raw.githubusercontent.com/{}/master/build/icon.png", ctx.repository);
    let mut all = vec![
        ("VERSION", VERSION),
        ("TAG", ctx.release_tag.as_str()),
        ("HOMEPAGE", HOMEPAGE),
        ("REPO_URL", repo_url.as_str()),
        ("LICENSE_URL", license_url.as_str()),
        ("ICON_URL", icon_url.as_str()),
        ("PUBLISHER", PUBLISHER),
        ("MAINTAINER_EMAIL", MAINTAINER_EMAIL),
        ("SUMMARY", SUMMARY),
        ("DESCRIPTION", DESCRIPTION),
    ];
    all.extend_from_slice(values);
    template(ctx, file, &all)
}

/// The SHA-256 of a file, in lowercase hex: what a manifest pins its download to.
fn sha256_of(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The size of everything under `dir`, in bytes.
fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => dir_size(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |meta| meta.len()),
            _ => 0,
        })
        .sum()
}

#[cfg(unix)]
fn symlink(target: &str, link: &Path) -> Result<()> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::os::unix::fs::symlink(target, link).map_err(|e| format!("{}: {e}", link.display()))
}

#[cfg(not(unix))]
fn symlink(_: &str, link: &Path) -> Result<()> {
    Err(format!("{}: a Linux package is laid out on Linux", link.display()))
}

fn on_path(name: &str) -> Option<PathBuf> {
    let output = Command::new(if WINDOWS { "where" } else { "which" }).arg(name).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).lines().next().map(|l| PathBuf::from(l.trim()))
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn first_existing(candidates: impl IntoIterator<Item = Option<PathBuf>>) -> Option<PathBuf> {
    candidates.into_iter().flatten().find(|c| c.exists())
}

// ---------------------------------------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------------------------------------

/// The compiler flags of a packaged build: they keep the build machine out of the binaries. A panic message
/// names the file it came from, and without these that is a path under the home directory of whoever built it.
fn release_rustflags(ctx: &Ctx) -> String {
    let home = if WINDOWS { "USERPROFILE" } else { "HOME" };
    let cargo_home = env_path("CARGO_HOME").or_else(|| env_path(home).map(|dir| dir.join(".cargo")));
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()));
    let mut flags = env::var("RUSTFLAGS").unwrap_or_default();
    for (path, name) in [(Some(ctx.repo.clone()), "bifurc"), (cargo_home, "cargo"), (sysroot, "rust")] {
        let Some(path) = path.map(|path| path.display().to_string()) else { continue };
        // RUSTFLAGS is split at spaces, so a path with one in it cannot be passed this way.
        if path.contains(char::is_whitespace) {
            println!("warning: {path} has a space in it and stays in the binaries as it is");
            continue;
        }
        flags.push_str(&format!(" --remap-path-prefix={path}={name}"));
    }
    flags.trim().to_string()
}

fn build(ctx: &Ctx) -> Result<()> {
    // One workspace, one cargo: the engine, CLI and MCP server, the app (which builds the tray helper beside it,
    // `crates/app/build.rs`) and the `bifurc` launcher that goes on PATH.
    let mut cargo = Command::new("cargo");
    cargo
        .args(["build", "--release", "--locked"])
        .args(["-p", "bifurc-engine", "-p", "bifurc-cli", "-p", "bifurc-mcp", "-p", "bifurc-ui", "-p", "bifurc-launcher"])
        .env("RUSTFLAGS", release_rustflags(ctx))
        .current_dir(&ctx.repo);
    exec(cargo, false)
}

/// A release build of `name` from the repository's cargo target directory.
fn cargo_output(ctx: &Ctx, name: &str) -> Result<PathBuf> {
    let target_dir = env_path("CARGO_TARGET_DIR").map_or_else(|| ctx.repo.join("target"), |dir| ctx.repo.join(dir));
    let file = target_dir.join("release").join(name);
    if file.exists() { Ok(file) } else { Err(format!("{} is missing — build first (drop --skip-build)", file.display())) }
}

// ---------------------------------------------------------------------------------------------------------
// The staged folder
// ---------------------------------------------------------------------------------------------------------

fn stage(ctx: &Ctx) -> Result<PathBuf> {
    let dir = ctx.out_dir.join(if WINDOWS {
        "win-unpacked"
    } else if MACOS {
        "mac-unpacked"
    } else {
        "linux-unpacked"
    });
    empty_dir(&dir)?;
    copy_file(&cargo_output(ctx, &format!("bifurc-ui{EXE}"))?, &dir.join(format!("bifurc-ui{EXE}")))?;
    copy_file(&cargo_output(ctx, &format!("bifurc-tray{EXE}"))?, &dir.join(format!("bifurc-tray{EXE}")))?;
    make_executable(&dir.join(format!("bifurc-tray{EXE}")))?;
    // The launcher is `bin/bifurc`; the CLI it runs and the engine are the others.
    copy_file(&cargo_output(ctx, &format!("bifurc-launcher{EXE}"))?, &dir.join("bin").join(format!("bifurc{EXE}")))?;
    copy_file(&cargo_output(ctx, &format!("bifurc{EXE}"))?, &dir.join("resources").join(format!("bifurc-cli{EXE}")))?;
    // The MCP server is a command of its own: it finds the engine above its `bin/` (`cli/src/locate.rs`).
    copy_file(&cargo_output(ctx, &format!("bifurc-mcp{EXE}"))?, &dir.join("resources").join(format!("bifurc-mcp{EXE}")))?;
    copy_file(&cargo_output(ctx, &format!("bifurc-engine{EXE}"))?, &dir.join(format!("bifurc-engine{EXE}")))?;
    copy_file(&ctx.repo.join("build").join("icon.png"), &dir.join("resources").join("icon.png"))?;
    copy_file(&ctx.repo.join("LICENSE"), &dir.join("LICENSE"))?;
    for file in [format!("bifurc-ui{EXE}"), format!("bifurc-engine{EXE}"), format!("bin/bifurc{EXE}"), format!("resources/bifurc-mcp{EXE}"), format!("resources/bifurc-cli{EXE}")] {
        make_executable(&dir.join(file))?;
    }
    smoke_test(&dir)?;
    Ok(dir)
}

/// The staged `bifurc` must reach the staged CLI, and the staged MCP server and engine must run, before anything
/// wraps them.
fn smoke_test(dir: &Path) -> Result<()> {
    let cli = Command::new(dir.join("bin").join(format!("bifurc{EXE}"))).args(["commands", "--json"]).output();
    match cli {
        Ok(out) if out.status.success() => {}
        Ok(out) => return Err(format!("the staged CLI does not run:\n{}", String::from_utf8_lossy(&out.stderr))),
        Err(e) => return Err(format!("the staged CLI does not run:\n{e}")),
    }
    let mcp = Command::new(dir.join("resources").join(format!("bifurc-mcp{EXE}"))).arg("--version").output();
    match mcp {
        Ok(out) if out.status.success() => {}
        Ok(out) => return Err(format!("the staged MCP server does not run:\n{}", String::from_utf8_lossy(&out.stderr))),
        Err(e) => return Err(format!("the staged MCP server does not run:\n{e}")),
    }
    let engine = Command::new(dir.join(format!("bifurc-engine{EXE}"))).arg("--help").output();
    match engine {
        Ok(out) if out.status.success() => {}
        Ok(out) => return Err(format!("the staged engine does not run:\n{}", String::from_utf8_lossy(&out.stderr))),
        Err(e) => return Err(format!("the staged engine does not run:\n{e}")),
    }
    println!("staged CLI, MCP server and engine start");
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Windows: installer
// ---------------------------------------------------------------------------------------------------------

fn find_makensis() -> Result<PathBuf> {
    first_existing([
        env_path("MAKENSIS"),
        on_path("makensis"),
        env_path("ProgramFiles(x86)").map(|d| d.join("NSIS").join("makensis.exe")),
        env_path("ProgramFiles").map(|d| d.join("NSIS").join("makensis.exe")),
    ])
    .ok_or_else(|| "makensis was not found. Install NSIS (https://nsis.sourceforge.io) or set MAKENSIS to makensis.exe.".to_owned())
}

fn package_nsis(ctx: &Ctx, staged: &Path) -> Result<PathBuf> {
    let out = ctx.out_dir.join(format!("Bifurc-{VERSION}-setup.exe"));
    let mut command = Command::new(find_makensis()?);
    command
        .arg("/V2")
        .arg(format!("/DVERSION={VERSION}"))
        .arg(format!("/DSTAGE={}", staged.display()))
        .arg(format!("/DOUT={}", out.display()))
        .arg(format!("/DICON={}", ctx.repo.join("build").join("icon.ico").display()))
        .arg(ctx.packaging().join("windows").join("bifurc.nsi"))
        .current_dir(&ctx.repo);
    exec(command, false)?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------
// Windows: Store package
// ---------------------------------------------------------------------------------------------------------

fn find_sdk_tool(name: &str) -> Result<PathBuf> {
    if let Some(found) = on_path(name) {
        return Ok(found);
    }
    let kits = env_path("ProgramFiles(x86)").map(|d| d.join("Windows Kits").join("10").join("bin"));
    let mut versions: Vec<String> = kits
        .as_ref()
        .and_then(|dir| fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("10."))
        .collect();
    // Newest first, comparing each dotted number as a number.
    let key = |v: &str| v.split('.').map(|part| part.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    versions.sort_by(|a, b| key(b).cmp(&key(a)));
    first_existing(versions.iter().map(|sdk| kits.as_ref().map(|k| k.join(sdk).join(sdk_arch()).join(name))))
        .ok_or_else(|| format!("{name} was not found. Install the Windows SDK."))
}

fn package_msix(ctx: &Ctx, staged: &Path) -> Result<PathBuf> {
    let layout = ctx.out_dir.join("msix-layout");
    empty_dir(&layout)?;
    copy_dir(staged, &layout)?;
    mark(&layout, "msix")?;
    // The icons are indexed from a folder of their own, so indexing never reads file names in the layout as
    // resource qualifiers.
    let icons = ctx.out_dir.join("msix-icons");
    empty_dir(&icons)?;
    for assets in [ctx.repo.join("build").join("appx"), ctx.packaging().join("windows").join("assets")] {
        for entry in fs::read_dir(&assets).map_err(|e| format!("{}: {e}", assets.display()))?.flatten() {
            if entry.file_name().to_string_lossy().ends_with(".png") {
                copy_file(&entry.path(), &icons.join("assets").join(entry.file_name()))?;
            }
        }
    }
    copy_dir(&icons, &layout)?;
    // A package version is four numbers, and the Store keeps the fourth for itself.
    let manifest = layout.join("AppxManifest.xml");
    write_file(&manifest, &template(ctx, "windows/AppxManifest.xml", &[("VERSION", &format!("{VERSION}.0")), ("ARCH", sdk_arch())])?)?;

    // The index that lets Windows pick the unplated, size-matched icons for the taskbar and the Start menu.
    let pri_config = ctx.out_dir.join("priconfig.xml");
    let makepri = find_sdk_tool("makepri.exe")?;
    let mut create = Command::new(&makepri);
    create.args(["createconfig", "/cf"]).arg(&pri_config).args(["/dq", "en-US", "/pv", "10.0.0", "/o"]).current_dir(&ctx.repo);
    exec(create, true)?;
    let mut index = Command::new(&makepri);
    index
        .args(["new", "/pr"])
        .arg(&icons)
        .arg("/cf")
        .arg(&pri_config)
        .arg("/of")
        .arg(layout.join("resources.pri"))
        .arg("/mn")
        .arg(&manifest)
        .arg("/o")
        .current_dir(&ctx.repo);
    exec(index, true)?;

    let out = ctx.out_dir.join(format!("Bifurc-{VERSION}.msix"));
    let mut pack = Command::new(find_sdk_tool("makeappx.exe")?);
    pack.args(["pack", "/d"]).arg(&layout).arg("/p").arg(&out).arg("/o").current_dir(&ctx.repo);
    exec(pack, true)?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------
// Windows: portable zip, and the manifests of winget and Scoop
// ---------------------------------------------------------------------------------------------------------

/// The staged folder as a zip: what Scoop installs, and a Bifurc that runs from wherever it is unpacked.
fn package_zip(ctx: &Ctx, staged: &Path) -> Result<PathBuf> {
    let out = ctx.out_dir.join(format!("Bifurc-{VERSION}-win-{}.zip", sdk_arch()));
    let _ = fs::remove_file(&out);
    // Windows' own tar (bsdtar) writes a zip when the name says so. Named by its path: a `tar` earlier on
    // PATH may be GNU tar, which does not.
    let tar = env_path("SystemRoot").map(|root| root.join("System32").join("tar.exe")).filter(|tar| tar.exists());
    let mut command = Command::new(tar.ok_or("tar.exe was not found in System32 (it ships with Windows 10 1803 and later)")?);
    command.args(["-a", "-c", "-f"]).arg(&out).arg("-C").arg(staged);
    // Each entry by name, so the archive's paths start at the app's files and not at `./`.
    let mut entries: Vec<_> = fs::read_dir(staged).map_err(|e| format!("{}: {e}", staged.display()))?.flatten().map(|e| e.file_name()).collect();
    entries.sort();
    command.args(entries);
    exec(command, true)?;
    Ok(out)
}

/// The three files of a winget-pkgs pull request, in the folder they have there.
fn manifest_winget(ctx: &Ctx, installer: &Path) -> Result<PathBuf> {
    let id = format!("{WINGET_PUBLISHER}.{WINGET_NAME}");
    let initial = WINGET_PUBLISHER[..1].to_lowercase();
    let root = ctx.manifests().join("winget");
    empty_dir(&root)?;
    let dir = root.join("manifests").join(initial).join(WINGET_PUBLISHER).join(WINGET_NAME).join(VERSION);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let url = ctx.asset_url(installer);
    let hash = sha256_of(installer)?.to_uppercase();
    let values = [("PACKAGE_ID", id.as_str()), ("ARCH", sdk_arch()), ("URL", url.as_str()), ("SHA256", hash.as_str())];
    for (template, file) in [
        ("version.yaml", format!("{id}.yaml")),
        ("installer.yaml", format!("{id}.installer.yaml")),
        ("locale.yaml", format!("{id}.locale.en-US.yaml")),
    ] {
        write_file(&dir.join(file), &manifest(ctx, &format!("windows/winget/{template}"), &values)?)?;
    }
    Ok(dir)
}

/// The Scoop manifest: `scoop install <its url>`, or a file of a bucket.
fn manifest_scoop(ctx: &Ctx, zip: &Path) -> Result<PathBuf> {
    let dir = ctx.manifests().join("scoop");
    empty_dir(&dir)?;
    let (url, hash) = (ctx.asset_url(zip), sha256_of(zip)?);
    let values = [("ARCH", scoop_arch()), ("FILE_ARCH", sdk_arch()), ("URL", url.as_str()), ("SHA256", hash.as_str())];
    let out = dir.join("bifurc.json");
    write_file(&out, &manifest(ctx, "windows/scoop/bifurc.json", &values)?)?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------
// macOS: disk image
// ---------------------------------------------------------------------------------------------------------

/// `Bifurc.app` in a disk image beside an Applications link. The staged folder is the app's `Contents/MacOS`,
/// so the interface, the engine, the tray helper and `bin/` and `resources/` stay where they look for each other.
/// `bin/bifurc` goes on PATH the first time the engine of this install runs (`runtime/shell_path.rs`).
///
/// Not signed or notarized: a downloaded copy is opened once with right click, Open.
fn package_dmg(ctx: &Ctx, staged: &Path) -> Result<PathBuf> {
    let root = ctx.out_dir.join("dmg-root");
    empty_dir(&root)?;
    let contents = root.join("Bifurc.app").join("Contents");
    let macos = contents.join("MacOS");
    copy_dir(staged, &macos)?;
    mark(&macos, "dmg")?;
    write_file(&contents.join("Info.plist"), &template(ctx, "macos/Info.plist", &[("VERSION", VERSION)])?)?;
    make_icns(ctx, &contents.join("Resources").join("Bifurc.icns"))?;
    symlink("/Applications", &root.join("Applications"))?;

    let out = ctx.out_dir.join(format!("Bifurc-{VERSION}-mac-{}.dmg", mac_arch()));
    let _ = fs::remove_file(&out);
    let mut command = Command::new("hdiutil");
    command.args(["create", "-volname", "Bifurc", "-fs", "HFS+", "-format", "UDZO", "-ov", "-srcfolder"]);
    command.arg(&root).arg(&out).current_dir(&ctx.repo);
    exec(command, false)?;
    // Not left behind: Spotlight would list this copy of the app beside the installed one.
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(ctx.out_dir.join("Bifurc.iconset"));
    Ok(out)
}

/// The app icon in the format macOS wants, from the 512 px PNG.
fn make_icns(ctx: &Ctx, out: &Path) -> Result<()> {
    let iconset = ctx.out_dir.join("Bifurc.iconset");
    empty_dir(&iconset)?;
    let png = ctx.repo.join("build").join("icon.png");
    for (size, name) in [
        (16, "16x16"),
        (32, "16x16@2x"),
        (32, "32x32"),
        (64, "32x32@2x"),
        (128, "128x128"),
        (256, "128x128@2x"),
        (256, "256x256"),
        (512, "256x256@2x"),
        (512, "512x512"),
        (1024, "512x512@2x"),
    ] {
        let mut sips = Command::new("sips");
        sips.args(["-z", &size.to_string(), &size.to_string()]).arg(&png).arg("--out").arg(iconset.join(format!("icon_{name}.png")));
        exec(sips, true)?;
    }
    fs::create_dir_all(out.parent().ok_or("no folder for the icon")?).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut iconutil = Command::new("iconutil");
    iconutil.args(["-c", "icns"]).arg(&iconset).arg("-o").arg(out);
    exec(iconutil, true)
}

// ---------------------------------------------------------------------------------------------------------
// Linux: .deb, tarball, and the AUR package's source
// ---------------------------------------------------------------------------------------------------------

/// The app's five executables, which are what its shared library dependencies are read from.
fn staged_executables(dir: &Path) -> [PathBuf; 5] {
    [
        dir.join("bifurc-ui"),
        dir.join("bifurc-engine"),
        dir.join("bin").join("bifurc"),
        dir.join("resources").join("bifurc-mcp"),
        dir.join("resources").join("bifurc-cli"),
    ]
}

/// The packages the executables' shared libraries come from, as `dpkg-shlibdeps` finds them on this machine.
/// Vulkan is loaded at run time and is in no executable's list, so it is added.
fn deb_depends(ctx: &Ctx, lib: &Path) -> String {
    const FALLBACK: &str = "libc6, libgcc-s1, libasound2, libfontconfig1, libx11-xcb1, libxkbcommon0, libxkbcommon-x11-0, libzstd1";
    // The tool reads a source package's control file from where it runs; an empty one is enough for it.
    let work = ctx.out_dir.join("deb-shlibdeps");
    let found = (|| -> Result<String> {
        empty_dir(&work.join("debian"))?;
        write_file(&work.join("debian").join("control"), "Source: bifurc\n\nPackage: bifurc\nArchitecture: any\n")?;
        let mut command = Command::new("dpkg-shlibdeps");
        command.args(["-O", "--ignore-missing-info"]).current_dir(&work);
        for executable in staged_executables(lib) {
            command.arg(format!("-e{}", executable.display()));
        }
        let output = command.output().map_err(|e| format!("could not run dpkg-shlibdeps: {e}"))?;
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines()
            .find_map(|line| line.strip_prefix("shlibs:Depends="))
            .map(|depends| depends.trim().to_owned())
            .filter(|depends| !depends.is_empty())
            .ok_or_else(|| format!("dpkg-shlibdeps named no dependency\n{}", String::from_utf8_lossy(&output.stderr)))
    })();
    let depends = found.unwrap_or_else(|why| {
        println!("warning: {why}\nwarning: the .deb depends on a fixed list instead: {FALLBACK}");
        FALLBACK.to_owned()
    });
    format!("{depends}, libvulkan1")
}

/// A Debian package: the staged folder under `/usr/lib/bifurc`, `bifurc` in `/usr/bin`, a launcher entry.
fn package_deb(ctx: &Ctx, staged: &Path) -> Result<PathBuf> {
    let root = ctx.out_dir.join("deb-root");
    empty_dir(&root)?;
    let usr = root.join("usr");
    let lib = usr.join("lib").join("bifurc");
    copy_dir(staged, &lib)?;
    mark(&lib, "deb")?;
    symlink("../lib/bifurc/bin/bifurc", &usr.join("bin").join("bifurc"))?;
    let share = usr.join("share");
    fs::create_dir_all(share.join("applications")).map_err(|e| format!("{}: {e}", share.display()))?;
    write_file(
        &share.join("applications").join("bifurc.desktop"),
        &template(ctx, "linux/bifurc.desktop", &[("EXEC", "/usr/lib/bifurc/bifurc-ui")])?,
    )?;
    let icon = ctx.repo.join("build").join("icon.png");
    copy_file(&icon, &share.join("icons").join("hicolor").join("512x512").join("apps").join("bifurc.png"))?;
    copy_file(&ctx.repo.join("LICENSE"), &share.join("doc").join("bifurc").join("copyright"))?;

    let depends = deb_depends(ctx, &lib);
    let installed_kib = dir_size(&usr).div_ceil(1024).to_string();
    let control = root.join("DEBIAN");
    fs::create_dir_all(&control).map_err(|e| format!("{}: {e}", control.display()))?;
    let values = [("ARCH", deb_arch()), ("DEPENDS", depends.as_str()), ("INSTALLED_SIZE", installed_kib.as_str())];
    write_file(&control.join("control"), &manifest(ctx, "linux/deb/control", &values)?)?;
    for script in ["postinst", "prerm"] {
        write_file(&control.join(script), &manifest(ctx, &format!("linux/deb/{script}"), &[])?)?;
        make_executable(&control.join(script))?;
    }

    let out = ctx.out_dir.join(format!("bifurc_{VERSION}_{}.deb", deb_arch()));
    let mut command = Command::new("dpkg-deb");
    command.args(["--root-owner-group", "--build"]).arg(&root).arg(&out).current_dir(&ctx.repo);
    exec(command, false)?;
    Ok(out)
}

/// The staged folder as `bifurc-<version>/` in a tarball, with the launcher entry a package puts in
/// `/usr/share/applications`: what the AUR package installs, and what any other distribution's would.
fn package_tarball(ctx: &Ctx, staged: &Path) -> Result<PathBuf> {
    let parent = ctx.out_dir.join("tarball");
    empty_dir(&parent)?;
    let name = format!("bifurc-{VERSION}");
    copy_dir(staged, &parent.join(&name))?;
    mark(&parent.join(&name), "tarball")?;
    write_file(
        &parent.join(&name).join("bifurc.desktop"),
        &template(ctx, "linux/bifurc.desktop", &[("EXEC", "/usr/lib/bifurc/bifurc-ui")])?,
    )?;
    let out = ctx.out_dir.join(format!("Bifurc-{VERSION}-linux-{}.tar.gz", appimage_arch()));
    let mut command = Command::new("tar");
    command.args(["--owner=0", "--group=0", "-czf"]).arg(&out).arg("-C").arg(&parent).arg(&name).current_dir(&ctx.repo);
    exec(command, false)?;
    Ok(out)
}

/// The AUR package `bifurc-bin`: the files its git repository holds.
fn manifest_aur(ctx: &Ctx, tarball: &Path) -> Result<PathBuf> {
    let dir = ctx.manifests().join("aur");
    empty_dir(&dir)?;
    let (url, hash) = (ctx.asset_url(tarball), sha256_of(tarball)?);
    let values = [("ARCH", appimage_arch()), ("URL", url.as_str()), ("SHA256", hash.as_str())];
    for (template, file) in [("PKGBUILD", "PKGBUILD"), ("SRCINFO", ".SRCINFO"), ("bifurc.install", "bifurc.install")] {
        write_file(&dir.join(file), &manifest(ctx, &format!("linux/aur/{template}"), &values)?)?;
    }
    Ok(dir)
}

// ---------------------------------------------------------------------------------------------------------
// Linux: AppImage
// ---------------------------------------------------------------------------------------------------------

fn find_appimagetool(ctx: &Ctx) -> Result<PathBuf> {
    if let Some(found) = env_path("APPIMAGETOOL").or_else(|| on_path("appimagetool")) {
        return Ok(found);
    }
    let arch = appimage_arch();
    let cache = ctx.out_dir.join("cache");
    let tool = cache.join(format!("appimagetool-{arch}.AppImage"));
    if !tool.exists() {
        fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
        let url = format!("https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-{arch}.AppImage");
        let mut curl = Command::new("curl");
        curl.args(["-fsSL", "-o"]).arg(&tool).arg(&url);
        exec(curl, false)?;
        make_executable(&tool)?;
    }
    Ok(tool)
}

fn package_appimage(ctx: &Ctx, staged: &Path) -> Result<PathBuf> {
    let arch = appimage_arch();
    let app_dir = ctx.out_dir.join("Bifurc.AppDir");
    empty_dir(&app_dir)?;
    copy_dir(staged, &app_dir.join("usr").join("lib").join("bifurc"))?;
    mark(&app_dir.join("usr").join("lib").join("bifurc"), "appimage")?;
    // Rebuilding the same version must still replace what an earlier build of it unpacked.
    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let build_id = format!("{VERSION}-{millis:x}");
    let app_run = app_dir.join("AppRun");
    write_file(&app_run, &template(ctx, "linux/AppRun", &[("BUILD_ID", &build_id)])?)?;
    make_executable(&app_run)?;
    write_file(&app_dir.join("bifurc.desktop"), &template(ctx, "linux/bifurc.desktop", &[("EXEC", "AppRun")])?)?;
    let icon = ctx.repo.join("build").join("icon.png");
    copy_file(&icon, &app_dir.join("bifurc.png"))?;
    copy_file(&icon, &app_dir.join(".DirIcon"))?;
    copy_file(&icon, &app_dir.join("usr").join("share").join("icons").join("hicolor").join("512x512").join("apps").join("bifurc.png"))?;

    let out = ctx.out_dir.join(format!("Bifurc-{VERSION}-{arch}.AppImage"));
    // The tool is itself an AppImage; unpacking instead of mounting spares the build machine FUSE.
    let mut command = Command::new(find_appimagetool(ctx)?);
    command.arg(&app_dir).arg(&out).env("ARCH", arch).env("APPIMAGE_EXTRACT_AND_RUN", "1").current_dir(&ctx.repo);
    exec(command, false)?;
    Ok(out)
}
