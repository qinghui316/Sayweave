//! In-target TSF composition component. It owns no microphone, network, vault or database.
#[cfg(windows)]
pub mod client;
#[cfg(any(test, all(windows, feature = "component")))]
mod composition_text;
#[cfg(windows)]
pub mod deployment;
#[cfg(windows)]
mod host_identity;
#[cfg(windows)]
pub mod registration;
#[cfg(all(windows, feature = "component"))]
mod service;
#[cfg(windows)]
pub mod transport;
pub mod wire;
