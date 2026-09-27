mod common;
use common::{Server, TestResult, json as response_json};
use nekodash_core::{
    BatchEvent, CancellationToken, ClientOptions, Endpoint, ErrorKind, Probe, Session,
};
use serde_json::json;
use std::time::Duration;

fn probes() -> Vec<Probe> {
    ["slow", "fast", "queued"]
        .into_iter()
        .map(|node| Probe {
            node: node.into(),
            provider: None,
            url: "https://example.test".into(),
            timeout_ms: 100,
        })
        .collect()
}

async fn server() -> TestResult<Server> {
    Server::start(|request| async move {
        if !request.uri.path().contains("fast") {
            std::future::pending::<()>().await;
        }
        response_json(json!({"delay":42}))
    })
    .await
}

#[tokio::test]
async fn emits_results_before_slow_nodes_finish_and_cancellation_keeps_progress() -> TestResult {
    let server = server().await?;
    let mut session = Session::default();
    let context = session.switch(
        Endpoint::new("batch", "", &server.url, "")?,
        ClientOptions::default(),
    )?;
    let mut batch = context.batch_tests(probes(), 2).result?;
    let event = tokio::time::timeout(Duration::from_secs(2), batch.recv())
        .await?
        .ok_or_else(|| std::io::Error::other("missing batch event"))?;
    assert!(session.accepts(&event.token));
    match event.result? {
        BatchEvent::Completed {
            completed,
            total,
            result,
        } => {
            assert_eq!((completed, total, result.index), (1, 3, 1));
            assert_eq!(result.probe.node, "fast");
            assert_eq!(result.result?.delay, 42);
        }
        _ => return Err(std::io::Error::other("batch finished before fast result").into()),
    }
    // Switching the same endpoint invalidates both buffered completions and the final cancellation.
    session.reconnect()?;
    assert!(!session.accepts(&event.token));
    let event = batch
        .recv()
        .await
        .ok_or_else(|| std::io::Error::other("missing batch cancellation"))?;
    assert!(!session.accepts(&event.token));
    assert!(matches!(
        event.result?,
        BatchEvent::Finished {
            completed: 1,
            total: 3,
            cancelled: true
        }
    ));
    assert!(batch.is_finished());
    assert!(batch.recv().await.is_none());
    Ok(())
}

#[tokio::test]
async fn collected_batch_returns_completed_results_when_cancelled() -> TestResult {
    let mut server = server().await?;
    let client = server.client()?;
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move { client.test_batch(probes(), 2, &task_cancel).await });
    // The queued request starts only after the collector has received the fast result.
    loop {
        if server.next().await?.uri.path().contains("queued") {
            break;
        }
    }
    cancel.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(1), task).await???;
    assert!(outcome.cancelled);
    assert_eq!(outcome.total, 3);
    assert_eq!(outcome.results.len(), 1);
    let result = outcome
        .results
        .first()
        .ok_or_else(|| std::io::Error::other("lost completed result"))?;
    assert_eq!(result.index, 1);
    assert_eq!(result.result.as_ref().map(|d| d.delay).ok(), Some(42));
    Ok(())
}

#[tokio::test]
async fn empty_invalid_and_duplicate_batches_have_unambiguous_outcomes() -> TestResult {
    let mut server = Server::start(|_| async { response_json(json!({"delay":1})) }).await?;
    let client = server.client()?;
    let cancel = CancellationToken::new();
    let mut empty = client.batch_tests(Vec::new(), 1, &cancel)?;
    assert!(matches!(
        empty.recv().await,
        Some(BatchEvent::Finished {
            completed: 0,
            total: 0,
            cancelled: false
        })
    ));
    assert!(empty.recv().await.is_none());
    assert!(server.received.try_recv().is_err());
    let mut session = Session::default();
    let context = session.switch(
        Endpoint::new("batch", "", &server.url, "")?,
        ClientOptions::default(),
    )?;
    let invalid = context.batch_tests(probes(), 0);
    assert!(session.accepts(&invalid.token));
    assert!(matches!(invalid.result, Err(e) if e.kind == ErrorKind::InvalidInput));
    let probe = Probe {
        node: "same".into(),
        provider: Some("provider".into()),
        url: "https://example.test".into(),
        timeout_ms: 100,
    };
    let results = client
        .test_batch(vec![probe.clone(), probe], 2, &cancel)
        .await?;
    let mut indices: Vec<_> = results.results.iter().map(|r| r.index).collect();
    indices.sort();
    assert_eq!(indices, vec![0, 1]);
    assert!(!results.cancelled);
    Ok(())
}
