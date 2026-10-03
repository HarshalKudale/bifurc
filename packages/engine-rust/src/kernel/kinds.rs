//! Entity kind names: the protocol says `proxyRules` and `wsConnections` where the store's directories are
//! `rules` and `sockets`.

pub fn to_protocol_kind(engine_kind: &str) -> &str {
    match engine_kind {
        "rules" => "proxyRules",
        "sockets" => "wsConnections",
        other => other,
    }
}

pub fn to_engine_kind(protocol_kind: &str) -> &str {
    match protocol_kind {
        "proxyRules" => "rules",
        "wsConnections" => "sockets",
        other => other,
    }
}
