//! Mocks: which one answers a request, and the response it builds. The mock entities themselves are
//! managed by the entity commands, `capture.buildMock` lives with capture and `script.execute` with the
//! script sandbox; the commands here are the local mock server's (`mocks.serve*`).

mod handler;
mod serve;

pub use handler::{
    ResolvedMockResponse, UpstreamResponse, is_fully_mocked, is_streaming_mock, match_mock, merge_mock_with_upstream,
    resolve_mock_only_response, serve_mock, serve_resolved_response, serve_streaming_mock, status_text, write_response,
};

pub use serve::stop_owned_by;

use crate::kernel::CommandRegistry;

pub fn register(registry: &mut CommandRegistry) {
    serve::register(registry);
}
