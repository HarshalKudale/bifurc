//! Finding the engine: the shared `runtime.json` discovery and default data directory.

pub use bifurc_client::{
    RUNTIME_FILE, RuntimeInfo, find_running_engine, is_process_alive, is_socket_listening, platform_default_data_dir,
    read_runtime_info, runtime_file_path,
};
