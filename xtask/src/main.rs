//! Packages the desktop app — the Rust UI, the Rust engine and the Rust CLI — as one install.
//!
//! ```text
//! cargo xtask package                       # this platform's packages
//! cargo xtask package --target nsis,msix    # windows: installer exe, Store package
//! cargo xtask package --target appimage     # linux
//! cargo xtask package --target dir          # the staged folder only
//! cargo xtask package --skip-build          # package what is already built
//! ```
//!
//! Every package is the same folder (`release/desktop/<platform>-unpacked`) in a different wrapper:
//!
//! ```text
//! bifurc-ui[.exe]      the interface
//! bifurc-engine[.exe]  the engine
//! bifurc-tray.exe      the engine's tray icon (Windows)
//! bin/bifurc[.exe]     the `bifurc` command — the only folder that goes on PATH; runs resources/bifurc-cli
//! resources/bifurc-cli[.exe]   the CLI (`packages/cli-rust`)
//! ```
//!
//! `ui/crates/engine-client/src/sidecar.rs` and `ui/crates/cli/src/main.rs` look for exactly these paths
//! beside their own executable, and the engine finds the tray helper beside itself, so the folder works from
//! wherever it is put and no installer has to write a configuration.

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

struct Ctx {
    repo: PathBuf,
    out_dir: PathBuf,
}

impl Ctx {
    fn packaging(&self) -> PathBuf {
        self.repo.join("packaging")
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
        _ => return Err("usage: cargo xtask package [--target dir,nsis,msix,appimage] [--skip-build]".into()),
    }
    let args: Vec<String> = args.collect();
    let skip_build = args.iter().any(|a| a == "--skip-build");
    let spec = match args.iter().position(|a| a == "--target") {
        Some(at) => args.get(at + 1).ok_or("--target needs a value")?.as_str(),
        None if WINDOWS => "nsis,msix",
        None => "appimage",
    };
    let targets: Vec<&str> = spec.split(',').filter(|t| !t.is_empty()).collect();
    for target in &targets {
        let platform = match *target {
            "dir" => None,
            "nsis" | "msix" => Some(true),
            "appimage" => Some(false),
            other => return Err(format!("unknown target \"{other}\" (dir, nsis, msix, appimage)")),
        };
        if let Some(windows_only) = platform
            && windows_only != WINDOWS
        {
            return Err(format!("{target} is built on {}; this is {}", if windows_only { "windows" } else { "linux" }, env::consts::OS));
        }
    }

    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("no repository above xtask")?.to_path_buf();
    let ctx = Ctx { out_dir: repo.join("release").join("desktop"), repo };

    if !skip_build {
        build(&ctx)?;
    }
    let staged = stage(&ctx)?;
    let mut made = vec![staged.clone()];
    if targets.iter().any(|t| *t == "nsis") {
        made.push(package_nsis(&ctx, &staged)?);
    }
    if targets.iter().any(|t| *t == "msix") {
        made.push(package_msix(&ctx, &staged)?);
    }
    if targets.iter().any(|t| *t == "appimage") {
        made.push(package_appimage(&ctx, &staged)?);
    }
    println!("\nBifurc {VERSION}:");
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
    let rustflags = release_rustflags(ctx);
    // The engine and the CLI: the repository's own cargo workspace.
    let mut engine = Command::new("cargo");
    engine
        .args(["build", "--release", "--locked", "-p", "bifurc-engine", "-p", "bifurc-cli"])
        .env("RUSTFLAGS", &rustflags)
        .current_dir(&ctx.repo);
    exec(engine, false)?;
    // The UI workspace: the app, which builds the tray helper beside it (`ui/crates/app/build.rs`), and the
    // `bifurc` launcher that goes on PATH.
    let mut ui = Command::new("cargo");
    ui.args(["build", "--release", "--locked", "-p", "bifurc-ui", "-p", "bifurc-cli"])
        .env("RUSTFLAGS", &rustflags)
        .current_dir(ctx.repo.join("ui"));
    exec(ui, false)
}

