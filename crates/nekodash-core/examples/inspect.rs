//! Read-only snapshot/stream inspector. Credentials are read from the environment.
use nekodash_core::{
    CancellationToken, CoreClient, Endpoint, StreamEvent, StreamKind, StreamOptions,
};
use std::{error::Error, time::Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let url = std::env::var("NEKODASH_ENDPOINT")?;
    let secret = std::env::var("NEKODASH_SECRET").unwrap_or_default();
    let client = CoreClient::new(Endpoint::new("inspect", "inspect", &url, secret)?)?;
    let (version, proxies, rules) =
        tokio::try_join!(client.version(), client.proxies(), client.rules())?;
    println!(
        "Core: {}\nProxies: {}\nRules: {}",
        version.version,
        proxies.proxies.len(),
        rules.len()
    );
    let cancel = CancellationToken::new();
    let mut subscription = client.subscribe(
        &tokio::runtime::Handle::current(),
        StreamKind::Traffic,
        StreamOptions::default(),
        &cancel,
    )?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match subscription.recv().await? {
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
    cancel.cancel();
    Ok(())
}
