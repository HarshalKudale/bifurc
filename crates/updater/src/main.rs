//! `bifurc-updater`: replaces the installed Bifurc with a newer release, and starts it.
//!
//! The engine decides that there is an update and what it is (`crates/engine/src/updates`); this program only
//! carries it out. It is packaged with the installs that can be replaced in place (the installer, winget, Scoop,
//! the zip, AppImage, deb and tarball) and left out of the Microsoft Store package, whose files the Store owns:
//! an engine that finds no updater beside itself sends the person to the Store or the website instead.
//!
//! ```text
//! bifurc-updater apply --package nsis|zip|tarball|appimage|deb --url <file> --sha256 <hex> --version <x.y.z>
//!                      --app-dir <install folder> --launch <bifurc-ui> --data-dir <data dir> [--target <AppImage>]
//! ```
//!
//! It copies itself to the temp folder first (the folder it was started from is about to be overwritten), then
//! downloads the file while Bifurc keeps running, checks its SHA-256, stops the engine and the interfaces with
//! `bifurc engine stop`, installs, and starts the interface again. Where it is in that is written to
//! `update-progress.json` in the data dir, which the engine reads and announces. A failed update leaves the
//! installed files as they were and starts the interface again if it had been stopped.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::{
    env,
    ffi::OsStr,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::json;
use sha2::{Digest, Sha256};

/// Where a download may come from: the releases repository and the website, the same as the engine allows.
const TRUSTED_PREFIXES: [&str; 2] = ["https://github.com/HarshalKudale/bifurc-releases/", "https://bifurc.app/"];
const PROGRESS_FILE: &str = "update-progress.json";
const EXE: &str = env::consts::EXE_SUFFIX;
const PACKAGES: [&str; 5] = ["nsis", "zip", "tarball", "appimage", "deb"];

struct Job {
    package: String,
    url: String,
    sha256: String,
    version: String,
    app_dir: PathBuf,
    launch: PathBuf,
    data_dir: PathBuf,
    /// The AppImage file itself, when the package is one.
    target: Option<PathBuf>,
}

