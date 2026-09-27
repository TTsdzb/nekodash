mod common;
use axum::{http::StatusCode, response::IntoResponse};
use common::{Server, TestResult, empty, json as response_json};
use nekodash_core::{
    CancellationToken, ClientOptions, CoreClient, Endpoint, ErrorKind, MaintenanceAction, Probe,
    models::Fields,
};
use serde_json::{Value, json};
use std::time::Duration;

#[tokio::test]
async fn reads_snapshots_and_preserves_core_extensions() -> TestResult {
    let mut server = Server::start(|r| async move {
        response_json(match r.uri.path() {
            "/version" => json!({"version":"v1.19.31","meta":true,"vendor":"future"}),
            "/configs" => json!({"mode":"rule","tun":{"enable":false},"future-key":[1,2]}),
            "/proxies" => json!({"proxies":{"auto":{"name":"auto","type":"URLTest","all":["DIRECT"],"future-key":7}}}),
            "/providers/proxies" => json!({"providers":{"p":{"name":"p","proxies":[],"subscriptionInfo":{"Total":1000}}}}),
            "/providers/rules" => json!({"providers":{"r":{"name":"r","ruleCount":42}}}),
            "/connections" => json!({"connections":null,"uploadTotal":7,"downloadTotal":8}),
            "/dns/query" => json!({"status":0,"Answer":[{"TTL":60,"data":"::1","name":"localhost.","type":28}],"AD":false}),
            _ => Value::Null,
        })
    }).await?;
    let client = server.client()?;
    assert_eq!(
        client.version().await?.additional.get("vendor"),
        Some(&json!("future"))
    );
    assert_eq!(
        client.config().await?.0.get("future-key"),
        Some(&json!([1, 2]))
    );
    assert_eq!(
        client
            .proxies()
            .await?
            .proxies
            .get("auto")
            .and_then(|p| p.additional.get("future-key")),
        Some(&json!(7))
    );
    assert_eq!(client.proxy_providers().await?.providers.len(), 1);
    assert_eq!(client.rule_providers().await?.providers.len(), 1);
    assert_eq!(client.connections().await?.map(|c| c.upload_total), Some(7));
    assert_eq!(
        client
            .dns_query("localhost", "AAAA")
            .await?
            .answer
            .map(|v| v.len()),
        Some(1)
    );
    for _ in 0..7 {
        assert_eq!(
            server
                .next()
                .await?
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer test-secret")
        );
    }
    Ok(())
}

