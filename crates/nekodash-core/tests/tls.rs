use futures_util::SinkExt;
use nekodash_core::{
    CancellationToken, ClientOptions, CoreClient, Endpoint, ErrorKind, StreamData, StreamEvent,
    StreamKind, StreamOptions,
};
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
struct TlsServer {
    url: String,
    pem: String,
    task: JoinHandle<()>,
}
impl Drop for TlsServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl TlsServer {
    async fn start<F, Fut>(name: &str, handler: F) -> TestResult<Self>
    where
        F: Fn(TlsStream<TcpStream>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = TestResult> + Send + 'static,
    {
        let certificate = rcgen::generate_simple_self_signed(vec![name.to_owned()])?;
        let pem = certificate.serialize_pem()?;
        let cert = CertificateDer::from(certificate.serialize_der()?);
        let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(
            certificate.serialize_private_key_der(),
        ));
        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()?
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("https://{}", listener.local_addr()?);
        let handler = Arc::new(handler);
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    incoming=listener.accept()=>{
                        let Ok((socket,_))=incoming else {break;};
                        let acceptor=acceptor.clone();let handler=handler.clone();
                        tasks.spawn(async move {if let Ok(stream)=acceptor.accept(socket).await{let _=handler(stream).await;}});
                    },
                    _=tasks.join_next(),if !tasks.is_empty()=>{},
                }
            }
        });
        Ok(Self { url, pem, task })
    }
    fn client(&self, trust: bool) -> TestResult<CoreClient> {
        Ok(CoreClient::with_options(
            Endpoint::new("tls", "", &self.url, "secret")?,
            ClientOptions {
                additional_ca_pem: if trust {
                    vec![self.pem.clone()]
                } else {
                    vec![]
                },
                ..Default::default()
            },
        )?)
    }
}

async fn http_response(mut stream: TlsStream<TcpStream>) -> TestResult {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
        request.push(stream.read_u8().await?);
    }
    let body = r#"{"version":"tls-test","meta":true}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn https_requires_trusted_ca_and_matching_hostname() -> TestResult {
    let server = TlsServer::start("127.0.0.1", http_response).await?;
    assert!(matches!(server.client(false)?.version().await,Err(e) if e.kind==ErrorKind::Transport));
    assert_eq!(server.client(true)?.version().await?.version, "tls-test");
    let wrong = TlsServer::start("wrong.example", http_response).await?;
    assert!(matches!(wrong.client(true)?.version().await,Err(e) if e.kind==ErrorKind::Transport));
    Ok(())
}

#[tokio::test]
async fn wss_uses_same_trust_configuration() -> TestResult {
    let server = TlsServer::start("127.0.0.1", |stream| async move {
        let mut socket = tokio_tungstenite::accept_async(stream).await?;
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                r#"{"up":5,"down":9}"#.into(),
            ))
            .await?;
        socket.close(None).await?;
        Ok(())
    })
    .await?;
    for trust in [false, true] {
        let mut sub = server.client(trust)?.subscribe(
            &tokio::runtime::Handle::current(),
            StreamKind::Traffic,
            StreamOptions::default(),
            &CancellationToken::new(),
        )?;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match sub.recv().await? {
                    StreamEvent::Error(e) => {
                        assert!(!trust);
                        assert_eq!(e.kind, ErrorKind::Transport);
                        return Ok::<(), nekodash_core::Error>(());
                    }
                    StreamEvent::Data(data) => {
                        assert!(trust);
                        assert!(matches!(data.as_ref(),StreamData::Traffic(t) if t.up==5));
                        return Ok(());
                    }
                    _ => {}
                }
            }
        })
        .await??;
    }
    Ok(())
}
