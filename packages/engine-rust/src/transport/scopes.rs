//! The scope each command requires. A command with no scope here is denied, so the table must be total;
//! `tests::scopes_are_total` holds it to the protocol.

use std::collections::HashSet;

use crate::kernel::Scope;

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
    ("account.list", Scope::Read),
    ("account.repos", Scope::Read),
    ("account.login.start", Scope::Admin),
    ("account.login.status", Scope::Read),
    ("account.login.cancel", Scope::Admin),
    ("account.logout", Scope::Admin),
    ("env.setActive", Scope::Write),
    ("env.resolveText", Scope::Read),
    ("vault.list", Scope::Read),
    ("vault.create", Scope::Write),
    ("vault.rename", Scope::Write),
    ("vault.delete", Scope::Write),
    ("vault.unseal", Scope::Write),
    ("vault.seal", Scope::Write),
    ("vault.changePassphrase", Scope::Write),
    ("vault.setSecret", Scope::Write),
    ("vault.deleteSecret", Scope::Write),
    ("vault.checkout", Scope::Execute),
    ("vault.release", Scope::Execute),
    ("server.status", Scope::Read),
    ("server.start", Scope::Admin),
    ("server.stop", Scope::Admin),
    ("server.restart", Scope::Admin),
    ("proxy.status", Scope::Read),
    ("services.discover", Scope::Read),
    ("graphql.introspect", Scope::Read),
    ("graphql.execute", Scope::Execute),
    ("soap.fetchWsdl", Scope::Read),
    ("soap.execute", Scope::Execute),
    ("grpc.execute", Scope::Execute),
    ("grpc.reflect", Scope::Read),
    ("application.list", Scope::Read),
    ("application.save", Scope::Write),
    ("application.delete", Scope::Admin),
    ("application.start", Scope::Admin),
    ("application.stop", Scope::Admin),
    ("application.getState", Scope::Read),
    ("application.getAllStates", Scope::Read),
    ("application.getLogs", Scope::Read),
    ("application.checkPort", Scope::Read),
    ("application.killPort", Scope::Admin),
    ("runner.saveReport", Scope::Write),
    ("runner.exportReport", Scope::Write),
    ("runner.getHistory", Scope::Read),
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
    ("request.parseCurl", Scope::Read),
    ("curl.buildRequest", Scope::Read),
    ("curl.buildMock", Scope::Read),
    ("healthbar.getServices", Scope::Read),
    ("healthbar.saveServices", Scope::Write),
    ("healthbar.checkUrl", Scope::Read),
    ("capture.shareJson", Scope::Write),
    ("capture.buildRequest", Scope::Read),
    ("capture.buildMock", Scope::Read),
    ("engine.shutdown", Scope::Admin),
    ("capture.list", Scope::Read),
    ("capture.get", Scope::Read),
    ("capture.setPaused", Scope::Write),
    ("capture.setDisableCache", Scope::Write),
    ("capture.clear", Scope::Write),
    ("capture.remove", Scope::Write),
    ("request.send", Scope::Execute),
    ("graphql.send", Scope::Execute),
    ("soap.send", Scope::Execute),
    ("grpc.send", Scope::Execute),
    ("runner.start", Scope::Execute),
    ("runner.cancel", Scope::Execute),
    ("runner.status", Scope::Read),
    ("grpc.stream.open", Scope::Execute),
    ("grpc.stream.send", Scope::Execute),
    ("grpc.stream.end", Scope::Execute),
    ("grpc.stream.cancel", Scope::Execute),
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

/// The browser extension: it reads and saves, and runs nothing.
pub fn companion_scopes() -> HashSet<Scope> {
    HashSet::from([Scope::Read, Scope::Write])
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
