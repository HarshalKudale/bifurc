//! The scope each command requires. A command with no scope here is denied, so the table must be total;
//! `tests::scopes_are_total` holds it to the protocol.

use std::collections::HashSet;

use crate::{
    kernel::Scope,
    protocol::{EngineError, ErrorCode},
};

const SCOPE_BY_COMMAND: &[(&str, Scope)] = &[
    ("entity.create", Scope::Write),
    ("entity.duplicate", Scope::Write),
    ("entity.update", Scope::Write),
    ("entity.delete", Scope::Write),
    ("entity.confirmDelete", Scope::Write),
    ("entity.revertDelete", Scope::Write),
    ("folder.confirmDelete", Scope::Write),
    ("folder.revertDelete", Scope::Write),
    ("entity.load", Scope::Read),
    ("entity.list", Scope::Read),
    ("entity.search", Scope::Read),
    ("entity.setEnabled", Scope::Write),
    ("folder.add", Scope::Write),
    ("folder.rename", Scope::Write),
    ("folder.move", Scope::Write),
    ("folder.delete", Scope::Write),
    ("folder.publish", Scope::Write),
    ("config.get", Scope::Read),
    ("config.save", Scope::Write),
    ("settings.update", Scope::Write),
    ("workspace.add", Scope::Write),
    ("workspace.rename", Scope::Write),
    ("workspace.delete", Scope::Write),
    ("workspace.setActive", Scope::Write),
    ("workspace.import", Scope::Write),
    ("setup.getState", Scope::Read),
    ("setup.acceptTerms", Scope::Write),
    ("setup.acceptPrivacy", Scope::Write),
    ("setup.complete", Scope::Write),
    ("setup.startServers", Scope::Admin),
    ("setup.allowLowPorts", Scope::Admin),
    ("account.list", Scope::Read),
    ("account.repos", Scope::Read),
    ("account.login.start", Scope::Admin),
    ("account.login.status", Scope::Read),
    ("account.login.cancel", Scope::Admin),
    ("account.logout", Scope::Admin),
    ("env.setActive", Scope::Write),
    ("env.resolveText", Scope::Read),
    ("cookies.list", Scope::Read),
    ("cookies.clear", Scope::Write),
    ("cookies.configure", Scope::Write),
    ("cookies.set", Scope::Write),
    ("changes.list", Scope::Read),
    ("claims.set", Scope::Write),
    ("claims.list", Scope::Read),
    ("session.get", Scope::Read),
    ("session.setVariables", Scope::Write),
    ("session.clearVariables", Scope::Write),
    ("session.reset", Scope::Write),
    ("vault.list", Scope::Read),
    ("vault.create", Scope::Write),
    ("vault.rename", Scope::Write),
    ("vault.delete", Scope::Write),
    ("vault.unseal", Scope::Write),
    ("vault.seal", Scope::Write),
    ("vault.changePassphrase", Scope::Write),
    ("vault.setSecret", Scope::Write),
    ("vault.deleteSecret", Scope::Write),
    ("vault.secrets", Scope::Execute),
    ("server.status", Scope::Read),
    ("server.start", Scope::Admin),
    ("server.stop", Scope::Admin),
    ("server.restart", Scope::Admin),
    ("proxy.status", Scope::Read),
    ("mocks.serve", Scope::Admin),
    ("mocks.serveStop", Scope::Admin),
    ("mocks.serveStatus", Scope::Read),
    ("services.discover", Scope::Read),
    ("devservice.define", Scope::Write),
    ("devservice.remove", Scope::Write),
    ("devservice.list", Scope::Read),
    ("devservice.logs", Scope::Read),
    ("devservice.up", Scope::Execute),
    ("devservice.down", Scope::Execute),
    ("devservice.freePort", Scope::Execute),
    ("graphql.introspect", Scope::Read),
    ("graphql.execute", Scope::Execute),
    ("soap.fetchWsdl", Scope::Read),
    ("soap.execute", Scope::Execute),
    ("grpc.execute", Scope::Execute),
    ("grpc.reflect", Scope::Read),
    ("runner.exportReport", Scope::Write),
    ("runner.saveConfig", Scope::Write),
    ("runner.loadConfig", Scope::Write),
    ("runner.listFolderIds", Scope::Read),
    ("sync.publish", Scope::Write),
    ("sync.push", Scope::Write),
    ("sync.all", Scope::Write),
    ("sync.pull", Scope::Write),
    ("sync.getState", Scope::Read),
    ("sync.getEntityStatus", Scope::Read),
    ("git.diff", Scope::Read),
    ("git.discard", Scope::Write),
    ("git.sync", Scope::Write),
    ("git.history", Scope::Read),
    ("entity.publish", Scope::Write),
    ("entity.restore", Scope::Write),
    ("audit.list", Scope::Read),
    ("audit.diff", Scope::Read),
    ("audit.export", Scope::Write),
    ("history.list", Scope::Read),
    ("history.diff", Scope::Read),
    ("history.patch", Scope::Read),
    ("tls.generate", Scope::Admin),
    ("tls.certStatus", Scope::Read),
    ("tls.removeCert", Scope::Admin),
    ("tls.exportCert", Scope::Write),
    ("tls.importCert", Scope::Write),
    ("tls.importKey", Scope::Write),
    ("export.formats", Scope::Read),
    ("export.create", Scope::Write),
    ("import.preflight", Scope::Write),
    ("import.commit", Scope::Write),
    ("blob.put", Scope::Write),
    ("blob.stat", Scope::Read),
    ("blob.read", Scope::Read),
    ("blob.release", Scope::Write),
    ("script.execute", Scope::Execute),
    ("request.replay", Scope::Execute),
    ("request.cancel", Scope::Execute),
    ("request.parseCurl", Scope::Read),
    ("curl.buildRequest", Scope::Read),
    ("curl.buildMock", Scope::Read),
    ("healthbar.getServices", Scope::Read),
    ("healthbar.saveServices", Scope::Write),
    ("healthbar.checkUrl", Scope::Read),
    ("capture.shareJson", Scope::Write),
    ("capture.buildRequest", Scope::Read),
    ("capture.buildMock", Scope::Read),
    ("capture.inspectImport", Scope::Read),
    ("capture.import", Scope::Write),
    ("engine.shutdown", Scope::Admin),
    ("engine.companions", Scope::Read),
    ("update.status", Scope::Read),
    ("update.check", Scope::Execute),
    ("update.skip", Scope::Write),
    ("capture.list", Scope::Read),
    ("capture.get", Scope::Read),
    ("capture.setPaused", Scope::Write),
    ("capture.setDisableCache", Scope::Write),
    ("capture.clear", Scope::Write),
    ("capture.remove", Scope::Write),
    ("capture.pin", Scope::Write),
    ("contract.saveSnapshots", Scope::Write),
    ("contract.listSnapshots", Scope::Read),
    ("contract.fromOpenApi", Scope::Read),
    ("request.send", Scope::Execute),
    ("response.history", Scope::Read),
    ("response.clearHistory", Scope::Write),
    ("graphql.send", Scope::Execute),
    ("soap.send", Scope::Execute),
    ("grpc.send", Scope::Execute),
    ("flow.validate", Scope::Read),
    ("flow.status", Scope::Read),
    ("flow.fromSteps", Scope::Write),
    ("flow.run", Scope::Execute),
    ("flow.cancel", Scope::Execute),
    ("flow.loadtest.start", Scope::Execute),
    ("flow.loadtest.cancel", Scope::Execute),
    ("flow.loadtest.status", Scope::Read),
    ("runner.start", Scope::Execute),
    ("runner.cancel", Scope::Execute),
    ("runner.status", Scope::Read),
    ("grpc.stream.open", Scope::Execute),
    ("grpc.stream.send", Scope::Execute),
    ("grpc.stream.end", Scope::Execute),
    ("grpc.stream.cancel", Scope::Execute),
    ("sse.connect", Scope::Execute),
    ("sse.disconnect", Scope::Execute),
    ("sse.status", Scope::Read),
    ("sse.list", Scope::Read),
    ("sse.clearMessages", Scope::Write),
    ("socketio.connect", Scope::Execute),
    ("socketio.disconnect", Scope::Execute),
    ("socketio.emit", Scope::Execute),
    ("socketio.listen", Scope::Execute),
    ("socketio.status", Scope::Read),
    ("socketio.list", Scope::Read),
    ("socketio.clearMessages", Scope::Write),
    ("mcp.connect", Scope::Execute),
    ("mcp.disconnect", Scope::Execute),
    ("mcp.call", Scope::Execute),
    ("mcp.status", Scope::Read),
    ("mcp.list", Scope::Read),
    ("mcp.clearMessages", Scope::Write),
    ("ws.connect", Scope::Execute),
    ("ws.disconnect", Scope::Execute),
    ("ws.send", Scope::Execute),
    ("ws.status", Scope::Read),
    ("ws.list", Scope::Read),
    ("ws.clearMessages", Scope::Write),
    ("env.getMerged", Scope::Read),
];

