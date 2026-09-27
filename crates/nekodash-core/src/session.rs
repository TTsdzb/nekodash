use crate::{
    BatchEvent, BatchTest, ClientOptions, CoreClient, Endpoint, Error, Probe, Result, StreamEvent,
    StreamKind, StreamOptions, Subscription,
};
use std::future::Future;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionToken {
    pub endpoint_id: String,
    pub generation: u64,
}

/// Every outcome carries its origin, including cancellation and receive errors.
#[derive(Clone, Debug)]
pub struct SessionEvent<T> {
    pub token: SessionToken,
    pub result: Result<T>,
}

#[derive(Clone)]
pub struct RequestContext {
    pub client: CoreClient,
    pub token: SessionToken,
    pub cancel: CancellationToken,
}

impl RequestContext {
    /// Tag asynchronous results so a GUI can discard callbacks queued before an endpoint switch.
    pub async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> SessionEvent<T> {
        let result = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(Error::cancelled()),
            value = work => value,
        };
        self.event(result)
    }

    pub(crate) fn event<T>(&self, result: Result<T>) -> SessionEvent<T> {
        SessionEvent {
            token: self.token.clone(),
            result,
        }
    }

    pub fn subscribe(
        &self,
        runtime: &Handle,
        kind: StreamKind,
        options: StreamOptions,
    ) -> SessionEvent<SessionSubscription> {
        self.event(
            self.client
                .subscribe(runtime, kind, options, &self.cancel)
                .map(|subscription| SessionSubscription {
                    token: self.token.clone(),
                    subscription,
                }),
        )
    }

    pub fn batch_tests(
        &self,
        probes: Vec<Probe>,
        concurrency: usize,
    ) -> SessionEvent<SessionBatch> {
        self.event(
            self.client
                .batch_tests(probes, concurrency, &self.cancel)
                .map(|batch| SessionBatch {
                    token: self.token.clone(),
                    batch,
                }),
        )
    }
}

pub struct SessionSubscription {
    token: SessionToken,
    subscription: Subscription,
}

impl SessionSubscription {
    pub async fn recv(&mut self) -> SessionEvent<StreamEvent> {
        SessionEvent {
            token: self.token.clone(),
            result: self.subscription.recv().await,
        }
    }
    pub fn cancel(&self) {
        self.subscription.cancel();
    }
    pub fn is_finished(&self) -> bool {
        self.subscription.is_finished()
    }
}

pub struct SessionBatch {
    token: SessionToken,
    batch: BatchTest,
}

impl SessionBatch {
    pub async fn recv(&mut self) -> Option<SessionEvent<BatchEvent>> {
        self.batch.recv().await.map(|event| SessionEvent {
            token: self.token.clone(),
            result: Ok(event),
        })
    }
    pub fn cancel(&self) {
        self.batch.cancel();
    }
    pub fn is_finished(&self) -> bool {
        self.batch.is_finished()
    }
}

#[derive(Default)]
pub struct Session {
    generation: u64,
    active: Option<RequestContext>,
}

impl Session {
    pub fn switch(&mut self, endpoint: Endpoint, options: ClientOptions) -> Result<RequestContext> {
        let client = CoreClient::with_options(endpoint, options)?;
        self.activate(client)
    }

    /// Renew the generation and HTTP pool after a network change, resume or core restart.
    pub fn reconnect(&mut self) -> Result<RequestContext> {
        let client = self
            .active
            .as_ref()
            .ok_or_else(|| Error::invalid("no active session to reconnect"))?
            .client
            .renewed()?;
        self.activate(client)
    }

    fn activate(&mut self, client: CoreClient) -> Result<RequestContext> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Error::invalid("session generation exhausted"))?;
        self.disconnect();
        self.generation = generation;
        let context = RequestContext {
            token: SessionToken {
                endpoint_id: client.endpoint().id().to_owned(),
                generation,
            },
            client,
            cancel: CancellationToken::new(),
        };
        self.active = Some(context.clone());
        Ok(context)
    }

    pub fn current(&self) -> Option<&RequestContext> {
        self.active.as_ref()
    }
    pub fn accepts(&self, token: &SessionToken) -> bool {
        self.active
            .as_ref()
            .is_some_and(|context| &context.token == token && !context.cancel.is_cancelled())
    }
    pub fn disconnect(&mut self) {
        if let Some(context) = self.active.take() {
            context.cancel.cancel();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.disconnect();
    }
}
