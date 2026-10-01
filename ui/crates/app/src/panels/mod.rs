//! The dockable panels: each is a view the dock area can place, tab, split, move and close.
//!
//! A panel holds no workspace state of its own. It renders from the model (or, for the request tabs, from
//! the session they wrap) and reports what the user did as events for the shell to act on.

pub mod activity;
pub mod details;
pub mod explorer;
pub mod request_tab;
pub mod response;
pub mod welcome;
