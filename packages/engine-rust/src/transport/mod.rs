//! How clients reach the engine: one session model (`session`) over three byte carriers.

pub mod event_log;
pub mod event_pump;
pub mod framing;
pub mod scopes;
pub mod session;
pub mod socket;
pub mod stdio;
pub mod ws;
