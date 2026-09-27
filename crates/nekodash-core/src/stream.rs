use crate::{CoreClient, Error, ErrorKind, Result, models::*};
use futures_util::{SinkExt, StreamExt};
use std::{sync::Arc, time::Duration};
use tokio::{
    runtime::Handle,
    sync::broadcast,
    task::JoinHandle,
    time::{Instant, timeout},
};
use tokio_tungstenite::{
    Connector, connect_async_tls_with_config,
    tungstenite::{self, Message, protocol::WebSocketConfig},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamKind {
    Connections,
    Traffic,
    Memory,
    Logs(LogLevel),
}

impl StreamKind {
    fn path(self) -> &'static str {
        match self {
            Self::Connections => "connections",
            Self::Traffic => "traffic",
            Self::Memory => "memory",
            Self::Logs(_) => "logs",
        }
    }
    fn level(self) -> Option<&'static str> {
        if let Self::Logs(level) = self {
            Some(level.as_str())
        } else {
            None
        }
    }
    fn decode(self, bytes: &[u8]) -> Result<StreamData> {
        let result = match self {
            Self::Connections => serde_json::from_slice(bytes).map(StreamData::Connections),
            Self::Traffic => serde_json::from_slice(bytes).map(StreamData::Traffic),
            Self::Memory => serde_json::from_slice(bytes).map(StreamData::Memory),
            Self::Logs(_) => serde_json::from_slice(bytes).map(StreamData::Log),
        };
        result.map_err(|e| Error::decode("decode WebSocket message", e))
    }
}

#[derive(Clone, Debug)]
pub enum StreamData {
    Connections(Option<Connections>),
    Traffic(Traffic),
    Memory(Memory),
    Log(Log),
}

#[derive(Clone, Debug)]
pub enum StreamState {
    Connecting,
    Connected,
    Reconnecting { delay: Duration },
    Stopped,
}

#[derive(Clone, Debug)]
pub enum StreamEvent {
    State(StreamState),
    Data(Arc<StreamData>),
    Error(Error),
}

#[derive(Clone, Debug)]
pub struct StreamOptions {
    pub handshake_timeout: Duration,
    pub initial_retry: Duration,
    pub max_retry: Duration,
    /// Liveness check interval. Idle logs use an HTTP probe after two intervals;
    /// periodic streams time out after two intervals without incoming frames.
    pub heartbeat: Duration,
    pub max_message_bytes: usize,
    pub queue_capacity: usize,
}

impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(5),
            initial_retry: Duration::from_secs(3),
            max_retry: Duration::from_secs(30),
            heartbeat: Duration::from_secs(30),
            max_message_bytes: 16 * 1024 * 1024,
            queue_capacity: 8,
        }
    }
}

