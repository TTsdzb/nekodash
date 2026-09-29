use super::Event;
use base64::Engine;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    runtime::Handle,
    sync::{Semaphore, mpsc},
};

const MAX_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 128;
const RETRY: Duration = Duration::from_secs(60);

pub(super) struct Download {
    bytes: Vec<u8>,
    svg: bool,
}
struct Entry {
    image: slint::Image,
    pending: bool,
    failed: bool,
    touched: Instant,
}
pub(super) struct GroupIcons {
    client: reqwest::Client,
    permits: Arc<Semaphore>,
    cache: RefCell<BTreeMap<String, Entry>>,
}
impl GroupIcons {
    pub fn new() -> Result<Self, reqwest::Error> {
        // External artwork follows the OS/environment proxy configuration.
        // The client has no core credentials or controller-specific headers.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        Ok(Self {
            client,
            permits: Arc::new(Semaphore::new(4)),
            cache: RefCell::new(BTreeMap::new()),
        })
    }
    pub fn retain_sources<'a>(&self, sources: impl Iterator<Item = &'a str>) {
        let active: BTreeSet<_> = sources.collect();
        self.cache
            .borrow_mut()
            .retain(|source, _| active.contains(source.as_str()));
    }
    pub fn get(&self, source: &str, runtime: &Handle, tx: &mpsc::Sender<Event>) -> slint::Image {
        if source.is_empty() || source.len() > MAX_BYTES * 2 {
            return slint::Image::default();
        }
        let mut cache = self.cache.borrow_mut();
        if let Some(entry) = cache.get_mut(source) {
            if entry.pending || (entry.failed && entry.touched.elapsed() < RETRY) {
                return entry.image.clone();
            }
            if !entry.failed {
                entry.touched = Instant::now();
                return entry.image.clone();
            }
        } else if cache.len() >= MAX_ENTRIES {
            return slint::Image::default();
        }
        cache.insert(
            source.into(),
            Entry {
                image: slint::Image::default(),
                pending: true,
                failed: false,
                touched: Instant::now(),
            },
        );
        let source = source.to_owned();
        let client = self.client.clone();
        let permits = self.permits.clone();
        let tx = tx.clone();
        runtime.spawn(async move {
            let result = match permits.acquire_owned().await {
                Ok(_permit) => download(&client, &source).await,
                Err(_) => Err("icon loader closed".into()),
            };
            let _ = tx.send(Event::Icon(source, result)).await;
        });
        slint::Image::default()
    }
    pub fn complete(&self, source: &str, result: Result<Download, String>) {
        let image = result.ok().and_then(|data| {
            slint::Image::load_from_data(&data.bytes, if data.svg { Some("svg") } else { None })
                .ok()
        });
        if let Some(entry) = self.cache.borrow_mut().get_mut(source) {
            entry.failed = image.is_none();
            entry.image = image.unwrap_or_default();
            entry.pending = false;
            entry.touched = Instant::now();
        }
    }
}
fn inline(source: &str) -> Result<Download, String> {
    let (header, payload) = source.split_once(',').ok_or("invalid image data URL")?;
    if !header.starts_with("data:image/") {
        return Err("expected image data URL".into());
    }
    let bytes = percent_encoding::percent_decode_str(payload).collect::<Vec<u8>>();
    let bytes = if header.ends_with(";base64") {
        base64::engine::general_purpose::STANDARD
            .decode(bytes)
            .map_err(|_| "invalid base64 image")?
    } else {
        bytes
    };
    if bytes.len() > MAX_BYTES {
        return Err("image exceeds size limit".into());
    }
    Ok(Download {
        bytes,
        svg: header.starts_with("data:image/svg+xml"),
    })
}
async fn download(client: &reqwest::Client, source: &str) -> Result<Download, String> {
    if source.starts_with("data:") {
        return inline(source);
    }
    let url = reqwest::Url::parse(source).map_err(|_| "invalid icon URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("unsupported icon URL scheme".into());
    }
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| "icon request failed")?
        .error_for_status()
        .map_err(|_| "icon HTTP error")?;
    if response
        .content_length()
        .is_some_and(|v| v > MAX_BYTES as u64)
    {
        return Err("image exceeds size limit".into());
    }
    let svg = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("image/svg+xml"));
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "icon body failed")? {
        if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
            return Err("image exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Download { bytes, svg })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn svg_data_urls_support_escaped_and_base64_content() -> Result<(), String> {
        let svg = b"<svg xmlns='http://www.w3.org/2000/svg'/>";
        let escaped = inline("data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'/%3E")?;
        assert!(escaped.svg);
        assert_eq!(escaped.bytes, svg);
        let data = inline(&format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(svg)
        ))?;
        assert_eq!(data.bytes, svg);
        assert!(inline("data:text/plain,hello").is_err());
        assert!(inline("data:image/png;base64,!").is_err());
        assert!(inline(&format!("data:image/png,{}", "x".repeat(MAX_BYTES + 1))).is_err());
        Ok(())
    }
}
