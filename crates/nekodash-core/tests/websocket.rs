use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::future::BoxFuture;
use nekodash_core::{
    CancellationToken, ClientOptions, CoreClient, Endpoint, ErrorKind, RecoveryOptions, Session,
    StreamData, StreamEvent, StreamKind, StreamOptions, StreamState, Subscription,
    models::LogLevel,
};
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Handler = Arc<dyn Fn(WebSocket) -> BoxFuture<'static, ()> + Send + Sync>;
#[derive(Clone)]
struct WsState {
    handler: Handler,
    visits: Arc<AtomicUsize>,
    tx: mpsc::UnboundedSender<Uri>,
    reject: bool,
    probes: Arc<AtomicUsize>,
    probe_status: StatusCode,
    probe_delay: Duration,
}

async fn version(State(state): State<WsState>, headers: HeaderMap) -> Response {
    state.probes.fetch_add(1, Ordering::SeqCst);
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Bearer secret &/%")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    tokio::time::sleep(state.probe_delay).await;
    (
        state.probe_status,
        axum::Json(serde_json::json!({"version":"1.19.31","meta":true})),
    )
        .into_response()
}

async fn upgrade(ws: WebSocketUpgrade, State(state): State<WsState>, uri: Uri) -> Response {
    state.visits.fetch_add(1, Ordering::SeqCst);
    let _ = state.tx.send(uri.clone());
    let params: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    if state.reject || params.get("token").map(String::as_str) != Some("secret &/%") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| (state.handler)(socket))
}

struct WsServer {
    url: String,
    visits: Arc<AtomicUsize>,
    probes: Arc<AtomicUsize>,
    received: mpsc::UnboundedReceiver<Uri>,
    task: JoinHandle<()>,
}
impl Drop for WsServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl WsServer {
    async fn start<F, Fut>(reject: bool, handler: F) -> TestResult<Self>
    where
        F: Fn(WebSocket) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self::with_probe(reject, StatusCode::OK, Duration::ZERO, handler).await
    }
    async fn with_probe<F, Fut>(
        reject: bool,
        probe_status: StatusCode,
        probe_delay: Duration,
        handler: F,
    ) -> TestResult<Self>
    where
        F: Fn(WebSocket) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let visits = Arc::new(AtomicUsize::new(0));
        let probes = Arc::new(AtomicUsize::new(0));
        let (tx, received) = mpsc::unbounded_channel();
        let state = WsState {
            handler: Arc::new(move |socket| Box::pin(handler(socket))),
            visits: visits.clone(),
            tx,
            reject,
            probes: probes.clone(),
            probe_status,
            probe_delay,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/prefix", listener.local_addr()?);
        let router = Router::new()
            .route("/prefix/version", get(version))
            .route("/prefix/{kind}", get(upgrade))
            .with_state(state);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Ok(Self {
            url,
            visits,
            probes,
            received,
            task,
        })
    }
    fn client(&self) -> TestResult<CoreClient> {
        Ok(CoreClient::new(Endpoint::new(
            "ws",
            "",
            &self.url,
            "secret &/%",
        )?)?)
    }
}

fn options() -> StreamOptions {
    StreamOptions {
        initial_retry: Duration::from_millis(20),
        max_retry: Duration::from_millis(80),
        queue_capacity: 64,
        heartbeat: Duration::from_millis(50),
        ..Default::default()
    }
}
async fn next(sub: &mut Subscription) -> TestResult<StreamEvent> {
    Ok(tokio::time::timeout(Duration::from_secs(3), sub.recv()).await??)
}

