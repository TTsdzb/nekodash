#![allow(dead_code)]
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Method, Request, Uri},
    response::{IntoResponse, Response},
};
use futures_util::future::BoxFuture;
use nekodash_core::{CoreClient, Endpoint};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug)]
pub struct Captured {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}
type Handler = Arc<dyn Fn(Captured) -> BoxFuture<'static, Response> + Send + Sync>;

#[derive(Clone)]
struct ServerState {
    handler: Handler,
    tx: mpsc::UnboundedSender<Captured>,
}

async fn dispatch(State(state): State<ServerState>, request: Request<Body>) -> Response {
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, 1024 * 1024).await {
        Ok(body) => body.to_vec(),
        Err(_) => return (axum::http::StatusCode::BAD_REQUEST, "bad test body").into_response(),
    };
    let capture = Captured {
        method: parts.method,
        uri: parts.uri,
        headers: parts.headers,
        body,
    };
    let _ = state.tx.send(capture.clone());
    (state.handler)(capture).await
}

pub struct Server {
    pub url: String,
    pub received: mpsc::UnboundedReceiver<Captured>,
    task: JoinHandle<()>,
}

impl Server {
    pub async fn start<F, Fut>(handler: F) -> TestResult<Self>
    where
        F: Fn(Captured) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        let (tx, received) = mpsc::unbounded_channel();
        let state = ServerState {
            handler: Arc::new(move |request| Box::pin(handler(request))),
            tx,
        };
        let router = Router::new().fallback(dispatch).with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Ok(Self {
            url,
            received,
            task,
        })
    }
    pub fn client(&self) -> TestResult<CoreClient> {
        Ok(CoreClient::new(Endpoint::new(
            "test",
            "test",
            &self.url,
            "test-secret",
        )?)?)
    }
    pub async fn next(&mut self) -> TestResult<Captured> {
        Ok(
            tokio::time::timeout(Duration::from_secs(3), self.received.recv())
                .await?
                .ok_or_else(|| std::io::Error::other("request channel closed"))?,
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn json(value: serde_json::Value) -> Response {
    axum::Json(value).into_response()
}
pub fn empty() -> Response {
    axum::http::StatusCode::NO_CONTENT.into_response()
}