impl Job {
    fn parse(args: &[String]) -> Option<Self> {
        if args.first().map(String::as_str) != Some("apply") {
            return None;
        }
        let value = |name: &str| args.iter().position(|arg| arg == name).and_then(|at| args.get(at + 1)).cloned();
        Some(Self {
            package: value("--package")?,
            url: value("--url")?,
            sha256: value("--sha256")?,
            version: value("--version")?,
            app_dir: value("--app-dir")?.into(),
            launch: value("--launch")?.into(),
            data_dir: value("--data-dir")?.into(),
            target: value("--target").map(PathBuf::from),
        })
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        println!("bifurc-updater {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let Some(job) = Job::parse(&args) else {
        eprintln!(
            "usage: bifurc-updater apply --package <p> --url <u> --sha256 <h> --version <v> --app-dir <d> --launch <ui> --data-dir <d> [--target <file>]"
        );
        return ExitCode::from(2);
    };
    if !args.iter().any(|arg| arg == "--relocated")
        && let Some(code) = relocate(&args)
    {
        return code;
    }
    run(&job)
}

/// A program that writes no window of its own: the helper commands this runs must not flash a console.
fn quiet(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Starts `command` apart from this process, so it outlives it and is not stopped with it.
fn detach(command: &mut Command) -> std::io::Result<std::process::Child> {
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()
}

/// Runs a copy of this program from the temp folder and returns, so nothing is running from the install
/// folder while it is replaced. `None` when the copy could not be made: the job then runs from where it is.
fn relocate(args: &[String]) -> Option<ExitCode> {
    let exe = env::current_exe().ok()?;
    let dir = env::temp_dir().join(format!("bifurc-updater-{}", std::process::id()));
    fs::create_dir_all(&dir).ok()?;
    let copy = dir.join(format!("bifurc-updater{EXE}"));
    fs::copy(&exe, &copy).ok()?;
    detach(Command::new(&copy).args(args).arg("--relocated")).ok()?;
    Some(ExitCode::SUCCESS)
}

fn progress(job: &Job, state: &str, received: u64, total: u64, error: Option<&str>) {
    let body = json!({ "state": state, "version": job.version, "received": received, "total": total, "error": error });
    let _ = fs::create_dir_all(&job.data_dir);
    let _ = fs::write(job.data_dir.join(PROGRESS_FILE), body.to_string());
}

fn run(job: &Job) -> ExitCode {
    let mut stopped = false;
    // A panic is a failure like any other: it is written down, and the interface is started again.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply(job, &mut stopped)))
        .unwrap_or_else(|_| Err("the updater stopped unexpectedly".to_string()));
    let _ = fs::remove_dir_all(job.data_dir.join("update"));
    match &outcome {
        Ok(()) => progress(job, "done", 0, 0, None),
        Err(error) => progress(job, "failed", 0, 0, Some(error)),
    }
    // Whatever happened, a person who had Bifurc open is left with it open.
    if stopped {
        launch(job);
    }
    if outcome.is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn apply(job: &Job, stopped: &mut bool) -> Result<(), String> {
    if !TRUSTED_PREFIXES.iter().any(|prefix| job.url.starts_with(prefix)) {
        return Err(format!("{} is not where releases are published", job.url));
    }
    if !PACKAGES.contains(&job.package.as_str()) {
        return Err(format!("a {} package is not updated by the updater", job.package));
    }
    let file = download_path(job)?;
    progress(job, "downloading", 0, 0, None);
    download(job, &file)?;
    progress(job, "installing", 0, 0, None);
    *stopped = true;
    stop_app(job);
    match job.package.as_str() {
        "nsis" => run_installer(&file, &job.app_dir),
        "zip" => extract(&file, &job.app_dir, false),
        "tarball" => extract(&file, &job.app_dir, true),
        "appimage" => replace_appimage(&file, job.target.as_deref()),
        _ => install_deb(&file),
    }
}

/// Where the download goes: beside the AppImage it replaces (so the swap is one rename), else in the data dir.
fn download_path(job: &Job) -> Result<PathBuf, String> {
    if job.package == "appimage" {
        let mut name = job.target.clone().ok_or("this AppImage did not say where it is (APPIMAGE)")?.into_os_string();
        name.push(".new");
        return Ok(PathBuf::from(name));
    }
    let dir = job.data_dir.join("update");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name: String = job
        .url
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    Ok(dir.join(if name.is_empty() { "download" } else { &name }))
}

/// The file at `job.url` into `to`, with its SHA-256 checked. A file that does not match is deleted.
fn download(job: &Job, to: &Path) -> Result<(), String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start: {e}"))?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("bifurc-updater/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        let mut response =
            client.get(&job.url).send().await.map_err(|e| format!("{} could not be reached: {e}", job.url))?;
        if !response.status().is_success() {
            return Err(format!("{} answered {}", job.url, response.status()));
        }
        let total = response.content_length().unwrap_or(0);
        let mut file = fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
        let (mut hasher, mut received, mut reported) = (Sha256::new(), 0u64, None::<Instant>);
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(30), response.chunk())
                .await
                .map_err(|_| "the download stalled".to_string())?
                .map_err(|e| format!("the download failed: {e}"))?;
            let Some(chunk) = chunk else { break };
            hasher.update(&chunk);
            file.write_all(&chunk).map_err(|e| format!("{}: {e}", to.display()))?;
            received += chunk.len() as u64;
            if reported.is_none_or(|at| at.elapsed() >= Duration::from_millis(250)) {
                progress(job, "downloading", received, total, None);
                reported = Some(Instant::now());
            }
        }
        file.flush().map_err(|e| format!("{}: {e}", to.display()))?;
        drop(file);
        if !hex::encode(hasher.finalize()).eq_ignore_ascii_case(job.sha256.trim()) {
            let _ = fs::remove_file(to);
            return Err("the download does not match its checksum, so it was not installed".to_string());
        }
        Ok(())
    })
}

/// Stops the engine, which closes the interfaces attached to it. `--no-start` keeps the command from starting
/// an engine to stop.
fn stop_app(job: &Job) {
    let cli = job.app_dir.join("bin").join(format!("bifurc{EXE}"));
    if cli.exists() {
        let _ = quiet(cli).args(["engine", "stop", "--no-start"]).env("BIFURC_DATA_DIR", &job.data_dir).status();
    }
    thread::sleep(Duration::from_secs(1));
}