pub fn required_scope_for(command: &str) -> Option<Scope> {
    SCOPE_BY_COMMAND.iter().find(|(name, _)| *name == command).map(|(_, scope)| *scope)
}

/// Whether a session holding `granted` may run `command`. An unscoped command is denied.
pub fn command_is_allowed(granted: &HashSet<Scope>, command: &str) -> bool {
    required_scope_for(command).is_some_and(|required| granted.contains(&required))
}

/// Every scope: a local interface (the desktop UI, the CLI).
pub fn shell_scopes() -> HashSet<Scope> {
    HashSet::from([Scope::Read, Scope::Write, Scope::Execute, Scope::Admin])
}

/// The browser extension's scopes. The companion endpoint is unauthenticated, so scopes are not what limits it:
/// see [`companion_refusal`] and [`COMPANION_EVENTS`].
pub fn companion_scopes() -> HashSet<Scope> {
    HashSet::from([Scope::Read, Scope::Write])
}

/// The only events the companion endpoint delivers: what the browser extension listens for.
pub const COMPANION_EVENTS: &[&str] = &["event.settings.changed", "event.server.error", "event.engine.stopping"];

/// Why the companion endpoint refuses `action`, if it does: it serves the proxy status and nothing else.
pub fn companion_refusal(action: &str) -> Option<EngineError> {
    (action != "server.status").then(|| {
        EngineError::new(ErrorCode::Forbidden, "the companion endpoint only serves status".to_string())
            .keyed("transport.companion_status_only", serde_json::json!({}))
    })
}

pub fn read_only_scopes() -> HashSet<Scope> {
    HashSet::from([Scope::Read])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::schema;

    #[test]
    fn scopes_are_total() {
        let commands = schema::command_names();
        let unscoped: Vec<&&str> = commands.iter().filter(|c| required_scope_for(c).is_none()).collect();
        let surplus: Vec<&str> =
            SCOPE_BY_COMMAND.iter().map(|(name, _)| *name).filter(|name| !commands.contains(name)).collect();
        assert!(unscoped.is_empty(), "unscoped (would be denied): {unscoped:?}");
        assert!(surplus.is_empty(), "scoped but not a command: {surplus:?}");
    }
}
