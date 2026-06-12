//! voindns-core: the platform-independent engine shared by the service and (for
//! the IPC client half) the GUI.
//!
//! - [`proxy`] — local DNS → DoH proxy (hickory).
//! - [`redirect`] — system DNS redirection per OS.
//! - [`controller`] — Connect/Disconnect state machine.
//! - [`ipc`] — GUI ↔ service local-socket protocol.

pub mod controller;
pub mod ipc;
pub mod proxy;
pub mod redirect;

pub use controller::Controller;
pub use proxy::DohProxy;