/// Bounded stream queue. Slow consumers receive `Lagged` with the lost event count.
/// Dropping a subscription aborts its socket task immediately.
pub struct Subscription {
    receiver: broadcast::Receiver<StreamEvent>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl Subscription {
    pub async fn recv(&mut self) -> Result<StreamEvent> {
        match self.receiver.recv().await {
            Ok(event) => Ok(event),
            Err(broadcast::error::RecvError::Lagged(count)) => Err(Error::new(
                ErrorKind::Lagged,
                "receive stream",
                format!("consumer missed {count} stream events"),
            )),
            Err(broadcast::error::RecvError::Closed) => Err(Error::cancelled()),
        }
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

impl CoreClient {
    pub fn subscribe(
        &self,
        runtime: &Handle,
        kind: StreamKind,
        options: StreamOptions,
        parent: &CancellationToken,
    ) -> Result<Subscription> {
        if options.handshake_timeout.is_zero()
            || options.initial_retry.is_zero()
            || options.max_retry < options.initial_retry
            || options.heartbeat.is_zero()
            || options.handshake_timeout > Duration::from_secs(3600)
            || options.max_retry > Duration::from_secs(3600)
            || options.heartbeat > Duration::from_secs(3600)
            || options.max_message_bytes == 0
            || !(1..=4096).contains(&options.queue_capacity)
        {
            return Err(Error::invalid("invalid WebSocket limits or timings"));
        }
        let url = self.endpoint().websocket_url(kind.path(), kind.level())?;
        let (sender, receiver) = broadcast::channel(options.queue_capacity);
        let cancel = parent.child_token();
        let task_cancel = cancel.clone();
        let connector = Connector::Rustls(self.tls_config.clone());
        let client = self.clone();
        let task = runtime.spawn(async move {
            tokio::select! {
                biased;
                _ = task_cancel.cancelled() => {},
                _ = run_stream(url, kind, options, &sender, connector, &client) => {},
            }
            let _ = sender.send(StreamEvent::State(StreamState::Stopped));
        });
        Ok(Subscription {
            receiver,
            cancel,
            task,
        })
    }
}

async fn run_stream(
    url: url::Url,
    kind: StreamKind,
    options: StreamOptions,
    sender: &broadcast::Sender<StreamEvent>,
    connector: Connector,
    client: &CoreClient,
) {
    let mut retry = options.initial_retry;
    loop {
        let _ = sender.send(StreamEvent::State(StreamState::Connecting));
        let config = WebSocketConfig::default()
            .max_message_size(Some(options.max_message_bytes))
            .max_frame_size(Some(options.max_message_bytes));
        let connect = timeout(
            options.handshake_timeout,
            connect_async_tls_with_config(
                url.as_str(),
                Some(config),
                false,
                Some(connector.clone()),
            ),
        )
        .await;
        let error = match connect {
            Ok(Ok((mut socket, _))) => {
                let _ = sender.send(StreamEvent::State(StreamState::Connected));
                let mut last_seen = Instant::now();
                let mut heartbeat =
                    tokio::time::interval_at(last_seen + options.heartbeat, options.heartbeat);
                heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let mut probe = std::pin::pin!(client.version());
                let mut probing = false;
                loop {
                    tokio::select! {
                        next = socket.next() => {
                            last_seen = Instant::now();
                            if probing {
                                // New frames establish activity and cancel the now stale HTTP probe.
                                probing = false;
                                probe.set(client.version());
                            }
                            match next {
                                Some(Ok(Message::Text(text))) => {
                                    match kind.decode(text.as_bytes()) {
                                        Ok(data) => { retry = options.initial_retry; let _ = sender.send(StreamEvent::Data(Arc::new(data))); },
                                        Err(error) => { let _ = sender.send(StreamEvent::Error(error)); },
                                    }
                                },
                                Some(Ok(Message::Binary(bytes))) => {
                                    match kind.decode(&bytes) {
                                        Ok(data) => { retry = options.initial_retry; let _ = sender.send(StreamEvent::Data(Arc::new(data))); },
                                        Err(error) => { let _ = sender.send(StreamEvent::Error(error)); },
                                    }
                                },
                                Some(Ok(Message::Close(_))) | None => break Error::new(ErrorKind::Transport, "WebSocket", "peer closed stream"),
                                Some(Ok(Message::Ping(_))) => {
                                    // tungstenite queues the matching pong while reading the ping.
                                    if let Err(error) = socket.flush().await { break ws_error(error); }
                                },
                                Some(Ok(_)) => {},
                                Some(Err(error)) => break ws_error(error),
                            }
                        },
                        result = &mut probe, if probing => {
                            probing = false;
                            match result {
                                Ok(_) => {
                                    last_seen = Instant::now();
                                    retry = options.initial_retry;
                                },
                                Err(error) => break error,
                            }
                        },
                        _ = heartbeat.tick() => {
                            if last_seen.elapsed() >= options.heartbeat.saturating_mul(2) {
                                if matches!(kind, StreamKind::Logs(_)) {
                                    // Mihomo's log handler writes frames without reading Ping/Pong.
                                    // Probe core reachability while still receiving logs and cancellation.
                                    if !probing {
                                        probe.set(client.version());
                                        probing = true;
                                    }
                                } else {
                                    break Error::new(ErrorKind::Timeout, "WebSocket", "stream data timed out");
                                }
                            }
                        },
                    }
                }
            }
            Ok(Err(error)) => ws_error(error),
            Err(_) => Error::new(
                ErrorKind::Timeout,
                "connect WebSocket",
                "handshake timed out",
            ),
        };
        let terminal = matches!(error.kind, ErrorKind::Unauthorized | ErrorKind::Unsupported);
        let _ = sender.send(StreamEvent::Error(error));
        if terminal {
            return;
        }
        let _ = sender.send(StreamEvent::State(StreamState::Reconnecting {
            delay: retry,
        }));
        tokio::time::sleep(retry).await;
        retry = retry.saturating_mul(2).min(options.max_retry);
    }
}

fn ws_error(error: tungstenite::Error) -> Error {
    match error {
        tungstenite::Error::Http(response) => Error::http(
            "connect WebSocket",
            response.status().as_u16(),
            "WebSocket handshake rejected",
        ),
        tungstenite::Error::Capacity(_) => Error::new(
            ErrorKind::ResponseTooLarge,
            "WebSocket",
            "message exceeds configured size limit",
        ),
        tungstenite::Error::Tls(_) => {
            Error::new(ErrorKind::Transport, "WebSocket", "TLS negotiation failed")
        }
        _ => Error::new(
            ErrorKind::Transport,
            "WebSocket",
            "WebSocket transport failed",
        ),
    }
}