#[cfg(windows)]
fn run_installer(file: &Path, app_dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // `/D=` must come last and unquoted, which is what `raw_arg` is for.
    let status = quiet(file)
        .arg("/S")
        .raw_arg(format!("/D={}", app_dir.display()))
        .status()
        .map_err(|e| format!("could not run the installer: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("the installer stopped with {status}")) }
}

#[cfg(not(windows))]
fn run_installer(_: &Path, _: &Path) -> Result<(), String> {
    Err("the installer runs on Windows only".to_string())
}

/// `file` unpacked over `to`. A tarball holds one folder, which `strip` unwraps. A file the stopped app still
/// holds is let go a moment later, so a failure is tried again.
fn extract(file: &Path, to: &Path, strip: bool) -> Result<(), String> {
    let tar = if cfg!(windows) {
        // Named by its path: a `tar` earlier on PATH may be GNU tar, which does not read a zip.
        env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32").join("tar.exe"))
            .filter(|tar| tar.exists())
            .ok_or("tar.exe was not found in System32")?
    } else {
        PathBuf::from("tar")
    };
    let mut reason = String::new();
    for _ in 0..10 {
        let mut command = quiet(&tar);
        command.arg("-xf").arg(file).arg("-C").arg(to);
        if strip {
            command.arg("--strip-components=1");
        }
        match command.output() {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => reason = String::from_utf8_lossy(&output.stderr).trim().to_string(),
            Err(e) => return Err(format!("could not run tar: {e}")),
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err(format!("could not unpack the update: {reason}"))
}

#[cfg(unix)]
fn replace_appimage(file: &Path, target: Option<&Path>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let target = target.ok_or("this AppImage did not say where it is (APPIMAGE)")?;
    fs::set_permissions(file, fs::Permissions::from_mode(0o755)).map_err(|e| format!("{}: {e}", file.display()))?;
    fs::rename(file, target).map_err(|e| format!("{} could not be replaced: {e}", target.display()))
}

#[cfg(not(unix))]
fn replace_appimage(_: &Path, _: Option<&Path>) -> Result<(), String> {
    Err("an AppImage is replaced on Linux only".to_string())
}

/// A package of the system: installing it needs a privileged account, which polkit asks the person for.
fn install_deb(file: &Path) -> Result<(), String> {
    let status = Command::new("pkexec")
        .args(["apt-get", "install", "-y"])
        .arg(file)
        .status()
        .map_err(|e| format!("could not ask for permission to install (pkexec): {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("the package was not installed: {status}")) }
}

fn launch(job: &Job) {
    let mut command = Command::new(job.target.as_ref().unwrap_or(&job.launch));
    command.env("BIFURC_DATA_DIR", &job.data_dir);
    // The AppImage that ran this one set these for itself, not for the next.
    for name in ["APPIMAGE", "APPDIR", "ARGV0", "OWD"] {
        command.env_remove(name);
    }
    let _ = detach(&mut command);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(extra: &[&str]) -> Vec<String> {
        let mut args: Vec<String> = [
            "apply",
            "--package",
            "zip",
            "--url",
            "https://bifurc.app/files/Bifurc 1.zip?x=1",
            "--sha256",
            "ab",
            "--version",
            "0.7.0",
            "--app-dir",
            "C:/app",
            "--launch",
            "C:/app/bifurc-ui",
            "--data-dir",
            "C:/data",
        ]
        .map(String::from)
        .into();
        args.extend(extra.iter().map(|arg| (*arg).to_string()));
        args
    }

    #[test]
    fn a_job_is_read_from_its_arguments() {
        let job = Job::parse(&args(&["--target", "/home/me/Bifurc.AppImage"])).expect("a job");
        assert_eq!((job.package.as_str(), job.version.as_str()), ("zip", "0.7.0"));
        assert_eq!(job.target, Some(PathBuf::from("/home/me/Bifurc.AppImage")));
        assert!(Job::parse(&["check".to_string()]).is_none());
        assert!(Job::parse(&args(&[])[..6]).is_none());
    }

    #[test]
    fn a_download_is_named_after_the_file_and_nothing_else() {
        let job = Job::parse(&args(&[])).unwrap();
        let dir = tempfile_dir();
        let job = Job { data_dir: dir.clone(), ..job };
        assert_eq!(download_path(&job).unwrap(), dir.join("update").join("Bifurc1.zip"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_download_from_anywhere_else_is_refused() {
        let job = Job { url: "https://example.com/Bifurc.zip".into(), ..Job::parse(&args(&[])).unwrap() };
        let mut stopped = false;
        assert!(apply(&job, &mut stopped).unwrap_err().contains("not where releases are published"));
        assert!(!stopped);
    }

    fn tempfile_dir() -> PathBuf {
        let dir = env::temp_dir().join(format!("bifurc-updater-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
