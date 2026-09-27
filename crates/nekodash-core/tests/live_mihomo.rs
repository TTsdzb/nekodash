//! This test owns its process, temporary home and loopback controller.
//! Run explicitly with MIHOMO_TEST_BIN=/path/to/mihomo and --ignored.
mod common;
use axum::response::IntoResponse;
use common::{Server, TestResult, empty};
use nekodash_core::{
    CancellationToken, ClientOptions, CommandOutcome, CoreClient, Endpoint, MaintenanceAction,
    Probe, RecoveryOptions, Session, StreamData, StreamEvent, StreamKind, StreamOptions,
    StreamState, models::LogLevel,
};
use serde_json::json;
use std::{process::Stdio, time::Duration};

struct DnsFixture {
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl DnsFixture {
    async fn start() -> TestResult<Self> {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
        let address = socket.local_addr()?;
        let task = tokio::spawn(async move {
            let mut buffer = [0; 4096];
            while let Ok((size, peer)) = socket.recv_from(&mut buffer).await {
                if let Some(response) = buffer.get(..size).and_then(dns_answer)
                    && socket.send_to(&response, peer).await.is_err()
                {
                    break;
                }
            }
        });
        Ok(Self { address, task })
    }
}

impl Drop for DnsFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

// This fixture handles one uncompressed IN/A question and returns 127.0.0.42.
fn dns_answer(query: &[u8]) -> Option<Vec<u8>> {
    if query.get(4..6)? != [0, 1] {
        return None;
    }
    let mut offset = 12;
    loop {
        let length = usize::from(*query.get(offset)?);
        if length > 63 {
            return None;
        }
        offset += 1 + length;
        if length == 0 {
            break;
        }
    }
    let end = offset + 4;
    if query.get(offset..end)? != [0, 1, 0, 1] {
        return None;
    }
    let mut response = query.get(..2)?.to_vec();
    response.extend_from_slice(&[0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0]);
    response.extend_from_slice(query.get(12..end)?);
    response.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 127, 0, 0, 42]);
    Some(response)
}

