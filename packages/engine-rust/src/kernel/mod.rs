//! What every module shares: the command registry, the event bus and traffic telemetry.

pub mod bus;
pub mod kinds;
pub mod registry;
pub mod telemetry;

pub use bus::{EngineEventBus, ListenerId, bus};
pub use registry::{CommandContext, CommandRegistry, Scope, SessionIdentity};
