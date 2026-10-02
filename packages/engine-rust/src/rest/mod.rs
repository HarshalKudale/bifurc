//! REST requests: replaying a request, parsing cURL, building a request from captured traffic.

pub mod commands;
pub mod request_body;

use crate::kernel::CommandRegistry;

pub fn register(registry: &mut CommandRegistry) {
    commands::register(registry);
}
