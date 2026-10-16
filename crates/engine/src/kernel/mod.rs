//! What every module shares: the command registry, the event bus and the record of proxied traffic.

pub mod bus;
pub mod kinds;
pub mod registry;
pub mod streams;
pub mod traffic;

pub use bus::{EngineEventBus, ListenerId, bus};
pub use registry::{CommandContext, CommandRegistry, Scope, SessionIdentity};
