use crate::{CoreClient, Error, Probe, Result, models::Delay};
use futures_util::{
    StreamExt,
    stream::{self, BoxStream},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct ProbeResult {
    /// Position in the input list, including when names occur more than once.
    pub index: usize,
    pub probe: Probe,
    pub result: Result<Delay>,
}

#[derive(Clone, Debug)]
pub enum BatchEvent {
    Completed {
        completed: usize,
        total: usize,
        result: ProbeResult,
    },
    Finished {
        completed: usize,
        total: usize,
        cancelled: bool,
    },
}

#[derive(Clone, Debug)]
pub struct BatchOutcome {
    /// Completed items in completion order, retained even when the batch is cancelled.
    pub results: Vec<ProbeResult>,
    pub total: usize,
    pub cancelled: bool,
}

/// Pull-driven, bounded concurrency. Dropping the batch drops its in-flight requests.
pub struct BatchTest {
    pending: BoxStream<'static, ProbeResult>,
    cancel: CancellationToken,
    completed: usize,
    total: usize,
    finished: bool,
}

impl BatchTest {
    /// Returns each completion, then exactly one Finished event, then None.
    pub async fn recv(&mut self) -> Option<BatchEvent> {
        if self.finished {
            return None;
        }
        if self.completed == self.total {
            return Some(self.finish(false));
        }
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Some(self.finish(true)),
            next = self.pending.next() => match next {
                Some(result) => {
                    self.completed += 1;
                    Some(BatchEvent::Completed { completed: self.completed, total: self.total, result })
                },
                None => Some(self.finish(false)),
            },
        }
    }

    fn finish(&mut self, cancelled: bool) -> BatchEvent {
        self.finished = true;
        self.pending = stream::empty().boxed();
        BatchEvent::Finished {
            completed: self.completed,
            total: self.total,
            cancelled,
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub(crate) async fn collect(mut self) -> BatchOutcome {
        let mut outcome = BatchOutcome {
            results: Vec::new(),
            total: self.total,
            cancelled: false,
        };
        while let Some(event) = self.recv().await {
            match event {
                BatchEvent::Completed { result, .. } => outcome.results.push(result),
                BatchEvent::Finished { cancelled, .. } => outcome.cancelled = cancelled,
            }
        }
        outcome
    }
}

impl CoreClient {
    pub fn batch_tests(
        &self,
        probes: Vec<Probe>,
        concurrency: usize,
        parent: &CancellationToken,
    ) -> Result<BatchTest> {
        if !(1..=32).contains(&concurrency) {
            return Err(Error::invalid("probe concurrency must be between 1 and 32"));
        }
        let total = probes.len();
        let client = self.clone();
        let pending = stream::iter(probes.into_iter().enumerate())
            .map(move |(index, probe)| {
                let client = client.clone();
                async move {
                    let result = client.test_proxy(&probe).await;
                    ProbeResult {
                        index,
                        probe,
                        result,
                    }
                }
            })
            .buffer_unordered(concurrency)
            .boxed();
        Ok(BatchTest {
            pending,
            cancel: parent.child_token(),
            completed: 0,
            total,
            finished: false,
        })
    }

    pub async fn test_batch(
        &self,
        probes: Vec<Probe>,
        concurrency: usize,
        cancel: &CancellationToken,
    ) -> Result<BatchOutcome> {
        Ok(self
            .batch_tests(probes, concurrency, cancel)?
            .collect()
            .await)
    }
}
