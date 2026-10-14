//! In-target TSF composition component. It owns no microphone, network, vault or database.
#[cfg(windows)]
pub mod atomic_record;
#[cfg(windows)]
pub mod client;
#[cfg(all(not(feature = "proxy"), any(test, all(windows, feature = "component"))))]
mod composition_text;
#[cfg(windows)]
pub mod deployment;
pub mod engine_abi;
#[cfg(windows)]
mod host_identity;
pub mod lifecycle;
#[cfg(all(not(feature = "proxy"), any(test, all(windows, feature = "component"))))]
mod prefix;
#[cfg(all(windows, feature = "proxy"))]
mod proxy;
#[cfg(windows)]
pub mod registration;
pub mod routing;
#[cfg(all(windows, feature = "component", not(feature = "proxy")))]
mod service;
pub mod settlement_boundary;
#[cfg(windows)]
pub mod transport;
pub mod wire;
