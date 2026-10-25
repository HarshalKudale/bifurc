//! The `cloudflared` program that makes a local port reachable from the internet for a few minutes (a Cloudflare
//! quick tunnel, no account), as webhook capture needs. Onboarding installs it into `<data dir>/tools/` with
//! `tunnel.install`; `tunnel.status` says whether capture can run.

use std::{
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use ring::digest;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};

use crate::{
    kernel::CommandRegistry,
    protocol::{EngineError, Result},
    workspace::paths::{data_dir, is_data_root_set},
};

const RELEASE: &str = "https://api.github.com/repos/cloudflare/cloudflared/releases/latest";

/// How long cloudflared is given to announce its address.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

/// The whole download, program included.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

/// The release file for this computer.
fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        // Windows on ARM runs the x64 build.
        ("windows", "x86_64" | "aarch64") => Some("cloudflared-windows-amd64.exe"),
        ("windows", "x86") => Some("cloudflared-windows-386.exe"),
        ("linux", "x86_64") => Some("cloudflared-linux-amd64"),
        ("linux", "aarch64") => Some("cloudflared-linux-arm64"),
        ("linux", "arm") => Some("cloudflared-linux-arm"),
        ("macos", "x86_64") => Some("cloudflared-darwin-amd64.tgz"),
        ("macos", "aarch64") => Some("cloudflared-darwin-arm64.tgz"),
        _ => None,
    }
}

fn program_name() -> &'static str {
    if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" }
}

/// Where the installed copy lives.
fn installed_path() -> PathBuf {
    data_dir().join("tools").join(program_name())
}

/// The program in a `PATH`-style list of directories.
fn on_path(paths: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(paths).map(|dir| dir.join(program_name())).find(|path| path.is_file())
}

/// The program to run: `BIFURC_CLOUDFLARED`, else the installed copy, else one on the `PATH`.
pub fn find() -> Option<PathBuf> {
    let named = std::env::var_os("BIFURC_CLOUDFLARED").map(PathBuf::from).filter(|path| path.is_file());
    named
        .or_else(|| is_data_root_set().then(installed_path).filter(|path| path.is_file()))
        .or_else(|| std::env::var_os("PATH").and_then(|paths| on_path(&paths)))
}

