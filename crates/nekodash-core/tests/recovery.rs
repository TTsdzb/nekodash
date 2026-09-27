mod common;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use common::{Server, TestResult, empty, json as response_json};
use nekodash_core::{
    ClientOptions, CommandOutcome, Endpoint, ErrorKind, MaintenanceAction, RecoveryOptions, Session,
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

fn options() -> RecoveryOptions {
    RecoveryOptions {
        timeout: Duration::from_secs(2),
        retry_interval: Duration::from_millis(10),
        stable_for: Duration::ZERO,
        command_timeout: Duration::from_millis(50),
    }
}

fn snapshot(path: &str) -> Response {
    response_json(match path {
        "/version" => json!({"version":"new","meta":true}),
        "/configs" => json!({"mode":"rule"}),
        "/proxies" => json!({"proxies":{}}),
        "/providers/proxies" | "/providers/rules" => json!({"providers":{}}),
        "/rules" => json!({"rules":[]}),
        "/connections" => json!({"connections":[],"uploadTotal":0,"downloadTotal":0}),
        _ => serde_json::Value::Null,
    })
}

fn session(server: &Server) -> TestResult<Session> {
    let mut session = Session::default();
    session.switch(
        Endpoint::new("core", "", &server.url, "")?,
        ClientOptions::default(),
    )?;
    Ok(session)
}

#[tokio::test]
async fn recovery_retries_availability_and_transient_snapshot_failures() -> TestResult {
    let versions = Arc::new(AtomicUsize::new(0));
    let proxies = Arc::new(AtomicUsize::new(0));
    let (v, p) = (versions.clone(), proxies.clone());
    let server = Server::start(move |request| {
        let (v, p) = (v.clone(), p.clone());
        async move {
            let path = request.uri.path();
            if (path == "/version" && v.fetch_add(1, Ordering::SeqCst) < 2)
                || (path == "/proxies" && p.fetch_add(1, Ordering::SeqCst) == 0)
            {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            snapshot(path)
        }
    })
    .await?;
    let mut session = session(&server)?;
    let old = session
        .current()
        .cloned()
        .ok_or_else(|| std::io::Error::other("missing session"))?;
    let operation = session.recover(options())?;
    assert!(old.cancel.is_cancelled());
    let event = operation.run().await;
    assert!(session.accepts(&event.token));
    assert!(!session.accepts(&old.token));
    let report = event.result?;
    assert!(report.command.is_none());
    let data = report.snapshot?;
    assert!(data.is_complete());
    assert_eq!(data.version.version, "new");
    assert!(versions.load(Ordering::SeqCst) >= 4);
    assert_eq!(proxies.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn successful_response_before_restart_does_not_bypass_stability_checks() -> TestResult {
    let versions = Arc::new(AtomicUsize::new(0));
    let v = versions.clone();
    let server = Server::start(move |request| {
        let v = v.clone();
        async move {
            if request.uri.path() == "/version" {
                match v.fetch_add(1, Ordering::SeqCst) {
                    0 => return response_json(json!({"version":"old"})),
                    1 => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
                    _ => {}
                }
            }
            snapshot(request.uri.path())
        }
    })
    .await?;
    let mut session = session(&server)?;
    let event = session
        .recover(RecoveryOptions {
            stable_for: Duration::from_millis(25),
            ..options()
        })?
        .run()
        .await;
    assert_eq!(event.result?.snapshot?.version.version, "new");
    assert!(versions.load(Ordering::SeqCst) >= 4);
    Ok(())
}

#[tokio::test]
async fn timed_out_mutations_keep_uncertain_outcome_and_refresh_without_resending() -> TestResult {
    for action in [
        MaintenanceAction::Restart,
        MaintenanceAction::UpgradeCore,
        MaintenanceAction::UpdateGeo,
    ] {
        let posts = Arc::new(AtomicUsize::new(0));
        let applied = Arc::new(AtomicBool::new(false));
        let (p, a) = (posts.clone(), applied.clone());
        let server = Server::start(move |request| {
            let (p, a) = (p.clone(), a.clone());
            async move {
                if request.method == "POST" {
                    p.fetch_add(1, Ordering::SeqCst);
                    a.store(true, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    return empty();
                }
                if request.uri.path() == "/configs" {
                    return response_json(json!({"applied":a.load(Ordering::SeqCst)}));
                }
                snapshot(request.uri.path())
            }
        })
        .await?;
        let mut session = session(&server)?;
        let event = session.maintain(action, options())?.run().await;
        assert!(session.accepts(&event.token));
        let report = event.result?;
        assert!(
            matches!(report.command, Some(CommandOutcome::Indeterminate(error)) if error.kind == ErrorKind::Timeout)
        );
        assert_eq!(
            report.snapshot?.config?.0.get("applied"),
            Some(&json!(true))
        );
        assert_eq!(posts.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[tokio::test]
async fn acknowledged_and_reported_failure_are_separate_from_recovery() -> TestResult {
    for status in [
        StatusCode::NO_CONTENT,
        StatusCode::BAD_REQUEST,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let posts = Arc::new(AtomicUsize::new(0));
        let p = posts.clone();
        let server = Server::start(move |request| {
            let p = p.clone();
            async move {
                if request.method == "POST" {
                    p.fetch_add(1, Ordering::SeqCst);
                    return status.into_response();
                }
                snapshot(request.uri.path())
            }
        })
        .await?;
        let mut session = session(&server)?;
        let report = session
            .maintain(MaintenanceAction::FlushDns, options())?
            .run()
            .await
            .result?;
        if status.is_success() {
            assert!(matches!(report.command, Some(CommandOutcome::Acknowledged)));
        } else {
            assert!(
                matches!(report.command, Some(CommandOutcome::ReportedFailure(error)) if error.status == Some(status.as_u16()))
            );
        }
        assert!(report.snapshot?.is_complete());
        assert_eq!(posts.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[tokio::test]
async fn unsupported_resources_remain_visible_as_individual_snapshot_errors() -> TestResult {
    let server = Server::start(|request| async move {
        if request.uri.path() == "/providers/rules" {
            return StatusCode::NOT_FOUND.into_response();
        }
        snapshot(request.uri.path())
    })
    .await?;
    let mut session = session(&server)?;
    let data = session.recover(options())?.run().await.result?.snapshot?;
    assert!(!data.is_complete());
    assert!(data.proxies.is_ok());
    assert!(matches!(data.rule_providers, Err(error) if error.kind == ErrorKind::Unsupported));
    Ok(())
}

#[tokio::test]
async fn authentication_and_invalid_version_stop_recovery_without_retry() -> TestResult {
    for invalid_json in [false, true] {
        let visits = Arc::new(AtomicUsize::new(0));
        let v = visits.clone();
        let server = Server::start(move |_| {
            let v = v.clone();
            async move {
                v.fetch_add(1, Ordering::SeqCst);
                if invalid_json {
                    response_json(json!({"wrong":"schema"}))
                } else {
                    StatusCode::UNAUTHORIZED.into_response()
                }
            }
        })
        .await?;
        let mut session = session(&server)?;
        let report = session.recover(options())?.run().await.result?;
        let kind = if invalid_json {
            ErrorKind::Decode
        } else {
            ErrorKind::Unauthorized
        };
        assert!(matches!(report.snapshot, Err(error) if error.kind == kind));
        assert_eq!(visits.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[tokio::test]
async fn recovery_deadline_bounds_an_unresponsive_request() -> TestResult {
    let server = Server::start(|_| async { std::future::pending::<Response>().await }).await?;
    let mut session = session(&server)?;
    let operation = session.recover(RecoveryOptions {
        timeout: Duration::from_millis(50),
        ..options()
    })?;
    let event = tokio::time::timeout(Duration::from_secs(1), operation.run()).await?;
    assert!(matches!(event.result?.snapshot, Err(error) if error.kind == ErrorKind::Timeout));
    assert!(session.accepts(&event.token));
    Ok(())
}

#[tokio::test]
async fn cancellation_before_and_during_mutation_keeps_origin_and_delivery_uncertainty()
-> TestResult {
    let mut server = Server::start(|_| async { std::future::pending::<Response>().await }).await?;
    let mut session = session(&server)?;
    let operation = session.maintain(MaintenanceAction::Restart, options())?;
    session.reconnect()?;
    let event = operation.run().await;
    assert!(matches!(event.result, Err(error) if error.kind == ErrorKind::Cancelled));
    assert!(!session.accepts(&event.token));
    assert!(server.received.try_recv().is_err());

    let operation = session.maintain(MaintenanceAction::Restart, options())?;
    let task = tokio::spawn(operation.run());
    assert_eq!(server.next().await?.method, "POST");
    session.reconnect()?;
    let event = tokio::time::timeout(Duration::from_secs(1), task).await??;
    assert!(!session.accepts(&event.token));
    let report = event.result?;
    assert!(
        matches!(report.command, Some(CommandOutcome::Indeterminate(error)) if error.kind == ErrorKind::Cancelled)
    );
    assert!(matches!(report.snapshot, Err(error) if error.kind == ErrorKind::Cancelled));
    assert!(server.received.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn invalid_options_preserve_active_session_and_switch_cancels_recovery() -> TestResult {
    let mut server = Server::start(|_| async { std::future::pending::<Response>().await }).await?;
    let mut session = session(&server)?;
    let old = session
        .current()
        .cloned()
        .ok_or_else(|| std::io::Error::other("missing session"))?;
    assert!(
        session
            .recover(RecoveryOptions {
                timeout: Duration::ZERO,
                ..options()
            })
            .is_err()
    );
    assert!(session.accepts(&old.token));
    let operation = session.recover(options())?;
    let task = tokio::spawn(operation.run());
    server.next().await?;
    session.disconnect();
    let event = tokio::time::timeout(Duration::from_secs(1), task).await??;
    assert!(!session.accepts(&event.token));
    assert!(matches!(event.result?.snapshot, Err(error) if error.kind == ErrorKind::Cancelled));
    Ok(())
}
