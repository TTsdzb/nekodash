use crate::{
    CoreClient, Error, ErrorKind, MaintenanceAction, RequestContext, Result, Session, SessionEvent,
    models::*,
};
use std::time::Duration;
use tokio::time::{Instant, sleep, timeout};

#[derive(Clone, Debug)]
pub struct RecoveryOptions {
    /// Total budget for availability checks and snapshot refresh after the command returns.
    pub timeout: Duration,
    pub retry_interval: Duration,
    /// Require repeated successful version checks before refreshing snapshots.
    pub stable_for: Duration,
    /// Local wait budget for the single maintenance request.
    pub command_timeout: Duration,
}

impl Default for RecoveryOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            retry_interval: Duration::from_millis(500),
            stable_for: Duration::from_secs(1),
            command_timeout: Duration::from_secs(60),
        }
    }
}

impl RecoveryOptions {
    fn validate(&self) -> Result<()> {
        if self.timeout.is_zero()
            || self.timeout > Duration::from_secs(3600)
            || self.retry_interval.is_zero()
            || self.retry_interval >= self.timeout
            || self.stable_for >= self.timeout
            || self.command_timeout.is_zero()
            || self.command_timeout > Duration::from_secs(60)
        {
            return Err(Error::invalid("invalid recovery timings"));
        }
        Ok(())
    }
}

/// Each resource keeps its own error so a core with fewer API capabilities remains usable.
#[derive(Clone, Debug)]
pub struct CoreSnapshot {
    pub version: Version,
    pub config: Result<Config>,
    pub proxies: Result<Proxies>,
    pub proxy_providers: Result<Providers<ProxyProvider>>,
    pub rules: Result<Vec<Rule>>,
    pub rule_providers: Result<Providers<RuleProvider>>,
    pub connections: Result<Option<Connections>>,
}

impl CoreSnapshot {
    fn errors(&self) -> impl Iterator<Item = &Error> {
        [
            self.config.as_ref().err(),
            self.proxies.as_ref().err(),
            self.proxy_providers.as_ref().err(),
            self.rules.as_ref().err(),
            self.rule_providers.as_ref().err(),
            self.connections.as_ref().err(),
        ]
        .into_iter()
        .flatten()
    }

    pub fn is_complete(&self) -> bool {
        self.errors().next().is_none()
    }
}

#[derive(Clone, Debug)]
pub enum CommandOutcome {
    /// The API returned success. Execution may still continue in the core.
    Acknowledged,
    /// The API explicitly returned an error response.
    ReportedFailure(Error),
    /// Delivery or the response was interrupted; the core may have applied the command.
    Indeterminate(Error),
}

#[derive(Clone, Debug)]
pub struct RecoveryReport {
    pub command: Option<CommandOutcome>,
    /// Availability and refreshed data are independent of the command's outcome.
    pub snapshot: Result<CoreSnapshot>,
}

/// Created synchronously to invalidate old UI events before scheduling asynchronous recovery.
/// Consuming run prevents sending the same maintenance command twice through this operation.
pub struct RecoveryOperation {
    context: RequestContext,
    options: RecoveryOptions,
    action: Option<MaintenanceAction>,
}

impl RecoveryOperation {
    pub fn context(&self) -> &RequestContext {
        &self.context
    }

    pub async fn run(self) -> SessionEvent<RecoveryReport> {
        let context = &self.context;
        if context.cancel.is_cancelled() {
            return context.event(Err(Error::cancelled()));
        }
        let command = if let Some(action) = self.action {
            let result = tokio::select! {
                biased;
                _ = context.cancel.cancelled() => Err(Error::cancelled()),
                result = timeout(self.options.command_timeout, context.client.maintenance(action)) => {
                    result.unwrap_or_else(|_| Err(Error::new(ErrorKind::Timeout, "core maintenance", "command response timed out")))
                },
            };
            Some(match result {
                Ok(()) => CommandOutcome::Acknowledged,
                Err(error) if error.status.is_some() => CommandOutcome::ReportedFailure(error),
                Err(error) => CommandOutcome::Indeterminate(error),
            })
        } else {
            None
        };
        let snapshot = tokio::select! {
            biased;
            _ = context.cancel.cancelled() => Err(Error::cancelled()),
            result = timeout(self.options.timeout, context.client.wait_snapshot(&self.options)) => {
                result.unwrap_or_else(|_| Err(Error::new(ErrorKind::Timeout, "recover session", "core did not recover within the time budget")))
            },
        };
        context.event(Ok(RecoveryReport { command, snapshot }))
    }
}

impl Session {
    /// Used on reconnect, foreground resume or a network change. Reopen subscriptions with
    /// operation.context() after applying the refreshed snapshot and checking its session token.
    pub fn recover(&mut self, options: RecoveryOptions) -> Result<RecoveryOperation> {
        self.begin_recovery(options, None)
    }

    pub fn maintain(
        &mut self,
        action: MaintenanceAction,
        options: RecoveryOptions,
    ) -> Result<RecoveryOperation> {
        self.begin_recovery(options, Some(action))
    }

    fn begin_recovery(
        &mut self,
        options: RecoveryOptions,
        action: Option<MaintenanceAction>,
    ) -> Result<RecoveryOperation> {
        options.validate()?;
        Ok(RecoveryOperation {
            context: self.reconnect()?,
            options,
            action,
        })
    }
}

impl CoreClient {
    async fn snapshot(&self, version: Version) -> CoreSnapshot {
        let (config, proxies, proxy_providers, rules, rule_providers, connections) = tokio::join!(
            self.config(),
            self.proxies(),
            self.proxy_providers(),
            self.rules(),
            self.rule_providers(),
            self.connections()
        );
        CoreSnapshot {
            version,
            config,
            proxies,
            proxy_providers,
            rules,
            rule_providers,
            connections,
        }
    }

    async fn wait_snapshot(&self, options: &RecoveryOptions) -> Result<CoreSnapshot> {
        let mut stable: Option<(String, Instant)> = None;
        loop {
            match self.version().await {
                Ok(version) => {
                    let since = match &stable {
                        Some((previous, since)) if previous == &version.version => *since,
                        _ => {
                            let now = Instant::now();
                            stable = Some((version.version.clone(), now));
                            now
                        }
                    };
                    if since.elapsed() >= options.stable_for {
                        let snapshot = self.snapshot(version).await;
                        if let Some(error) = snapshot
                            .errors()
                            .find(|error| error.kind == ErrorKind::Unauthorized)
                        {
                            return Err(error.clone());
                        }
                        if !snapshot.errors().any(retryable) {
                            return Ok(snapshot);
                        }
                        stable = None;
                    }
                }
                Err(error) if retryable(&error) => stable = None,
                Err(error) => return Err(error),
            }
            sleep(options.retry_interval).await;
        }
    }
}

fn retryable(error: &Error) -> bool {
    matches!(error.kind, ErrorKind::Transport | ErrorKind::Timeout)
        || (error.kind == ErrorKind::Http
            && error
                .status
                .is_some_and(|status| status == 408 || status == 429 || status >= 500))
}