#[tokio::test]
async fn mutations_encode_segments_and_accept_empty_success() -> TestResult {
    let mut server = Server::start(|_| async { empty() }).await?;
    let client = CoreClient::new(Endpoint::new(
        "one",
        "",
        &format!("{}/api/core", server.url),
        "s&= +/%",
    )?)?;
    client.select_proxy("香港 /?%#", "日本").await?;
    let request = server.next().await?;
    assert_eq!(request.method, "PUT");
    assert_eq!(
        request.uri.path(),
        "/api/core/proxies/%E9%A6%99%E6%B8%AF%20%2F%3F%25%23"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body)?,
        json!({"name":"日本"})
    );
    assert!(request.uri.query().is_none());
    client.unfix_proxy("auto").await?;
    assert_eq!(server.next().await?.method, "DELETE");
    client.health_check_provider("p/q").await?;
    assert_eq!(
        server.next().await?.uri.path(),
        "/api/core/providers/proxies/p%2Fq/healthcheck"
    );
    client.update_proxy_provider("p").await?;
    assert_eq!(server.next().await?.method, "PUT");
    client.update_rule_provider("r").await?;
    assert_eq!(
        server.next().await?.uri.path(),
        "/api/core/providers/rules/r"
    );
    client.set_rule_disabled(12, true).await?;
    let request = server.next().await?;
    assert_eq!(request.method, "PATCH");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body)?,
        json!({"12":true})
    );
    client.close_connection("id/1").await?;
    assert_eq!(
        server.next().await?.uri.path(),
        "/api/core/connections/id%2F1"
    );
    client.close_all_connections().await?;
    assert_eq!(server.next().await?.uri.path(), "/api/core/connections");
    let patch: Fields = serde_json::from_value(
        json!({"mode":"global","dns":{"nameserver":["https://dns.example/query"]}}),
    )?;
    client.patch_config(patch.clone()).await?;
    assert_eq!(
        serde_json::from_slice::<Value>(&server.next().await?.body)?,
        Value::Object(patch)
    );
    client.reload_config().await?;
    let request = server.next().await?;
    assert_eq!(request.uri.query(), Some("force=true"));
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body)?,
        json!({"path":"","payload":""})
    );
    assert!(client.select_proxy("..", "DIRECT").await.is_err());
    assert!(client.close_connection("").await.is_err());
    assert!(server.received.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn all_maintenance_routes_send_exactly_one_post() -> TestResult {
    let mut server = Server::start(|_| async { empty() }).await?;
    let client = server.client()?;
    for (action, path) in [
        (MaintenanceAction::FlushDns, "/cache/dns/flush"),
        (MaintenanceAction::FlushFakeIp, "/cache/fakeip/flush"),
        (MaintenanceAction::UpdateGeo, "/configs/geo"),
        (MaintenanceAction::Restart, "/restart"),
        (MaintenanceAction::UpgradeCore, "/upgrade"),
        (MaintenanceAction::UpgradeHostedUi, "/upgrade/ui"),
    ] {
        client.maintenance(action).await?;
        let request = server.next().await?;
        assert_eq!(request.method, "POST");
        assert_eq!(request.uri.path(), path);
        assert!(request.body.is_empty());
    }
    assert!(server.received.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn rule_arrays_and_sparse_numeric_objects_keep_correct_indices() -> TestResult {
    for payload in [
        json!({"rules":[{"type":"MATCH","proxy":"DIRECT","size":-1}]}),
        json!({"rules":{"12":{"type":"MATCH","proxy":"DIRECT"},"2":{"type":"DOMAIN","proxy":"REJECT"}}}),
    ] {
        let server = Server::start(move |_| {
            let payload = payload.clone();
            async move { response_json(payload) }
        })
        .await?;
        let rules = server.client()?.rules().await?;
        let indices: Vec<_> = rules.into_iter().filter_map(|r| r.index).collect();
        assert!(indices == vec![0] || indices == vec![2, 12]);
    }
    let server = Server::start(|_| async {
        response_json(json!({"rules":{"bad":{"type":"MATCH","proxy":"DIRECT"}}}))
    })
    .await?;
    assert!(matches!(server.client()?.rules().await, Err(e) if e.kind == ErrorKind::Decode));
    Ok(())
}

#[tokio::test]
async fn errors_are_classified_redacted_and_mutations_are_not_retried() -> TestResult {
    for (status, kind) in [
        (401, ErrorKind::Unauthorized),
        (403, ErrorKind::Unauthorized),
        (404, ErrorKind::Unsupported),
        (500, ErrorKind::Http),
        (302, ErrorKind::Http),
    ] {
        let code = StatusCode::from_u16(status)?;
        let mut server = Server::start(move |_| async move {
            (
                code,
                axum::Json(json!({"message":"test-secret was rejected"})),
            )
                .into_response()
        })
        .await?;
        let result = server.client()?.select_proxy("group", "node").await;
        match result {
            Err(error) => {
                assert_eq!(error.kind, kind);
                assert_eq!(error.status, Some(status));
                assert!(!format!("{error:?} {error}").contains("test-secret"));
            }
            Ok(()) => return Err(std::io::Error::other("expected HTTP failure").into()),
        }
        server.next().await?;
        assert!(server.received.try_recv().is_err());
    }
    Ok(())
}

#[tokio::test]
async fn invalid_and_oversized_json_never_looks_like_a_valid_snapshot() -> TestResult {
    for body in ["not JSON test-secret", "{}", "{\"version\":5}", ""] {
        let server = Server::start(move |_| async move { body.into_response() }).await?;
        assert!(
            matches!(server.client()?.version().await, Err(e) if e.kind == ErrorKind::Decode && !format!("{e:?}").contains("test-secret"))
        );
    }
    let server =
        Server::start(|_| async { response_json(json!({"version":"x".repeat(1000)})) }).await?;
    let client = CoreClient::with_options(
        Endpoint::new("x", "", &server.url, "")?,
        ClientOptions {
            max_response_bytes: 32,
            ..Default::default()
        },
    )?;
    assert!(matches!(client.version().await, Err(e) if e.kind == ErrorKind::ResponseTooLarge));
    Ok(())
}

#[tokio::test]
async fn timeout_covers_response_and_refused_connections_are_transport_errors() -> TestResult {
    let server = Server::start(|_| async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        empty()
    })
    .await?;
    let client = CoreClient::with_options(
        Endpoint::new("x", "", &server.url, "secret")?,
        ClientOptions {
            request_timeout: Duration::from_millis(20),
            ..Default::default()
        },
    )?;
    assert!(matches!(client.version().await, Err(e) if e.kind == ErrorKind::Timeout));
    let endpoint = Endpoint::new("closed", "", &server.url, "secret")?;
    drop(server);
    tokio::task::yield_now().await;
    assert!(
        matches!(CoreClient::new(endpoint)?.version().await, Err(e) if e.kind == ErrorKind::Transport)
    );
    Ok(())
}

#[tokio::test]
async fn remote_config_fetch_preserves_query_and_does_not_send_core_credentials() -> TestResult {
    let mut source =
        Server::start(|_| async { "mode: rule\nrules: [MATCH,DIRECT]".into_response() }).await?;
    let mut core = Server::start(|_| async { empty() }).await?;
    core.client()?
        .load_config_url(&format!(
            "{}/profile.yaml?token=subscription-token",
            source.url
        ))
        .await?;
    let request = source.next().await?;
    assert_eq!(request.uri.path(), "/profile.yaml");
    assert_eq!(request.uri.query(), Some("token=subscription-token"));
    assert!(request.headers.get("authorization").is_none());
    let request = core.next().await?;
    assert_eq!(request.uri.path(), "/configs");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body)?,
        json!({"path":"","payload":"mode: rule\nrules: [MATCH,DIRECT]"})
    );
    Ok(())
}

