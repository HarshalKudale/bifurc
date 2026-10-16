//! What every Bifurc client shares with the engine, so the paths and the wire format cannot drift apart:
//! the default data directory, `<dataDir>/runtime.json` and the length-prefixed socket frames.
//!
//! No async runtime and no application logic. Starting the engine and talking to it stay with each client.

pub mod frame;
pub mod paths;
pub mod runtime;

/// The environment variable that names the engine binary outright.
pub const ENGINE_ENV: &str = "BIFURC_ENGINE";

/// The engine's file name on this platform.
pub fn engine_file_name() -> String {
    format!("bifurc-engine{}", std::env::consts::EXE_SUFFIX)
}

pub use paths::{DATA_DIR_ENV, default_data_dir, platform_default_data_dir};
pub use runtime::{
    RUNTIME_FILE, RuntimeInfo, find_running_engine, is_process_alive, is_socket_listening, read_runtime_info,
    runtime_file_path,
};
