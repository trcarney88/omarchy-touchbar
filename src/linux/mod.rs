//! Linux physical-display coordination.
//!
//! The application entry point is deliberately responsible for opting into this
//! module; importing it does not touch the desktop session or hardware.

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub mod data;
pub mod display;
pub mod input;
pub mod keyboard;
pub mod lease;
pub mod quiescence;
pub mod runtime;
pub mod session;
pub mod session_worker;
pub mod shutdown;
pub mod volume;
