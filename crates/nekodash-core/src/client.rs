use crate::{Endpoint, Error, ErrorKind, Result, models::*};
use futures_util::StreamExt;
use reqwest::{Method, header::AUTHORIZATION};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

#[derive(Clone, Debug)]
pub struct ClientOptions {
    pub request_timeout: Duration,
    pub connect_timeout: Duration,
    pub max_response_bytes: usize,
    /// Additional trust anchors for private controllers, in PEM format.
    pub additional_ca_pem: Vec<String>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(5),
            max_response_bytes: 16 * 1024 * 1024,
            additional_ca_pem: Vec::new(),
        }
    }
}

/// A client remains bound to its original endpoint, including in-flight requests.
#[derive(Clone)]
pub struct CoreClient {
    endpoint: Endpoint,
    http: reqwest::Client,
    options: ClientOptions,
    pub(crate) tls_config: Arc<rustls::ClientConfig>,
}

#[derive(Clone, Copy, Debug)]
pub enum MaintenanceAction {
    FlushDns,
    FlushFakeIp,
    UpdateGeo,
    Restart,
    UpgradeCore,
    UpgradeHostedUi,
}

impl MaintenanceAction {
    fn path(self) -> &'static [&'static str] {
        match self {
            Self::FlushDns => &["cache", "dns", "flush"],
            Self::FlushFakeIp => &["cache", "fakeip", "flush"],
            Self::UpdateGeo => &["configs", "geo"],
            Self::Restart => &["restart"],
            Self::UpgradeCore => &["upgrade"],
            Self::UpgradeHostedUi => &["upgrade", "ui"],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub node: String,
    pub provider: Option<String>,
    pub url: String,
    pub timeout_ms: u32,
}

impl CoreClient {
    pub fn new(endpoint: Endpoint) -> Result<Self> {
        Self::with_options(endpoint, ClientOptions::default())
    }

    pub fn with_options(endpoint: Endpoint, options: ClientOptions) -> Result<Self> {
        endpoint.validate()?;
        if options.request_timeout.is_zero()
            || options.connect_timeout.is_zero()
            || options.max_response_bytes == 0
        {
            return Err(Error::invalid(
                "timeouts and response limit must be positive",
            ));
        }
        let tls_config = crate::tls::config(&options.additional_ca_pem)?;
        let http = reqwest::Client::builder()
            .use_preconfigured_tls((*tls_config).clone())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(options.request_timeout)
            .connect_timeout(options.connect_timeout)
            .user_agent(concat!("nekodash/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::transport("create HTTP client", &e))?;
        Ok(Self {
            endpoint,
            http,
            options,
            tls_config,
        })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub(crate) fn renewed(&self) -> Result<Self> {
        Self::with_options(self.endpoint.clone(), self.options.clone())
    }

    async fn request(
        &self,
        operation: &'static str,
        method: Method,
        path: &[&str],
        query: &[(&str, String)],
        body: Option<Value>,
        timeout: Option<Duration>,
    ) -> Result<Vec<u8>> {
        let url = self.endpoint.resource(path)?;
        let mut request = self.http.request(method, url).query(query);
        if let Some(auth) = self.endpoint.authorization()? {
            request = request.header(AUTHORIZATION, auth);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let response = request
            .send()
            .await
            .map_err(|e| Error::transport(operation, &e))?;
        self.read_response(operation, response).await
    }

    async fn read_response(
        &self,
        operation: &'static str,
        response: reqwest::Response,
    ) -> Result<Vec<u8>> {
        let status = response.status();
        // Bound error responses too, before reading their body. Preserve auth/status classification.
        let limit = if status.is_success() {
            self.options.max_response_bytes
        } else {
            8192
        };
        if response
            .content_length()
            .is_some_and(|len| len > limit as u64)
        {
            return Err(if status.is_success() {
                Error::new(
                    ErrorKind::ResponseTooLarge,
                    operation,
                    "response exceeds configured size limit",
                )
            } else {
                Error::http(operation, status.as_u16(), "core rejected request")
            });
        }
        let mut bytes = Vec::new();
        let mut chunks = response.bytes_stream();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|e| Error::transport(operation, &e))?;
            if chunk.len() > limit.saturating_sub(bytes.len()) {
                return Err(if status.is_success() {
                    Error::new(
                        ErrorKind::ResponseTooLarge,
                        operation,
                        "response exceeds configured size limit",
                    )
                } else {
                    Error::http(operation, status.as_u16(), "core rejected request")
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let message = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_else(|| "core rejected request".to_owned());
            let message: String = self.endpoint.redact(&message).chars().take(512).collect();
            return Err(Error::http(operation, status.as_u16(), message));
        }
        Ok(bytes)
    }

    async fn get<T: DeserializeOwned>(
        &self,
        operation: &'static str,
        path: &[&str],
        query: &[(&str, String)],
        timeout: Option<Duration>,
    ) -> Result<T> {
        let bytes = self
            .request(operation, Method::GET, path, query, None, timeout)
            .await?;
        serde_json::from_slice(&bytes).map_err(|e| Error::decode(operation, e))
    }

    pub async fn version(&self) -> Result<Version> {
        self.get("get version", &["version"], &[], None).await
    }
    pub async fn config(&self) -> Result<Config> {
        self.get("get config", &["configs"], &[], None).await
    }
    pub async fn proxies(&self) -> Result<Proxies> {
        self.get("get proxies", &["proxies"], &[], None).await
    }
    pub async fn proxy_providers(&self) -> Result<Providers<ProxyProvider>> {
        self.get("get proxy providers", &["providers", "proxies"], &[], None)
            .await
    }
    pub async fn rule_providers(&self) -> Result<Providers<RuleProvider>> {
        self.get("get rule providers", &["providers", "rules"], &[], None)
            .await
    }
    pub async fn connections(&self) -> Result<Option<Connections>> {
        self.get("get connections", &["connections"], &[], None)
            .await
    }

    pub async fn rules(&self) -> Result<Vec<Rule>> {
        let response: RulesResponse = self.get("get rules", &["rules"], &[], None).await?;
        let mut rules = match response.rules {
            RuleList::Array(rules) => rules
                .into_iter()
                .enumerate()
                .map(|(index, mut rule)| {
                    rule.index = Some(index as u64);
                    rule
                })
                .collect(),
            RuleList::Object(rules) => {
                let mut result = Vec::with_capacity(rules.len());
                for (key, mut rule) in rules {
                    rule.index = Some(key.parse::<u64>().map_err(|_| {
                        Error::new(
                            ErrorKind::Decode,
                            "get rules",
                            "rule index is not an unsigned integer",
                        )
                    })?);
                    result.push(rule);
                }
                result
            }
        };
        rules.sort_by_key(|rule| rule.index);
        Ok(rules)
    }

    pub async fn patch_config(&self, fields: Fields) -> Result<()> {
        self.request(
            "patch config",
            Method::PATCH,
            &["configs"],
            &[],
            Some(Value::Object(fields)),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn reload_config(&self) -> Result<()> {
        self.load_config("").await
    }

    /// Loads YAML into the selected running core. Caller can fetch/edit the text beforehand.
    pub async fn load_config(&self, payload: &str) -> Result<()> {
        self.request(
            "load config",
            Method::PUT,
            &["configs"],
            &[("force", "true".into())],
            Some(json!({"path":"", "payload":payload})),
            Some(Duration::from_secs(60)),
        )
        .await?;
        Ok(())
    }

    /// Fetches config text with a separate unauthenticated request, then loads it into this core.
    pub async fn load_config_url(&self, source: &str) -> Result<()> {
        let source =
            url::Url::parse(source).map_err(|_| Error::invalid("invalid config source URL"))?;
        if !matches!(source.scheme(), "http" | "https")
            || source.host_str().is_none()
            || !source.username().is_empty()
            || source.password().is_some()
            || source.fragment().is_some()
        {
            return Err(Error::invalid("config source requires an HTTP(S) URL"));
        }
        let downloader = reqwest::Client::builder()
            .use_preconfigured_tls((*self.tls_config).clone())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::limited(5))
            .connect_timeout(self.options.connect_timeout)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| Error::transport("create config downloader", &e))?;
        let response = downloader
            .get(source)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| Error::transport("fetch config", &e))?;
        let bytes = self
            .read_response("fetch config", response)
            .await
            .map_err(|mut error| {
                // External servers can echo subscription tokens from their query string.
                if let Some(status) = error.status {
                    error.message = format!("configuration download failed with HTTP {status}");
                }
                error
            })?;
        let text = String::from_utf8(bytes).map_err(|_| {
            Error::new(
                ErrorKind::Decode,
                "fetch config",
                "configuration is not UTF-8",
            )
        })?;
        self.load_config(&text).await
    }

    pub async fn select_proxy(&self, group: &str, node: &str) -> Result<()> {
        self.request(
            "select proxy",
            Method::PUT,
            &["proxies", group],
            &[],
            Some(json!({"name":node})),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn unfix_proxy(&self, group: &str) -> Result<()> {
        self.request(
            "restore automatic selection",
            Method::DELETE,
            &["proxies", group],
            &[],
            None,
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn update_proxy_provider(&self, name: &str) -> Result<()> {
        self.request(
            "update proxy provider",
            Method::PUT,
            &["providers", "proxies", name],
            &[],
            None,
            Some(Duration::from_secs(60)),
        )
        .await?;
        Ok(())
    }

    pub async fn update_rule_provider(&self, name: &str) -> Result<()> {
        self.request(
            "update rule provider",
            Method::PUT,
            &["providers", "rules", name],
            &[],
            None,
            Some(Duration::from_secs(60)),
        )
        .await?;
        Ok(())
    }

    pub async fn health_check_provider(&self, name: &str) -> Result<()> {
        self.request(
            "health check provider",
            Method::GET,
            &["providers", "proxies", name, "healthcheck"],
            &[],
            None,
            Some(Duration::from_secs(20)),
        )
        .await?;
        Ok(())
    }

    pub async fn test_proxy(&self, probe: &Probe) -> Result<Delay> {
        validate_probe(&probe.url, probe.timeout_ms)?;
        let path = match &probe.provider {
            Some(provider) => vec![
                "providers",
                "proxies",
                provider.as_str(),
                probe.node.as_str(),
                "healthcheck",
            ],
            None => vec!["proxies", probe.node.as_str(), "delay"],
        };
        self.get(
            "test proxy",
            &path,
            &[
                ("url", probe.url.clone()),
                ("timeout", probe.timeout_ms.to_string()),
            ],
            Some(Duration::from_millis(
                u64::from(probe.timeout_ms)
                    .saturating_add(10_000)
                    .max(20_000),
            )),
        )
        .await
    }

    pub async fn test_group(
        &self,
        group: &str,
        url: &str,
        timeout_ms: u32,
    ) -> Result<BTreeMap<String, u64>> {
        validate_probe(url, timeout_ms)?;
        self.get(
            "test group",
            &["group", group, "delay"],
            &[("url", url.into()), ("timeout", timeout_ms.to_string())],
            Some(Duration::from_millis(
                (u64::from(timeout_ms) * 2 + 10_000).max(30_000),
            )),
        )
        .await
    }

    pub async fn set_rule_disabled(&self, index: u64, disabled: bool) -> Result<()> {
        let mut body = Fields::new();
        body.insert(index.to_string(), Value::Bool(disabled));
        self.request(
            "set rule state",
            Method::PATCH,
            &["rules", "disable"],
            &[],
            Some(Value::Object(body)),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn close_connection(&self, id: &str) -> Result<()> {
        self.request(
            "close connection",
            Method::DELETE,
            &["connections", id],
            &[],
            None,
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn close_all_connections(&self) -> Result<()> {
        self.request(
            "close all connections",
            Method::DELETE,
            &["connections"],
            &[],
            None,
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn dns_query(&self, name: &str, kind: &str) -> Result<DnsResponse> {
        if name.trim().is_empty() || kind.trim().is_empty() {
            return Err(Error::invalid("DNS name and type are required"));
        }
        self.get(
            "query DNS",
            &["dns", "query"],
            &[("name", name.into()), ("type", kind.into())],
            None,
        )
        .await
    }

    /// Sends one command. A timeout can occur after the core applied it; reconcile state before retrying.
    pub async fn maintenance(&self, action: MaintenanceAction) -> Result<()> {
        self.request(
            "core maintenance",
            Method::POST,
            action.path(),
            &[],
            None,
            Some(Duration::from_secs(60)),
        )
        .await?;
        Ok(())
    }
}

fn validate_probe(url: &str, timeout_ms: u32) -> Result<()> {
    let url = url::Url::parse(url).map_err(|_| Error::invalid("invalid probe URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || timeout_ms == 0 {
        return Err(Error::invalid(
            "probe requires an HTTP(S) URL and positive timeout",
        ));
    }
    Ok(())
}
