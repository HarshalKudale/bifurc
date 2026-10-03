//! REST requests: replaying a request, parsing cURL, building a request from captured traffic, and the
//! `request.send` pipeline.

pub mod commands;
pub mod direct_http;
pub mod pipeline;
pub mod request_body;
pub mod send_commands;
#[cfg(test)]
pub(crate) mod test_server;

use crate::kernel::CommandRegistry;

pub fn register(registry: &mut CommandRegistry) {
    commands::register(registry);
    send_commands::register(registry);
}