/// A release build from `workspace` (the repository root, or `ui`).
fn cargo_output(ctx: &Ctx, name: &str, root: bool) -> Result<PathBuf> {
    let base = if root { ctx.repo.clone() } else { ctx.repo.join("ui") };
    let target_dir = env_path("CARGO_TARGET_DIR").map_or_else(|| base.join("target"), |dir| base.join(dir));
    let file = target_dir.join("release").join(name);
    if file.exists() { Ok(file) } else { Err(format!("{} is missing — build first (drop --skip-build)", file.display())) }
}

// ---------------------------------------------------------------------------------------------------------
// The staged folder
// ---------------------------------------------------------------------------------------------------------

fn stage(ctx: &Ctx) -> Result<PathBuf> {
    let dir = ctx.out_dir.join(if WINDOWS { "win-unpacked" } else { "linux-unpacked" });
    empty_dir(&dir)?;
    copy_file(&cargo_output(ctx, &format!("bifurc-ui{EXE}"), false)?, &dir.join(format!("bifurc-ui{EXE}")))?;
    if WINDOWS {
        copy_file(&cargo_output(ctx, "bifurc-tray.exe", false)?, &dir.join("bifurc-tray.exe"))?;
    }
    // The launcher (ui workspace) is `bin/bifurc`; the CLI it runs (repository workspace) and the engine are the others.
    copy_file(&cargo_output(ctx, &format!("bifurc{EXE}"), false)?, &dir.join("bin").join(format!("bifurc{EXE}")))?;
    copy_file(&cargo_output(ctx, &format!("bifurc{EXE}"), true)?, &dir.join("resources").join(format!("bifurc-cli{EXE}")))?;
    copy_file(&cargo_output(ctx, &format!("bifurc-engine{EXE}"), true)?, &dir.join(format!("bifurc-engine{EXE}")))?;
    copy_file(&ctx.repo.join("build").join("icon.png"), &dir.join("resources").join("icon.png"))?;
    copy_file(&ctx.repo.join("LICENSE"), &dir.join("LICENSE"))?;
    for file in [format!("bifurc-ui{EXE}"), format!("bifurc-engine{EXE}"), format!("bin/bifurc{EXE}"), format!("resources/bifurc-cli{EXE}")] {
        make_executable(&dir.join(file))?;
    }
    smoke_test(&dir)?;
    Ok(dir)
}

/// The staged `bifurc` must reach the staged CLI, and the staged engine must run, before anything wraps them.
fn smoke_test(dir: &Path) -> Result<()> {
    let cli = Command::new(dir.join("bin").join(format!("bifurc{EXE}"))).args(["commands", "--json"]).output();
    match cli {
        Ok(out) if out.status.success() => {}
        Ok(out) => return Err(format!("the staged CLI does not run:\n{}", String::from_utf8_lossy(&out.stderr))),
        Err(e) => return Err(format!("the staged CLI does not run:\n{e}")),
    }
    let engine = Command::new(dir.join(format!("bifurc-engine{EXE}"))).arg("--help").output();
    match engine {
        Ok(out) if out.status.success() => {}
        Ok(out) => return Err(format!("the staged engine does not run:\n{}", String::from_utf8_lossy(&out.stderr))),
        Err(e) => return Err(format!("the staged engine does not run:\n{e}")),
    }
    println!("staged CLI and engine start");
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Windows: installer
// ---------------------------------------------------------------------------------------------------------

fn find_makensis() -> Result<PathBuf> {
    let builder_cache = env_path("LOCALAPPDATA").map(|d| d.join("electron-builder").join("Cache").join("nsis"));
    let cached: Vec<Option<PathBuf>> = builder_cache
        .and_then(|dir| fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| Some(entry.path().join("makensis.exe")))
        .collect();
    first_existing(
        [env_path("MAKENSIS"), on_path("makensis"), env_path("ProgramFiles(x86)").map(|d| d.join("NSIS").join("makensis.exe")), env_path("ProgramFiles").map(|d| d.join("NSIS").join("makensis.exe"))]
            .into_iter()
            .chain(cached),
    )
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