fn status() -> Value {
    let found = find();
    json!({
        "installed": found.is_some(),
        "path": found.map(|path| path.display().to_string()),
        "supported": asset_name().is_some(),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The file called `wanted` in a gzipped tar, which a release of one program holds.
fn untar(archive: &[u8], wanted: &str) -> Option<Vec<u8>> {
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(archive).read_to_end(&mut tar).ok()?;
    let mut at = 0;
    while at + 512 <= tar.len() {
        let head = &tar[at..at + 512];
        let name = String::from_utf8_lossy(head.get(..100)?);
        let name = name.trim_end_matches('\0');
        if name.is_empty() {
            return None;
        }
        let size = usize::from_str_radix(String::from_utf8_lossy(&head[124..135]).trim_matches(['\0', ' ']), 8).ok()?;
        let data = tar.get(at + 512..at + 512 + size)?;
        if name.rsplit('/').next() == Some(wanted) {
            return Some(data.to_vec());
        }
        at += 512 + size.div_ceil(512) * 512;
    }
    None
}

fn upstream(what: &str, error: impl std::fmt::Display) -> EngineError {
    EngineError::upstream(format!("cloudflared {what}: {error}"))
}

/// Downloads the program from its GitHub release and installs it, after checking it against the checksum the
/// release lists for it. A release that lists none is not trusted.
pub(crate) async fn install() -> Result<Value> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let asset = asset_name().ok_or_else(|| {
        EngineError::bad_request(format!(
            "there is no cloudflared build for {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .user_agent("bifurc")
        .build()
        .map_err(|error| upstream("could not start the download", error))?;
    let fetch = |url: String| {
        let client = client.clone();
        async move {
            let response = client.get(&url).send().await.and_then(reqwest::Response::error_for_status);
            match response {
                Ok(response) => response.bytes().await.map_err(|error| upstream("download failed", error)),
                Err(error) => Err(upstream("download failed", error)),
            }
        }
    };
    let release: Value = serde_json::from_slice(&fetch(RELEASE.to_string()).await?)
        .map_err(|error| upstream("release could not be read", error))?;
    let entry = release["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|entry| entry["name"] == asset))
        .ok_or_else(|| upstream("release", format!("has no {asset}")))?;
    let expected = entry["digest"]
        .as_str()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or_else(|| upstream("release", format!("lists no checksum for {asset}, so it was not installed")))?;
    let url = entry["browser_download_url"].as_str().ok_or_else(|| upstream("release", "lists no address"))?;
    let bytes = fetch(url.to_string()).await?;
    if hex(digest::digest(&digest::SHA256, &bytes).as_ref()) != expected {
        return Err(upstream("download", format!("does not match the checksum of {asset}, so it was not installed")));
    }
    let program = if asset.ends_with(".tgz") {
        untar(&bytes, "cloudflared").ok_or_else(|| upstream("download", "holds no cloudflared"))?
    } else {
        bytes.to_vec()
    };
    let target = installed_path();
    write_program(&target, &program)
        .map_err(|error| EngineError::engine(format!("cannot install cloudflared: {error}")))?;
    Ok(status())
}

fn write_program(target: &Path, program: &[u8]) -> std::io::Result<()> {
    let dir = target.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let partial = target.with_extension("partial");
    std::fs::write(&partial, program)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755))?;
    }
    // A program that is running cannot be replaced on Windows, but it can be moved aside.
    let _ = std::fs::remove_file(target);
    std::fs::rename(&partial, target)
}

/// The address in a line of cloudflared's log, if the line announces the quick tunnel.
fn url_in(line: &str) -> Option<String> {
    line.match_indices("https://").find_map(|(at, _)| {
        let host: String =
            line[at + 8..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.').collect();
        (host.ends_with(".trycloudflare.com") && host != "api.trycloudflare.com").then(|| format!("https://{host}"))
    })
}

/// A quick tunnel to a local port, closed when this is dropped.
pub struct Tunnel {
    pub url: String,
    _child: Child,
}

/// Starts cloudflared and waits for the public address it gives the port.
pub async fn open(port: u16) -> Result<Tunnel> {
    let program = find().ok_or_else(|| {
        EngineError::not_found("cloudflared is not installed; install it from the setup or settings")
            .keyed("tunnel.not_installed", json!({}))
    })?;
    let mut command = Command::new(program);
    command
        .args(["tunnel", "--no-autoupdate", "--url", &format!("http://127.0.0.1:{port}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child =
        command.spawn().map_err(|error| EngineError::engine(format!("cannot start cloudflared: {error}")))?;
    let stderr = child.stderr.take().ok_or_else(|| EngineError::engine("cloudflared gave no output"))?;
    let mut lines = BufReader::new(stderr).lines();
    let announced = tokio::time::timeout(OPEN_TIMEOUT, async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(url) = url_in(&line) {
                return Some(url);
            }
        }
        None
    })
    .await;
    let Ok(Some(url)) = announced else {
        return Err(EngineError::upstream("cloudflared did not give a public address; is this computer online?")
            .keyed("tunnel.no_address", json!({})));
    };
    // The pipe stays read, so a log line never blocks the program.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    Ok(Tunnel { url, _child: child })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("tunnel.status", |_, _| Ok(status()));
    registry.register("tunnel.install", |_, _| async move { install().await });
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn the_quick_tunnel_address_is_found_in_the_log_and_the_api_host_is_not_it() {
        let boxed = "2026-10-21T10:00:00Z INF |  https://calm-river-fox-1234.trycloudflare.com                 |";
        assert_eq!(url_in(boxed).as_deref(), Some("https://calm-river-fox-1234.trycloudflare.com"));
        assert_eq!(url_in("INF Requesting new quick Tunnel on https://api.trycloudflare.com..."), None);
        assert_eq!(url_in("INF Registered tunnel connection connIndex=0"), None);
        assert_eq!(
            url_in("see https://example.com and https://a-b.trycloudflare.com/x").as_deref(),
            Some("https://a-b.trycloudflare.com")
        );
    }

    #[test]
    fn a_program_is_taken_out_of_a_gzipped_tar() {
        let program = b"MZ not really a program".to_vec();
        let mut tar = Vec::new();
        for (name, data) in [("README", b"hi".to_vec()), ("./cloudflared", program.clone())] {
            let mut head = [0u8; 512];
            head[..name.len()].copy_from_slice(name.as_bytes());
            head[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            tar.extend_from_slice(&head);
            tar.extend_from_slice(&data);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        }
        tar.extend_from_slice(&[0u8; 1024]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        let archive = gz.finish().unwrap();
        assert_eq!(untar(&archive, "cloudflared"), Some(program));
        assert_eq!(untar(&archive, "nothing"), None);
        assert_eq!(untar(b"not a gzip", "cloudflared"), None);
    }

    #[test]
    fn a_checksum_is_the_lowercase_hex_of_sha256() {
        let sum = hex(digest::digest(&digest::SHA256, b"abc").as_ref());
        assert_eq!(sum, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn the_program_is_found_on_a_path_and_a_missing_one_is_not() {
        let dir = std::env::temp_dir().join(format!("bifurc-tunnel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = std::env::join_paths([dir.clone()]).unwrap();
        assert_eq!(on_path(&paths), None);
        std::fs::write(dir.join(program_name()), b"x").unwrap();
        assert_eq!(on_path(&paths), Some(dir.join(program_name())));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_program_is_written_whole_and_replaces_the_old_one() {
        let dir = std::env::temp_dir().join(format!("bifurc-tunnel-write-{}", std::process::id()));
        let target = dir.join("tools").join(program_name());
        write_program(&target, b"one").unwrap();
        write_program(&target, b"two").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"two");
        assert!(!target.with_extension("partial").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn this_computer_has_a_build_to_install() {
        assert!(asset_name().is_some());
    }
}
