/// The request types the engine can store and send. WebSocket is one of them, not a separate feature:
/// it has a saved entity, a folder tree and an id-addressed command family exactly like the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestKind {
    Rest,
    GraphQl,
    Grpc,
    Soap,
    WebSocket,
}

impl RequestKind {
    pub const ALL: [RequestKind; 5] =
        [RequestKind::Rest, RequestKind::GraphQl, RequestKind::Grpc, RequestKind::Soap, RequestKind::WebSocket];

    pub fn label(self) -> &'static str {
        match self {
            RequestKind::Rest => "REST",
            RequestKind::GraphQl => "GraphQL",
            RequestKind::Grpc => "gRPC",
            RequestKind::Soap => "SOAP",
            RequestKind::WebSocket => "WebSocket",
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
        }
    }

    /// The `kind` of `folder.add` / `folder.rename` / `folder.delete`.
    pub fn folder_kind(self) -> &'static str {
        match self {
            RequestKind::Rest => "request",
            RequestKind::GraphQl => "graphqlRequest",
            RequestKind::Grpc => "grpcRequest",
            RequestKind::Soap => "soapRequest",
            RequestKind::WebSocket => "ws",
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
        }
    }

    /// The by-id send command for the kinds that have one. WebSocket connects instead of sending.
    pub fn send_command(self) -> Option<&'static str> {
        match self {
            RequestKind::Rest => Some("request.send"),
            RequestKind::GraphQl => Some("graphql.send"),
            RequestKind::Grpc => Some("grpc.send"),
            RequestKind::Soap => Some("soap.send"),
            RequestKind::WebSocket => None,
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
    fn every_kind_has_distinct_names_on_the_wire() {
        use std::collections::HashSet;
        let entity: HashSet<_> = RequestKind::ALL.iter().map(|k| k.entity_kind()).collect();
        let folder: HashSet<_> = RequestKind::ALL.iter().map(|k| k.folder_kind()).collect();
        assert_eq!(entity.len(), RequestKind::ALL.len());
        assert_eq!(folder.len(), RequestKind::ALL.len());
    }
}