#[tokio::test]
async fn proxy_scopes_and_probe_query_survive_url_encoding() -> TestResult {
    let mut server = Server::start(|r| async move {
        if r.uri.path().starts_with("/group/") {
            response_json(json!({"n":19}))
        } else {
            response_json(json!({"delay":13}))
        }
    })
    .await?;
    let client = server.client()?;
    let mut probe = Probe {
        node: "n/a".into(),
        provider: Some("p/#".into()),
        url: "https://example.com/204?q=a&b=2".into(),
        timeout_ms: 5000,
    };
    assert_eq!(client.test_proxy(&probe).await?.delay, 13);
    let r = server.next().await?;
    assert_eq!(r.uri.path(), "/providers/proxies/p%2F%23/n%2Fa/healthcheck");
    let query: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(r.uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    assert_eq!(query.get("url"), Some(&probe.url));
    assert_eq!(query.get("timeout").map(String::as_str), Some("5000"));
    probe.provider = None;
    client.test_proxy(&probe).await?;
    assert_eq!(server.next().await?.uri.path(), "/proxies/n%2Fa/delay");
    assert_eq!(
        client
            .test_group("g", &probe.url, probe.timeout_ms)
            .await?
            .get("n"),
        Some(&19)
    );
    Ok(())
}

#[tokio::test]
async fn redirects_are_scoped_to_config_downloads() -> TestResult {
    let mut destination = Server::start(|_| async { "mode: rule".into_response() }).await?;
    let location = format!("{}/config?token=other", destination.url);
    let mut redirect = Server::start(move |_| {
        let location = location.clone();
        async move { (StatusCode::FOUND, [("location", location)], "").into_response() }
    })
    .await?;
    let mut core = Server::start(|_| async { empty() }).await?;
    core.client()?
        .load_config_url(&format!("{}/download?token=source", redirect.url))
        .await?;
    assert!(
        redirect
            .next()
            .await?
            .headers
            .get("authorization")
            .is_none()
    );
    assert!(
        destination
            .next()
            .await?
            .headers
            .get("authorization")
            .is_none()
    );
    assert_eq!(core.next().await?.uri.path(), "/configs");
    assert!(matches!(redirect.client()?.version().await, Err(e) if e.status == Some(302)));
    assert!(destination.received.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn failed_download_never_applies_config_or_exposes_subscription_token() -> TestResult {
    let source = Server::start(|_| async {
        (
            StatusCode::FORBIDDEN,
            axum::Json(json!({"message":"subscription-secret rejected"})),
        )
            .into_response()
    })
    .await?;
    let mut core = Server::start(|_| async { empty() }).await?;
    let result = core
        .client()?
        .load_config_url(&format!(
            "{}/download?token=subscription-secret",
            source.url
        ))
        .await;
    assert!(
        matches!(result, Err(e) if e.kind == ErrorKind::Unauthorized && !format!("{e:?}").contains("subscription-secret"))
    );
    assert!(core.received.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn chunked_responses_obey_size_and_total_timeout_limits() -> TestResult {
    use axum::body::Body;
    let server = Server::start(|_| async {
        let stream = futures_util::stream::iter([
            Ok::<_, std::io::Error>(vec![b'x'; 20]),
            Ok(vec![b'y'; 20]),
        ]);
        Body::from_stream(stream).into_response()
    })
    .await?;
    let client = CoreClient::with_options(
        Endpoint::new("x", "", &server.url, "")?,
        ClientOptions {
            max_response_bytes: 32,
            ..Default::default()
        },
    )?;
    assert!(matches!(client.version().await, Err(e) if e.kind == ErrorKind::ResponseTooLarge));
    let server = Server::start(|_| async {
        let stream = futures_util::stream::once(async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok::<_, std::io::Error>(b"{}".to_vec())
        });
        Body::from_stream(stream).into_response()
    })
    .await?;
    let client = CoreClient::with_options(
        Endpoint::new("x", "", &server.url, "")?,
        ClientOptions {
            request_timeout: Duration::from_millis(20),
            ..Default::default()
        },
    )?;
    assert!(matches!(client.version().await, Err(e) if e.kind == ErrorKind::Timeout));
    Ok(())
}

#[tokio::test]
async fn batch_checks_concurrency_and_cancellation() -> TestResult {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let a = active.clone();
    let m = maximum.clone();
    let server = Server::start(move |r| {
        let a = a.clone();
        let m = m.clone();
        async move {
            let current = a.fetch_add(1, Ordering::SeqCst) + 1;
            m.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            a.fetch_sub(1, Ordering::SeqCst);
            if r.uri.path().contains("bad") {
                StatusCode::GATEWAY_TIMEOUT.into_response()
            } else {
                response_json(json!({"delay":1}))
            }
        }
    })
    .await?;
    let client = server.client()?;
    let probes: Vec<_> = (0..7)
        .map(|i| Probe {
            node: if i == 0 { "bad".into() } else { i.to_string() },
            provider: None,
            url: "https://example.com".into(),
            timeout_ms: 100,
        })
        .collect();
    let cancel = CancellationToken::new();
    let results = client.test_batch(probes.clone(), 2, &cancel).await?;
    assert_eq!(results.len(), 7);
    assert_eq!(results.iter().filter(|r| r.result.is_err()).count(), 1);
    assert_eq!(maximum.load(Ordering::SeqCst), 2);
    cancel.cancel();
    assert!(
        matches!(client.test_batch(probes,2,&cancel).await,Err(e) if e.kind==ErrorKind::Cancelled)
    );
    Ok(())
}
