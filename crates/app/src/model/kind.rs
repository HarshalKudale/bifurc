use crate::i18n::t;

/// The request types the engine can store and send. WebSocket is one of them, not a separate feature:
/// it has a saved entity and an id-addressed command family exactly like the others, and it is kept in the
/// same folders: the request folders are every type's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestKind {
    Rest,
    GraphQl,
    Grpc,
    Soap,
    WebSocket,
    Sse,
    SocketIo,
    Mcp,
}

impl RequestKind {
    pub const ALL: [RequestKind; 8] = [
        RequestKind::Rest,
        RequestKind::GraphQl,
        RequestKind::Grpc,
        RequestKind::Soap,
        RequestKind::WebSocket,
        RequestKind::Sse,
        RequestKind::SocketIo,
        RequestKind::Mcp,
    ];

    pub fn label(self) -> &'static str {
        t(match self {
            RequestKind::Rest => "model.kind.rest",
            RequestKind::GraphQl => "model.kind.graphql",
            RequestKind::Grpc => "model.kind.grpc",
            RequestKind::Soap => "model.kind.soap",
            RequestKind::WebSocket => "model.kind.websocket",
            RequestKind::Sse => "model.kind.sse",
            RequestKind::SocketIo => "model.kind.socketio",
            RequestKind::Mcp => "model.kind.mcp",
        })
    }

    /// The protocol's `EntityKind`, as `entity.create` / `entity.load` / `entity.update` / `entity.delete` take it.
    pub fn entity_kind(self) -> &'static str {
        match self {
            RequestKind::Rest => "requests",
            RequestKind::GraphQl => "graphqlRequests",
            RequestKind::Grpc => "grpcRequests",
            RequestKind::Soap => "soapRequests",
            RequestKind::WebSocket => "wsConnections",
            RequestKind::Sse => "sseRequests",
            RequestKind::SocketIo => "socketioRequests",
            RequestKind::Mcp => "mcpRequests",
        }
    }

    /// The request type an `EntityKind` names, for an answer of the engine that says which type it built.
    pub fn from_entity_kind(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|candidate| candidate.entity_kind() == kind)
    }

    /// The array of this kind's request stubs in `config.get`.
    pub(crate) fn config_key(self) -> &'static str {
        match self {
            RequestKind::Rest => "requests",
            RequestKind::GraphQl => "graphqlRequests",
            RequestKind::Grpc => "grpcRequests",
            RequestKind::Soap => "soapRequests",
            RequestKind::WebSocket => "wsConnections",
            RequestKind::Sse => "sseRequests",
            RequestKind::SocketIo => "socketioRequests",
            RequestKind::Mcp => "mcpRequests",
        }
    }

    /// The by-id send command for the kinds that have one. WebSocket, SSE, Socket.IO and MCP connect instead of sending.
    pub fn send_command(self) -> Option<&'static str> {
        match self {
            RequestKind::Rest => Some("request.send"),
            RequestKind::GraphQl => Some("graphql.send"),
            RequestKind::Grpc => Some("grpc.send"),
            RequestKind::Soap => Some("soap.send"),
            RequestKind::WebSocket | RequestKind::Sse | RequestKind::SocketIo | RequestKind::Mcp => None,
        }
    }

    /// A few characters that mark the type in a list, where a REST row shows its method instead.
    pub fn badge(self) -> &'static str {
        t(match self {
            RequestKind::Rest => "model.kind.rest",
            RequestKind::GraphQl => "model.kind.badge_graphql",
            RequestKind::Grpc => "model.kind.grpc",
            RequestKind::Soap => "model.kind.soap",
            RequestKind::WebSocket => "model.kind.badge_websocket",
            RequestKind::Sse => "model.kind.sse",
            RequestKind::SocketIo => "model.kind.badge_socketio",
            RequestKind::Mcp => "model.kind.mcp",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_is_one_of_the_request_kinds_with_the_same_shape_of_names() {
        assert!(RequestKind::ALL.contains(&RequestKind::WebSocket));
        assert_eq!(RequestKind::WebSocket.entity_kind(), "wsConnections");
        assert_eq!(RequestKind::WebSocket.send_command(), None);
        assert_eq!(RequestKind::Rest.send_command(), Some("request.send"));
    }

    #[test]
    fn sse_is_one_of_the_request_kinds_with_the_same_shape_of_names() {
        assert!(RequestKind::ALL.contains(&RequestKind::Sse));
        assert_eq!(RequestKind::Sse.entity_kind(), "sseRequests");
        assert_eq!(RequestKind::Sse.config_key(), "sseRequests");
        assert_eq!(RequestKind::Sse.send_command(), None);
        assert_eq!(RequestKind::Sse.badge(), "SSE");
        assert_eq!(RequestKind::from_entity_kind("sseRequests"), Some(RequestKind::Sse));
    }

    #[test]
    fn socketio_is_one_of_the_request_kinds_with_the_same_shape_of_names() {
        assert!(RequestKind::ALL.contains(&RequestKind::SocketIo));
        assert_eq!(RequestKind::SocketIo.label(), "Socket.IO");
        assert_eq!(RequestKind::SocketIo.entity_kind(), "socketioRequests");
        assert_eq!(RequestKind::SocketIo.config_key(), "socketioRequests");
        assert_eq!(RequestKind::SocketIo.send_command(), None);
        assert_eq!(RequestKind::SocketIo.badge(), "SIO");
        assert_eq!(RequestKind::from_entity_kind("socketioRequests"), Some(RequestKind::SocketIo));
    }

    #[test]
    fn mcp_is_one_of_the_request_kinds_with_the_same_shape_of_names() {
        assert!(RequestKind::ALL.contains(&RequestKind::Mcp));
        assert_eq!(RequestKind::Mcp.label(), "MCP");
        assert_eq!(RequestKind::Mcp.entity_kind(), "mcpRequests");
        assert_eq!(RequestKind::Mcp.config_key(), "mcpRequests");
        assert_eq!(RequestKind::Mcp.send_command(), None);
        assert_eq!(RequestKind::Mcp.badge(), "MCP");
        assert_eq!(RequestKind::from_entity_kind("mcpRequests"), Some(RequestKind::Mcp));
    }

    #[test]
    fn every_kind_has_distinct_names_on_the_wire() {
        use std::collections::HashSet;
        let entity: HashSet<_> = RequestKind::ALL.iter().map(|k| k.entity_kind()).collect();
        assert_eq!(entity.len(), RequestKind::ALL.len());
    }
}
