//! Read-only snapshot/stream inspector. Credentials are read from the environment.
use nekodash_core::{
    ClientOptions, Endpoint, RecoveryOptions, Session, StreamEvent, StreamKind, StreamOptions,
};
use std::{error::Error, time::Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = std::env::var("NEKODASH_ENDPOINT")?;
    let secret = std::env::var("NEKODASH_SECRET").unwrap_or_default();
    let mut session = Session::default();
    session.switch(
        Endpoint::new("inspect", "inspect", &url, secret)?,
        ClientOptions::default(),
    )?;
    let recovery = session.recover(RecoveryOptions::default())?;
    let context = recovery.context().clone();
    let event = recovery.run().await;
    if !session.accepts(&event.token) {
        return Ok(());
    }
    let snapshot = event.result?.snapshot?;
    println!(
        "Core: {}\nProxies: {}\nRules: {}",
        snapshot.version.version,
        snapshot.proxies?.proxies.len(),
        snapshot.rules?.len()
    );
    let mut subscription = context
        .subscribe(
            &tokio::runtime::Handle::current(),
            StreamKind::Traffic,
            StreamOptions::default(),
        )
        .result?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = subscription.recv().await;
            // In a GUI, make this check after the event reaches the UI thread as well.
            if !session.accepts(&event.token) {
                return Ok(());
            }
            match event.result? {
                StreamEvent::Data(data) => {
                    println!("{data:?}");
                    return Ok::<(), nekodash_core::Error>(());
                }
                StreamEvent::Error(error) => return Err(error),
                _ => {}
            }
        }
    })
    .await??;
    session.disconnect();
    Ok(())
}
