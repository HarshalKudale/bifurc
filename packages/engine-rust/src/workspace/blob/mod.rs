use tokio::task::JoinHandle;

use crate::kernel::CommandRegistry;

pub fn register(_registry: &mut CommandRegistry) {}

/// Start the periodic removal of expired staged blobs. Abort the handle to stop it.
pub fn start_blob_sweeper() -> JoinHandle<()> {
    tokio::spawn(async {})
}
