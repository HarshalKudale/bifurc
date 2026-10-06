//! The command registry: every command the engine serves, by its `namespace.verb` wire name.
//!
//! A handler receives the payload after it passed the protocol's schema (`protocol::schema`), so it may read
//! required fields without re-checking them.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use serde_json::Value;

use crate::protocol::{EngineError, ErrorCode, Result, schema};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Read,
    Write,
    Execute,
    Admin,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Write => "write",
            Scope::Execute => "execute",
            Scope::Admin => "admin",
        }
    }
}

/// Who is on the other end of a transport session.
#[derive(Debug, Clone, Default)]
pub struct SessionIdentity {
    pub session_id: String,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
    pub peer: Option<String>,
    /// What the client said about itself in its handshake (`clientInfo`), already sanitized.
    pub client_info: Option<Value>,
    pub scopes: Option<HashSet<Scope>>,
}

/// What a handler is given besides its payload.
#[derive(Clone)]
pub struct CommandContext {
    /// The registry the command was invoked on, so a handler can dispatch to another command.
    pub registry: Arc<CommandRegistry>,
    /// The calling session. `None` for a call the engine makes itself, which is trusted.
    pub session: Option<SessionIdentity>,
}

impl CommandContext {
    pub fn internal(registry: Arc<CommandRegistry>) -> Self {
        Self { registry, session: None }
    }

    pub fn has_scope(&self, scope: Scope) -> bool {
        match &self.session {
            None => true,
            Some(session) => session.scopes.as_ref().is_some_and(|scopes| scopes.contains(&scope)),
        }
    }

    /// Whether the caller may change entities it did not name (e.g. disable a conflicting mock).
    pub fn may_affect_unnamed_entities(&self) -> bool {
        self.has_scope(Scope::Admin)
    }
}

pub type HandlerFuture = Pin<Box<dyn Future<Output = Result<Value>> + Send>>;
pub type CommandHandler = Arc<dyn Fn(Value, CommandContext) -> HandlerFuture + Send + Sync>;

#[derive(Default)]
pub struct CommandRegistry {
    handlers: HashMap<String, CommandHandler>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an async handler. Panics on a duplicate or on a name the protocol does not declare: both are
    /// programming errors that must not reach a running engine.
    pub fn register<F, Fut>(&mut self, action: &str, handler: F)
    where
        F: Fn(Value, CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        assert!(schema::is_known_command(action), "\"{action}\" is not a command the protocol declares");
        assert!(!self.handlers.contains_key(action), "Command \"{action}\" is already registered.");
        self.handlers.insert(action.to_string(), Arc::new(move |params, ctx| Box::pin(handler(params, ctx))));
    }

    /// Register a handler that does not await.
    pub fn register_sync<F>(&mut self, action: &str, handler: F)
    where
        F: Fn(Value, &CommandContext) -> Result<Value> + Send + Sync + 'static,
    {
        self.register(action, move |params, ctx| {
            let result = handler(params, &ctx);
            async move { result }
        });
    }

    pub fn is_registered(&self, action: &str) -> bool {
        self.handlers.contains_key(action)
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.handlers.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    /// Validate `payload` and run the handler for `action`.
    pub async fn invoke(&self, action: &str, payload: Value, ctx: CommandContext) -> Result<Value> {
        let Some(handler) = self.handlers.get(action).cloned() else {
            return Err(EngineError::new(ErrorCode::UnknownCommand, format!("Unknown command \"{action}\".")));
        };
        let params = schema::validate(action, &payload).map_err(|reason| {
            EngineError::bad_request(format!("Invalid payload for command \"{action}\": {reason}"))
        })?;
        handler(params, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn invoke_validates_then_runs() {
        let mut registry = CommandRegistry::new();
        registry.register_sync("env.resolveText", |params, _| Ok(json!({ "text": params["text"] })));
        let registry = Arc::new(registry);
        let ctx = CommandContext::internal(registry.clone());

        let ok = registry.invoke("env.resolveText", json!({ "workspaceId": "w", "text": "a" }), ctx.clone()).await;
        assert_eq!(ok.unwrap(), json!({ "text": "a" }));

        let bad = registry.invoke("env.resolveText", json!({ "text": 1 }), ctx.clone()).await.unwrap_err();
        assert_eq!(bad.code, ErrorCode::BadRequest);

        let unknown = registry.invoke("config.get", json!({}), ctx).await.unwrap_err();
        assert_eq!(unknown.code, ErrorCode::UnknownCommand);
    }
}
