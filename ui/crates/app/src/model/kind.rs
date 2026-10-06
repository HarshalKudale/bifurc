/// The request types the engine can store and send. WebSocket is one of them, not a separate feature:
/// it has a saved entity, a folder tree and an id-addressed command family exactly like the others.
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
        match self {
            RequestKind::Rest => "REST",
            RequestKind::GraphQl => "GraphQL",
            RequestKind::Grpc => "gRPC",
            RequestKind::Soap => "SOAP",
            RequestKind::WebSocket => "WebSocket",
            RequestKind::Sse => "SSE",
            RequestKind::SocketIo => "Socket.IO",
            RequestKind::Mcp => "MCP",
        }
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

    /// The `kind` of `folder.add` / `folder.rename` / `folder.delete`.
    pub fn folder_kind(self) -> &'static str {
        match self {
            RequestKind::Rest => "request",
            RequestKind::GraphQl => "graphqlRequest",
            RequestKind::Grpc => "grpcRequest",
            RequestKind::Soap => "soapRequest",
            RequestKind::WebSocket => "ws",
            RequestKind::Sse => "sseRequest",
            RequestKind::SocketIo => "socketioRequest",
            RequestKind::Mcp => "mcpRequest",
        }
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

    /// The array of this kind's folders in `config.get`.
    pub(crate) fn folders_key(self) -> &'static str {
        match self {
            RequestKind::Rest => "requestFolders",
            RequestKind::GraphQl => "graphqlRequestFolders",
            RequestKind::Grpc => "grpcRequestFolders",
            RequestKind::Soap => "soapRequestFolders",
            RequestKind::WebSocket => "wsFolders",
            RequestKind::Sse => "sseRequestFolders",
            RequestKind::SocketIo => "socketioRequestFolders",
            RequestKind::Mcp => "mcpRequestFolders",
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
        match self {
            RequestKind::Rest => "REST",
            RequestKind::GraphQl => "GQL",
            RequestKind::Grpc => "gRPC",
            RequestKind::Soap => "SOAP",
            RequestKind::WebSocket => "WS",
            RequestKind::Sse => "SSE",
            RequestKind::SocketIo => "SIO",
            RequestKind::Mcp => "MCP",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_is_one_of_the_request_kinds_with_the_same_shape_of_names() {
        assert!(RequestKind::ALL.contains(&RequestKind::WebSocket));
        assert_eq!(RequestKind::WebSocket.entity_kind(), "wsConnections");
        assert_eq!(RequestKind::WebSocket.folder_kind(), "ws");
        assert_eq!(RequestKind::WebSocket.send_command(), None);
        assert_eq!(RequestKind::Rest.send_command(), Some("request.send"));
    }

    #[test]
    fn sse_is_one_of_the_request_kinds_with_the_same_shape_of_names() {
        assert!(RequestKind::ALL.contains(&RequestKind::Sse));
        assert_eq!(RequestKind::Sse.entity_kind(), "sseRequests");
        assert_eq!(RequestKind::Sse.folder_kind(), "sseRequest");
        assert_eq!(RequestKind::Sse.config_key(), "sseRequests");
        assert_eq!(RequestKind::Sse.folders_key(), "sseRequestFolders");
        assert_eq!(RequestKind::Sse.send_command(), None);
        assert_eq!(RequestKind::Sse.badge(), "SSE");
        assert_eq!(RequestKind::from_entity_kind("sseRequests"), Some(RequestKind::Sse));
    }

    #[test]
    fn socketio_is_one_of_the_request_kinds_with_the_same_shape_of_names() {
        assert!(RequestKind::ALL.contains(&RequestKind::SocketIo));
        assert_eq!(RequestKind::SocketIo.label(), "Socket.IO");
        assert_eq!(RequestKind::SocketIo.entity_kind(), "socketioRequests");
        assert_eq!(RequestKind::SocketIo.folder_kind(), "socketioRequest");
        assert_eq!(RequestKind::SocketIo.config_key(), "socketioRequests");
        assert_eq!(RequestKind::SocketIo.folders_key(), "socketioRequestFolders");
        assert_eq!(RequestKind::SocketIo.send_command(), None);
        assert_eq!(RequestKind::SocketIo.badge(), "SIO");
        assert_eq!(RequestKind::from_entity_kind("socketioRequests"), Some(RequestKind::SocketIo));
    }

    #[test]
    fn mcp_is_one_of_the_request_kinds_with_the_same_shape_of_names() {
        assert!(RequestKind::ALL.contains(&RequestKind::Mcp));
        assert_eq!(RequestKind::Mcp.label(), "MCP");
        assert_eq!(RequestKind::Mcp.entity_kind(), "mcpRequests");
        assert_eq!(RequestKind::Mcp.folder_kind(), "mcpRequest");
        assert_eq!(RequestKind::Mcp.config_key(), "mcpRequests");
        assert_eq!(RequestKind::Mcp.folders_key(), "mcpRequestFolders");
        assert_eq!(RequestKind::Mcp.send_command(), None);
        assert_eq!(RequestKind::Mcp.badge(), "MCP");
        assert_eq!(RequestKind::from_entity_kind("mcpRequests"), Some(RequestKind::Mcp));
    }

    #[test]
    fn every_kind_has_distinct_names_on_the_wire() {
        use std::collections::HashSet;
        let entity: HashSet<_> = RequestKind::ALL.iter().map(|k| k.entity_kind()).collect();
        let folder: HashSet<_> = RequestKind::ALL.iter().map(|k| k.folder_kind()).collect();
        assert_eq!(entity.len(), RequestKind::ALL.len());
        assert_eq!(folder.len(), RequestKind::ALL.len());
    }
}
