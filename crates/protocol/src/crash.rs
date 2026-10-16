//! Crash reports: what a Bifurc process leaves in the data directory when it panics, or when something it
//! needs fails before an engine is there to be told.
//!
//! A report is one JSON file in `<data dir>/crashes/`, named `<ms>-<process>-<pid>.json`:
//!
//! ```json
//! { "kind": "panic", "process": "engine", "version": "0.5.0", "os": "windows", "arch": "x86_64",
//!   "at": 1760000000000, "thread": "main", "location": "bifurc/crates/engine/src/main.rs:53:9",
//!   "message": "…", "backtrace": "…" }
//! { "kind": "failure", "process": "desktop", "version": "0.5.0", …, "what": "engine_start" }
//! ```
//!
//! The file stays on this machine, whole: it is what a user attaches to an issue. The engine reads the
//! reports and sends on only where in Bifurc's own code it happened, never the message or the backtrace
//! (`analytics` in `crates/engine`).

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

/// The folder of the reports, inside the data directory.
pub const DIR: &str = "crashes";

fn write(data_dir: &Path, process: &str, version: &str, kind: &str, more: Value) {
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64);
    let mut report = json!({
        "kind": kind, "process": process, "version": version,
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "at": at,
    });
    if let (Some(report), Some(more)) = (report.as_object_mut(), more.as_object()) {
        report.extend(more.clone());
    }
    let dir = data_dir.join(DIR);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("{at}-{process}-{}.json", std::process::id())), report.to_string());
}

/// Leave a report whenever this process panics, then panic as it would have. Call it once, first thing.
pub fn install(process: &'static str, version: &'static str, data_dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()));
        let more = json!({
            "thread": std::thread::current().name(),
            "location": location,
            "message": info.payload_as_str(),
            "backtrace": std::backtrace::Backtrace::force_capture().to_string(),
        });
        write(&data_dir, process, version, "panic", more);
        previous(info);
    }));
}

/// Leave a report of something that failed without a panic. `what` is a fixed word (`engine_start`), never
/// the error's text.
pub fn failure(data_dir: &Path, process: &str, version: &str, what: &str) {
    write(data_dir, process, version, "failure", json!({ "what": what }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_is_one_file_in_the_crashes_folder() {
        let dir = std::env::temp_dir().join(format!("bifurc-crash-test-{}", std::process::id()));
        failure(&dir, "desktop", "0.5.0", "engine_start");
        let files: Vec<_> = std::fs::read_dir(dir.join(DIR)).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);
        let report: Value = serde_json::from_str(&std::fs::read_to_string(files[0].path()).unwrap()).unwrap();
        assert_eq!(
            (&report["kind"], &report["process"], &report["what"]),
            (&json!("failure"), &json!("desktop"), &json!("engine_start"))
        );
        assert!(report["at"].as_u64().unwrap() > 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_panic_leaves_a_report_of_where_and_why() {
        let dir = std::env::temp_dir().join(format!("bifurc-panic-test-{}", std::process::id()));
        install("engine", "0.5.0", dir.clone());
        let line = line!() + 1;
        assert!(std::panic::catch_unwind(|| panic!("no such thing: {}", 7)).is_err());
        let _ = std::panic::take_hook();
        let files: Vec<_> = std::fs::read_dir(dir.join(DIR)).unwrap().flatten().collect();
        let report: Value = serde_json::from_str(&std::fs::read_to_string(files[0].path()).unwrap()).unwrap();
        assert_eq!((&report["kind"], &report["message"]), (&json!("panic"), &json!("no such thing: 7")));
        let location = report["location"].as_str().unwrap().replace('\\', "/");
        assert!(location.contains(&format!("src/crash.rs:{line}:")), "{location}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
