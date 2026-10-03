//! What the CLI knows of the engine protocol: the handshake constants and the command names `call` accepts
//! (`packages/protocol/src/commands/index.ts`).

/// The protocol version this client speaks. The engine accepts any 1.x.
pub const PROTOCOL_VERSION: &str = "1.0.0";
/// This client's name in the `hello` handshake.
pub const CLIENT_NAME: &str = "cli";

/// Every engine command, sorted.
pub const COMMAND_NAMES: [&str; 115] = [
    "application.checkPort",
    "application.delete",
    "application.getAllStates",
    "application.getLogs",
    "application.getState",
    "application.killPort",
    "application.list",
    "application.save",
    "application.start",
    "application.stop",
    "audit.diff",
    "audit.export",
    "audit.list",
    "blob.put",
    "blob.read",
    "blob.release",
    "blob.stat",
    "capture.buildMock",
    "capture.buildRequest",
    "capture.clear",
    "capture.get",
    "capture.list",
    "capture.remove",
    "capture.setDisableCache",
    "capture.setPaused",
    "capture.shareJson",
    "config.get",
    "config.save",
    "engine.shutdown",
    "entity.create",
    "entity.delete",
    "entity.duplicate",
    "entity.list",
    "entity.load",
    "entity.publish",
    "entity.restore",
    "entity.setEnabled",
    "entity.update",
    "env.getMerged",
    "env.resolveText",
    "env.setActive",
    "export.create",
    "export.formats",
    "folder.add",
    "folder.delete",
    "folder.move",
    "folder.publish",
    "folder.rename",
    "git.diff",
    "git.discard",
    "git.history",
    "git.sync",
    "graphql.execute",
    "graphql.introspect",
    "graphql.send",
    "grpc.execute",
    "grpc.reflect",
    "grpc.send",
    "grpc.stream.cancel",
    "grpc.stream.end",
    "grpc.stream.open",
    "grpc.stream.send",
    "healthbar.checkUrl",
    "healthbar.getServices",
    "healthbar.saveServices",
    "history.diff",
    "history.list",
    "import.commit",
    "import.preflight",
    "proxy.status",
    "request.parseCurl",
    "request.replay",
    "request.send",
    "runner.cancel",
    "runner.exportReport",
    "runner.getHistory",
    "runner.listFolderIds",
    "runner.loadConfig",
    "runner.saveConfig",
    "runner.saveReport",
    "runner.start",
    "runner.status",
    "script.execute",
    "server.restart",
    "server.start",
    "server.status",
    "server.stop",
    "services.discover",
    "settings.update",
    "soap.execute",
    "soap.fetchWsdl",
    "soap.send",
    "sync.disconnect",
    "sync.getEntityStatus",
    "sync.getState",
    "sync.pull",
    "sync.push",
    "sync.setAutoSync",
    "sync.setRemote",
    "tls.certStatus",
    "tls.exportCert",
    "tls.generate",
    "tls.importCert",
    "tls.importKey",
    "tls.removeCert",
    "workspace.add",
    "workspace.delete",
    "workspace.rename",
    "workspace.setActive",
    "ws.clearMessages",
    "ws.connect",
    "ws.disconnect",
    "ws.list",
    "ws.send",
    "ws.status",
];

/// Whether `name` is an engine command.
pub fn is_known_command(name: &str) -> bool {
    COMMAND_NAMES.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_are_sorted_and_unique() {
        assert!(COMMAND_NAMES.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn known_commands_are_recognised() {
        assert!(is_known_command("config.get"));
        assert!(is_known_command("ws.status"));
        assert!(!is_known_command("config.nope"));
        assert!(!is_known_command(""));
    }
}