#[tokio::test]
#[ignore = "starts an isolated core using MIHOMO_TEST_BIN"]
async fn isolated_mihomo_http_streams_and_mutations() -> TestResult {
    let binary = std::env::var_os("MIHOMO_TEST_BIN")
        .ok_or_else(|| std::io::Error::other("set MIHOMO_TEST_BIN to the test binary"))?;
    let home = tempfile::tempdir()?;
    let reservation = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = reservation.local_addr()?.port();
    let secret = format!("nekodash-test-{}-{port}", std::process::id());
    let dns = DnsFixture::start().await?;
    let subscription = Server::start(|_| async {
        (
            [(
                "subscription-userinfo",
                "upload=0; download=0; total=0; expire=-1",
            )],
            "proxies:\n  - name: Subscription Direct\n    type: direct\n",
        )
            .into_response()
    })
    .await?;
    // Mihomo treats sub-millisecond URL tests (delay == 0) as failures.
    let target = Server::start(|_| async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        empty()
    })
    .await?;
    let config = format!(
        r#"external-controller: 127.0.0.1:{port}
secret: '{secret}'
mixed-port: 0
port: 0
socks-port: 0
redir-port: 0
tproxy-port: 0
allow-lan: false
mode: rule
log-level: debug
ipv6: false
dns:
  enable: true
  nameserver: ['{dns}']
tun:
  enable: false
profile:
  store-selected: false
proxy-providers:
  subscription:
    type: http
    url: '{subscription}/provider.yaml'
    path: subscription.yaml
    interval: 86400
    health-check:
      enable: false
  fixture:
    type: file
    path: provider.yaml
    health-check:
      enable: false
      url: '{target}/generate_204'
rule-providers:
  fixture-rules:
    type: file
    behavior: domain
    format: yaml
    path: rules.yaml
proxy-groups:
  - name: Test Group
    type: select
    proxies: [DIRECT, REJECT]
rules:
  - RULE-SET,fixture-rules,Test Group
  - MATCH,Test Group
"#,
        target = target.url,
        subscription = subscription.url,
        dns = dns.address,
    );
    std::fs::write(home.path().join("config.yaml"), config)?;
    std::fs::write(
        home.path().join("provider.yaml"),
        "proxies:\n  - name: Fixture Direct\n    type: direct\n",
    )?;
    std::fs::write(
        home.path().join("rules.yaml"),
        "payload:\n  - example.test\n",
    )?;
    let output_path = home.path().join("mihomo.log");
    let stdout = std::fs::File::create(&output_path)?;
    let stderr = stdout.try_clone()?;
    drop(reservation);
    let mut process = tokio::process::Command::new(binary)
        .arg("-d")
        .arg(home.path())
        .arg("-f")
        .arg(home.path().join("config.yaml"))
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .kill_on_drop(true)
        .spawn()?;
    let client = CoreClient::new(Endpoint::new(
        "isolated",
        "test",
        &format!("http://127.0.0.1:{port}"),
        secret.clone(),
    )?)?;
    let result: TestResult = async {
        let version = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(status) = process.try_wait()? {
                    return Err(std::io::Error::other(format!(
                        "isolated core exited: {status}; {}",
                        std::fs::read_to_string(&output_path)?
                    ))
                    .into());
                }
                if let Ok(version) = client.version().await {
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(version);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await??;
        eprintln!("isolated core: {}", version.version);
        assert!(version.meta);
        assert!(client.proxies().await?.proxies.contains_key("Test Group"));
        assert!(
            client
                .proxy_providers()
                .await?
                .providers
                .contains_key("fixture")
        );
        // Force the HTTP provider fetch so this also covers asynchronously initialized cores.
        client.update_proxy_provider("subscription").await?;
        let providers = client.proxy_providers().await?.providers;
        assert!(providers.contains_key("fixture"));
        assert_eq!(
            providers
                .get("subscription")
                .and_then(|p| p.subscription_info.as_ref())
                .map(|info| info.expire),
            Some(-1)
        );
        let response = client.dns_query("fixture.test", "A").await?;
        assert_eq!(response.status, 0);
        let answer = response
            .answer
            .as_ref()
            .and_then(|answers| answers.first())
            .ok_or_else(|| std::io::Error::other("missing isolated DNS answer"))?;
        assert_eq!(answer.data, "127.0.0.42");
        assert_eq!(answer.kind, 1);
        assert!(
            client
                .rule_providers()
                .await?
                .providers
                .contains_key("fixture-rules")
        );
        assert_eq!(client.rules().await?.len(), 2);
        client.select_proxy("Test Group", "REJECT").await?;
        assert_eq!(
            client
                .proxies()
                .await?
                .proxies
                .get("Test Group")
                .map(|p| p.now.as_str()),
            Some("REJECT")
        );
        client.select_proxy("Test Group", "DIRECT").await?;
        client
            .patch_config(serde_json::from_value(json!({"mode":"global"}))?)
            .await?;
        assert_eq!(client.config().await?.0.get("mode"), Some(&json!("global")));
        client
            .patch_config(serde_json::from_value(json!({"mode":"rule"}))?)
            .await?;
        client.update_proxy_provider("fixture").await?;
        client.update_rule_provider("fixture-rules").await?;
        client.health_check_provider("fixture").await?;
        let mut probe = Probe {
            node: "DIRECT".into(),
            provider: None,
            url: format!("{}/generate_204", target.url),
            timeout_ms: 1000,
        };
        client.test_proxy(&probe).await?;
        probe.node = "Fixture Direct".into();
        probe.provider = Some("fixture".into());
        client.test_proxy(&probe).await?;
        client.test_group("Test Group", &probe.url, 1000).await?;
        client.connections().await?;
        client.close_all_connections().await?;
        client.set_rule_disabled(0, true).await?;
        client.set_rule_disabled(0, false).await?;
        client.maintenance(MaintenanceAction::FlushDns).await?;
        client.maintenance(MaintenanceAction::FlushFakeIp).await?;
        let cancel = CancellationToken::new();
        for kind in [
            StreamKind::Traffic,
            StreamKind::Memory,
            StreamKind::Connections,
        ] {
            let mut subscription = client.subscribe(
                &tokio::runtime::Handle::current(),
                kind,
                StreamOptions::default(),
                &cancel,
            )?;
            let data = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match subscription.recv().await? {
                        StreamEvent::Data(data) => return Ok::<_, nekodash_core::Error>(data),
                        StreamEvent::Error(error) => return Err(error),
                        _ => {}
                    }
                }
            })
            .await??;
            assert!(matches!(
                (kind, data.as_ref()),
                (StreamKind::Traffic, StreamData::Traffic(_))
                    | (StreamKind::Memory, StreamData::Memory(_))
                    | (StreamKind::Connections, StreamData::Connections(_))
            ));
        }
        cancel.cancel();
        client
            .patch_config(serde_json::from_value(json!({"log-level":"silent"}))?)
            .await?;
        let mut logs = client.subscribe(
            &tokio::runtime::Handle::current(),
            StreamKind::Logs(LogLevel::Info),
            StreamOptions {
                heartbeat: Duration::from_millis(50),
                ..Default::default()
            },
            &CancellationToken::new(),
        )?;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match logs.recv().await? {
                    StreamEvent::State(StreamState::Connected) => {
                        return Ok::<_, nekodash_core::Error>(());
                    }
                    StreamEvent::Error(error) => return Err(error),
                    _ => {}
                }
            }
        })
        .await??;
        assert!(
            tokio::time::timeout(Duration::from_millis(400), logs.recv())
                .await
                .is_err()
        );
        logs.cancel();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), logs.recv()).await??,
            StreamEvent::State(StreamState::Stopped)
        ));
        client.reload_config().await?;
        client.version().await?;
        let mut session = Session::default();
        let original = session.switch(client.endpoint().clone(), ClientOptions::default())?;
        let report = session
            .maintain(MaintenanceAction::FlushDns, RecoveryOptions::default())?
            .run()
            .await;
        assert!(!session.accepts(&original.token));
        assert!(session.accepts(&report.token));
        let report = report.result?;
        assert!(matches!(report.command, Some(CommandOutcome::Acknowledged)));
        assert!(report.snapshot?.is_complete());

        // Linux Mihomo restarts with exec, retaining this test's child PID for cleanup.
        // Other platforms need a process-tree harness to own any replacement child.
        #[cfg(target_os = "linux")]
        {
            let operation =
                session.maintain(MaintenanceAction::Restart, RecoveryOptions::default())?;
            let refreshed = operation.context().clone();
            let event = operation.run().await;
            assert!(session.accepts(&event.token));
            let report = event.result?;
            assert!(matches!(report.command, Some(CommandOutcome::Acknowledged)));
            let snapshot = report.snapshot?;
            assert!(snapshot.is_complete());
            assert_eq!(snapshot.version.version, version.version);
            assert!(process.try_wait()?.is_none());
            let mut traffic = refreshed
                .subscribe(
                    &tokio::runtime::Handle::current(),
                    StreamKind::Traffic,
                    StreamOptions::default(),
                )
                .result?;
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let event = traffic.recv().await;
                    assert!(session.accepts(&event.token));
                    match event.result? {
                        StreamEvent::Data(data) => {
                            assert!(matches!(data.as_ref(), StreamData::Traffic(_)));
                            return Ok::<_, nekodash_core::Error>(());
                        }
                        StreamEvent::Error(error) => return Err(error),
                        _ => {}
                    }
                }
            })
            .await??;
        }
        Ok(())
    }
    .await;
    // The handle belongs only to the process started above. Always reap it before returning.
    if process.try_wait()?.is_none() {
        process.kill().await?;
    }
    process.wait().await?;
    if result.is_err() {
        eprintln!(
            "isolated core output:\n{}",
            std::fs::read_to_string(output_path)?
        );
    }
    result
}
