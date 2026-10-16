//! Mocks: which one answers a request, and the response it builds. The mock entities themselves are
//! managed by the entity commands, `capture.buildMock` lives with capture and `script.execute` with the
//! script sandbox, so this module registers no commands of its own.

mod handler;

pub use handler::{
    ResolvedMockResponse, UpstreamResponse, is_fully_mocked, is_streaming_mock, match_mock, merge_mock_with_upstream,
    resolve_mock_only_response, serve_mock, serve_resolved_response, serve_streaming_mock, status_text, write_response,
};

use crate::kernel::CommandRegistry;

pub fn register(_registry: &mut CommandRegistry) {}