#[tokio::test]
async fn malformed_messages_are_reported_and_following_data_is_delivered() -> TestResult {
    let mut server = WsServer::start(false, |mut socket| async move {
        let _ = socket
            .send(Message::Text("bad json with secret &/%".into()))
            .await;
        let _ = socket
            .send(Message::Text(r#"{"type":"info","payload":"hello"}"#.into()))
            .await;
        while socket.recv().await.is_some() {}
    })
    .await?;
    let mut sub = server.client()?.subscribe(
        &tokio::runtime::Handle::current(),
        StreamKind::Logs(LogLevel::Debug),
        options(),
        &CancellationToken::new(),
    )?;
    let mut saw_decode = false;
    loop {
        match next(&mut sub).await? {
            StreamEvent::Error(error) => {
                assert_eq!(error.kind, ErrorKind::Decode);
                assert!(!format!("{error:?}").contains("secret"));
                saw_decode = true;
            }
            StreamEvent::Data(data) => {
                assert!(matches!(data.as_ref(),StreamData::Log(log) if log.payload=="hello"));
                break;
            }
            _ => {}
        }
    }
    assert!(saw_decode);
    let uri = server
        .received
        .recv()
        .await
        .ok_or_else(|| std::io::Error::other("missing WS request"))?;
    assert_eq!(uri.path(), "/prefix/logs");
    let params: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    assert_eq!(params.get("level").map(String::as_str), Some("debug"));
    assert_eq!(params.get("token").map(String::as_str), Some("secret &/%"));
    sub.cancel();
    Ok(())
}

#[tokio::test]
async fn reconnects_after_close_and_parent_cancellation_stops_retries() -> TestResult {
    let server = WsServer::start(false, |mut socket| async move {
        let _ = socket
            .send(Message::Text(r#"{"up":12,"down":34}"#.into()))
            .await;
        let _ = socket.send(Message::Close(None)).await;
    })
    .await?;
    let parent = CancellationToken::new();
    let mut sub = server.client()?.subscribe(
        &tokio::runtime::Handle::current(),
        StreamKind::Traffic,
        options(),
        &parent,
    )?;
    let mut data_count = 0;
    let mut reconnect = false;
    while data_count < 2 {
        match next(&mut sub).await? {
            StreamEvent::Data(data) => {
                assert!(matches!(data.as_ref(),StreamData::Traffic(t) if t.up==12&&t.down==34));
                data_count += 1;
            }
            StreamEvent::State(StreamState::Reconnecting { .. }) => reconnect = true,
            _ => {}
        }
    }
    assert!(reconnect);
    parent.cancel();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !sub.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let count = server.visits.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.visits.load(Ordering::SeqCst), count);
    Ok(())
}

#[tokio::test]
async fn rejected_auth_is_terminal_and_secrets_are_not_in_errors() -> TestResult {
    let server = WsServer::start(true, |_| async {}).await?;
    let mut sub = server.client()?.subscribe(
        &tokio::runtime::Handle::current(),
        StreamKind::Memory,
        options(),
        &CancellationToken::new(),
    )?;
    loop {
        if let StreamEvent::Error(error) = next(&mut sub).await? {
            assert_eq!(error.kind, ErrorKind::Unauthorized);
            assert!(!format!("{error:?}").contains("secret"));
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(sub.is_finished());
    assert_eq!(server.visits.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn idle_logs_resume_without_pong() -> TestResult {
    let server = WsServer::start(false, |mut socket| async move {
        // Like Mihomo, this server writes logs but never reads client control frames.
        tokio::time::sleep(Duration::from_millis(350)).await;
        let _ = socket
            .send(Message::Text(
                r#"{"type":"info","payload":"after idle"}"#.into(),
            ))
            .await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    })
    .await?;
    let mut sub = server.client()?.subscribe(
        &tokio::runtime::Handle::current(),
        StreamKind::Logs(LogLevel::Info),
        options(),
        &CancellationToken::new(),
    )?;
    loop {
        match next(&mut sub).await? {
            StreamEvent::Error(error) => return Err(error.into()),
            StreamEvent::Data(data) => {
                assert!(
                    matches!(data.as_ref(), StreamData::Log(log) if log.payload == "after idle")
                );
                break;
            }
            StreamEvent::State(StreamState::Reconnecting { .. } | StreamState::Stopped) => {
                return Err(std::io::Error::other("idle stream was interrupted").into());
            }
            _ => {}
        }
    }
    assert_eq!(server.visits.load(Ordering::SeqCst), 1);
    assert!(server.probes.load(Ordering::SeqCst) > 0);
    assert!(!sub.is_finished());
    Ok(())
}

#[tokio::test]
async fn idle_log_probe_failure_reconnects_and_auth_failure_stops() -> TestResult {
    for (status, kind) in [
        (StatusCode::SERVICE_UNAVAILABLE, ErrorKind::Http),
        (StatusCode::UNAUTHORIZED, ErrorKind::Unauthorized),
    ] {
        let server = WsServer::with_probe(false, status, Duration::ZERO, |socket| async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            drop(socket);
        })
        .await?;
        let mut sub = server.client()?.subscribe(
            &tokio::runtime::Handle::current(),
            StreamKind::Logs(LogLevel::Info),
            options(),
            &CancellationToken::new(),
        )?;
        loop {
            if let StreamEvent::Error(error) = next(&mut sub).await? {
                assert_eq!(error.kind, kind);
                assert_eq!(error.status, Some(status.as_u16()));
                break;
            }
        }
        let event = next(&mut sub).await?;
        if status == StatusCode::UNAUTHORIZED {
            assert!(matches!(event, StreamEvent::State(StreamState::Stopped)));
            assert_eq!(server.visits.load(Ordering::SeqCst), 1);
        } else {
            assert!(matches!(
                event,
                StreamEvent::State(StreamState::Reconnecting { .. })
            ));
            while !matches!(
                next(&mut sub).await?,
                StreamEvent::State(StreamState::Connected)
            ) {}
            assert_eq!(server.visits.load(Ordering::SeqCst), 2);
        }
    }
    Ok(())
}

#[tokio::test]
async fn slow_log_probe_keeps_receiving_and_can_be_cancelled() -> TestResult {
    let server = WsServer::with_probe(
        false,
        StatusCode::OK,
        Duration::from_secs(2),
        |mut socket| async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let _ = socket
                .send(Message::Text(
                    r#"{"type":"info","payload":"during probe"}"#.into(),
                ))
                .await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        },
    )
    .await?;
    let parent = CancellationToken::new();
    let mut sub = server.client()?.subscribe(
        &tokio::runtime::Handle::current(),
        StreamKind::Logs(LogLevel::Info),
        options(),
        &parent,
    )?;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match next(&mut sub).await? {
                StreamEvent::Error(error) => return Err(error.into()),
                StreamEvent::Data(data) => {
                    assert!(matches!(data.as_ref(), StreamData::Log(log) if log.payload == "during probe"));
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                },
                _ => {},
            }
        }
    }).await??;
    assert_eq!(server.probes.load(Ordering::SeqCst), 1);
    // The delivered log cancels the first probe; cancel the subscription during the next one.
    tokio::time::timeout(Duration::from_secs(1), async {
        while server.probes.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    parent.cancel();
    assert!(matches!(
        tokio::time::timeout(Duration::from_millis(500), sub.recv()).await??,
        StreamEvent::State(StreamState::Stopped)
    ));
    assert_eq!(server.visits.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn stalled_periodic_streams_time_out() -> TestResult {
    for kind in [
        StreamKind::Traffic,
        StreamKind::Memory,
        StreamKind::Connections,
    ] {
        let server = WsServer::start(false, |socket| async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            drop(socket);
        })
        .await?;
        let mut sub = server.client()?.subscribe(
            &tokio::runtime::Handle::current(),
            kind,
            options(),
            &CancellationToken::new(),
        )?;
        loop {
            if let StreamEvent::Error(error) = next(&mut sub).await? {
                assert_eq!(error.kind, ErrorKind::Timeout);
                break;
            }
        }
        assert_eq!(server.probes.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[tokio::test]
async fn bounded_queue_reports_lost_events() -> TestResult {
    let server = WsServer::start(false, |mut socket| async move {
        for _ in 0..200 {
            if socket
                .send(Message::Text(r#"{"inuse":10}"#.into()))
                .await
                .is_err()
            {
                return;
            }
        }
        while socket.recv().await.is_some() {}
    })
    .await?;
    let mut session = Session::default();
    let context = session.switch(
        Endpoint::new("ws", "", &server.url, "secret &/%")?,
        ClientOptions::default(),
    )?;
    let invalid = context.subscribe(
        &tokio::runtime::Handle::current(),
        StreamKind::Memory,
        StreamOptions {
            queue_capacity: 0,
            ..options()
        },
    );
    assert!(session.accepts(&invalid.token));
    assert!(matches!(invalid.result, Err(error) if error.kind == ErrorKind::InvalidInput));
    let mut sub = context
        .subscribe(
            &tokio::runtime::Handle::current(),
            StreamKind::Memory,
            StreamOptions {
                queue_capacity: 2,
                ..options()
            },
        )
        .result?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let event = sub.recv().await;
    assert!(session.accepts(&event.token));
    assert!(matches!(event.result,Err(e) if e.kind==ErrorKind::Lagged));
    session.reconnect()?;
    assert!(!session.accepts(&event.token));
    Ok(())
}

#[tokio::test]
async fn recovery_invalidates_old_stream_events_and_reopens_scoped_subscriptions() -> TestResult {
    let server = WsServer::start(false, |mut socket| async move {
        let _ = socket.send(Message::Text("bad json".into())).await;
        let _ = socket.send(Message::Text(r#"{"inuse":42}"#.into())).await;
        while socket.recv().await.is_some() {}
    })
    .await?;
    let mut session = Session::default();
    let context = session.switch(
        Endpoint::new("ws", "", &server.url, "secret &/%")?,
        ClientOptions::default(),
    )?;
    let mut old = context
        .subscribe(
            &tokio::runtime::Handle::current(),
            StreamKind::Memory,
            options(),
        )
        .result?;
    let mut saw_decode = false;
    let queued = loop {
        let event = tokio::time::timeout(Duration::from_secs(2), old.recv()).await?;
        assert!(session.accepts(&event.token));
        match &event.result {
            Ok(StreamEvent::Error(error)) if error.kind == ErrorKind::Decode => saw_decode = true,
            Ok(StreamEvent::Data(_)) => break event,
            _ => {}
        }
    };
    assert!(saw_decode);
    let operation = session.recover(RecoveryOptions {
        stable_for: Duration::ZERO,
        ..Default::default()
    })?;
    let current = operation.context().clone();
    assert!(!session.accepts(&queued.token));
    let recovered = operation.run().await;
    assert!(session.accepts(&recovered.token));
    assert_eq!(recovered.result?.snapshot?.version.version, "1.19.31");
    tokio::time::timeout(Duration::from_secs(1), async {
        while !old.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let stopped = old.recv().await;
    assert!(!session.accepts(&stopped.token));
    assert!(matches!(
        stopped.result?,
        StreamEvent::State(StreamState::Stopped)
    ));
    let closed = old.recv().await;
    assert_eq!(closed.token, context.token);
    assert!(matches!(closed.result, Err(error) if error.kind == ErrorKind::Cancelled));

    let mut fresh = current
        .subscribe(
            &tokio::runtime::Handle::current(),
            StreamKind::Memory,
            options(),
        )
        .result?;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), fresh.recv()).await?;
        assert!(session.accepts(&event.token));
        if let StreamEvent::Data(data) = event.result? {
            assert!(matches!(data.as_ref(), StreamData::Memory(memory) if memory.inuse == 42));
            break;
        }
    }
    assert_eq!(server.visits.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn oversize_websocket_message_is_bounded() -> TestResult {
    let server = WsServer::start(false, |mut socket| async move {
        let _ = socket.send(Message::Text("x".repeat(1000).into())).await;
    })
    .await?;
    let mut sub = server.client()?.subscribe(
        &tokio::runtime::Handle::current(),
        StreamKind::Memory,
        StreamOptions {
            max_message_bytes: 32,
            ..options()
        },
        &CancellationToken::new(),
    )?;
    loop {
        if let StreamEvent::Error(error) = next(&mut sub).await? {
            assert_eq!(error.kind, ErrorKind::ResponseTooLarge);
            break;
        }
    }
    Ok(())
}
