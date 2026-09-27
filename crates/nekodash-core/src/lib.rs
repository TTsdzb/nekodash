//! Core API transport, typed snapshots and endpoint-scoped subscriptions.
//! All network calls run on a caller-owned Tokio runtime.

mod batch;
mod client;
mod endpoint;
mod error;
pub mod models;
mod recovery;
mod session;
mod store;
mod stream;
mod tls;

pub use batch::{BatchEvent, BatchOutcome, BatchTest, ProbeResult};
pub use client::{ClientOptions, CoreClient, MaintenanceAction, Probe};
pub use endpoint::Endpoint;
pub use error::{Error, ErrorKind, Result};
pub use recovery::{
    CommandOutcome, CoreSnapshot, RecoveryOperation, RecoveryOptions, RecoveryReport,
};
pub use session::{
    RequestContext, Session, SessionBatch, SessionEvent, SessionSubscription, SessionToken,
};
pub use store::EndpointStore;
pub use stream::{StreamData, StreamEvent, StreamKind, StreamOptions, StreamState, Subscription};
pub use tokio_util::sync::CancellationToken;
