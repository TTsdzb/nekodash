//! Core API transport, typed snapshots and endpoint-scoped subscriptions.
//! All network calls run on a caller-owned Tokio runtime.

mod client;
mod endpoint;
mod error;
pub mod models;
mod session;
mod store;
mod stream;
mod tls;

pub use client::{ClientOptions, CoreClient, MaintenanceAction, Probe, ProbeResult};
pub use endpoint::Endpoint;
pub use error::{Error, ErrorKind, Result};
pub use session::{RequestContext, Session, SessionToken};
pub use store::EndpointStore;
pub use stream::{StreamData, StreamEvent, StreamKind, StreamOptions, StreamState, Subscription};
pub use tokio_util::sync::CancellationToken;
