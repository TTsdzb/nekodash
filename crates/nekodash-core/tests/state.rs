mod common;
use axum::response::IntoResponse;
use common::{Server, TestResult, json as response_json};
use nekodash_core::{ClientOptions, Endpoint, EndpointStore, ErrorKind, Session};
use serde_json::json;

#[test]
fn endpoints_validate_urls_headers_and_redact_debug() -> TestResult {
    for url in [
        "file:///tmp/core",
        "ws://host",
        "http://user:password@host",
        "http://host?token=secret",
        "http://host/#frag",
        "not a url",
    ] {
        assert!(Endpoint::new("x", "", url, "").is_err(), "accepted {url}");
    }
    assert!(Endpoint::new("", "", "http://localhost", "").is_err());
    assert!(Endpoint::new("x", "", "http://localhost", "\r\ninjected").is_err());
    let endpoint = Endpoint::new("x", "router", "https://[::1]:9090/prefix", "private-secret")?;
    assert_eq!(endpoint.url().as_str(), "https://[::1]:9090/prefix/");
    assert!(!format!("{endpoint:?}").contains("private-secret"));
    Ok(())
}

#[test]
fn storage_roundtrip_replacement_removal_and_validation() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("subdir/endpoints.json");
    let mut store = EndpointStore::load(&path)?;
    assert!(store.endpoints().is_empty());
    store.upsert(Endpoint::new("a", "one", "http://localhost:1", "secret")?)?;
    store.upsert(Endpoint::new("b", "two", "https://localhost:2", "")?)?;
    store.select("a")?;
    store.save(&path)?;
    let mut loaded = EndpointStore::load(&path)?;
    assert_eq!(loaded.selected().map(Endpoint::label), Some("one"));
    loaded.upsert(Endpoint::new(
        "a",
        "new label",
        "http://localhost:3",
        "new secret",
    )?)?;
    loaded.save(&path)?;
    let mut loaded = EndpointStore::load(&path)?;
    assert_eq!(loaded.endpoints().len(), 2);
    assert_eq!(loaded.selected().map(Endpoint::label), Some("new label"));
    assert!(loaded.select("missing").is_err());
    loaded.remove("a");
    assert!(loaded.selected().is_none());
    assert!(!format!("{loaded:?}").contains("new secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777,
            0o600
        );
    }
    let original = std::fs::read(&path)?;
    let mut corrupt: serde_json::Value = serde_json::from_slice(&original)?;
    corrupt["schema_version"] = json!(999);
    std::fs::write(&path, serde_json::to_vec(&corrupt)?)?;
    assert!(EndpointStore::load(&path).is_err());
    assert_eq!(std::fs::read(&path)?, serde_json::to_vec(&corrupt)?);
    corrupt["schema_version"] = json!(1);
    corrupt["endpoints"][0]["url"] = json!("file:///secret");
    std::fs::write(&path, serde_json::to_vec(&corrupt)?)?;
    assert!(EndpointStore::load(&path).is_err());
    Ok(())
}

#[tokio::test]
async fn switching_cancels_inflight_work_and_rejects_old_callbacks() -> TestResult {
    let mut slow = Server::start(|_| async {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        response_json(json!({"version":"old"}))
    })
    .await?;
    let new = Server::start(|_| async { response_json(json!({"version":"new"})) }).await?;
    let mut session = Session::default();
    let old = session.switch(
        Endpoint::new("same-id", "", &slow.url, "")?,
        ClientOptions::default(),
    )?;
    let old_token = old.token.clone();
    let work = tokio::spawn(async move { old.run(old.client.version()).await });
    slow.next().await?;
    let current = session.switch(
        Endpoint::new("same-id", "", &new.url, "")?,
        ClientOptions::default(),
    )?;
    let event = work.await?;
    assert_eq!(event.token, old_token);
    assert!(matches!(event.result,Err(e) if e.kind==ErrorKind::Cancelled));
    assert!(!session.accepts(&old_token));
    let event = current.run(current.client.version()).await;
    assert_eq!(event.result?.version, "new");
    assert!(session.accepts(&event.token));
    session.disconnect();
    assert!(!session.accepts(&event.token));
    assert!(current.cancel.is_cancelled());
    Ok(())
}

#[tokio::test]
async fn errors_queued_before_reconnect_keep_their_original_generation() -> TestResult {
    let server =
        Server::start(|_| async { axum::http::StatusCode::UNAUTHORIZED.into_response() }).await?;
    let mut session = Session::default();
    let old = session.switch(
        Endpoint::new("same", "", &server.url, "")?,
        ClientOptions::default(),
    )?;
    let error = old.run(old.client.version()).await;
    assert!(matches!(error.result, Err(e) if e.kind == ErrorKind::Unauthorized));
    assert!(session.accepts(&error.token));
    let current = session.reconnect()?;
    assert_eq!(current.token.endpoint_id, old.token.endpoint_id);
    assert_ne!(current.token.generation, old.token.generation);
    assert!(old.cancel.is_cancelled());
    assert!(!session.accepts(&error.token));
    assert!(session.accepts(&current.token));
    Ok(())
}
