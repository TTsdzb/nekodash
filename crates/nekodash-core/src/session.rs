use crate::{ClientOptions, CoreClient, Endpoint, Error, Result};
use std::future::Future;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionToken {
    pub endpoint_id: String,
    pub generation: u64,
}

#[derive(Clone)]
pub struct RequestContext {
    pub client: CoreClient,
    pub token: SessionToken,
    pub cancel: CancellationToken,
}

impl RequestContext {
    /// Tag asynchronous results so a GUI can discard callbacks queued before an endpoint switch.
    pub async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> Result<(SessionToken, T)> {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(Error::cancelled()),
            value = work => Ok((self.token.clone(), value?)),
        }
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
