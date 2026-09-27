//! Wire models retain unknown fields so newer core responses remain available to the UI.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub type Fields = Map<String, Value>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Version {
    pub version: String,
    #[serde(default)]
    pub meta: bool,
    #[serde(flatten)]
    pub additional: Fields,
}

impl Version {
    pub fn is_sing_box(&self) -> bool {
        self.version.to_ascii_lowercase().contains("sing-box")
    }
}

/// Runtime config and patches preserve core-specific keys and nested sections verbatim.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Config(pub Fields);

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("keys", &self.0.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DelayHistory {
    pub time: String,
    pub delay: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Proxy {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub now: String,
    #[serde(default)]
    pub fixed: Option<String>,
    #[serde(default)]
    pub history: Vec<DelayHistory>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub udp: bool,
    #[serde(default)]
    pub xudp: bool,
    #[serde(default)]
    pub tfo: bool,
    #[serde(default, rename = "testUrl")]
    pub test_url: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(flatten)]
    pub additional: Fields,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Proxies {
    pub proxies: BTreeMap<String, Proxy>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SubscriptionInfo {
    #[serde(default, rename = "Download")]
    pub download: u64,
    #[serde(default, rename = "Upload")]
    pub upload: u64,
    #[serde(default, rename = "Total")]
    pub total: u64,
    #[serde(default, rename = "Expire")]
    pub expire: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyProvider {
    pub name: String,
    #[serde(default)]
    pub proxies: Vec<Proxy>,
    #[serde(default)]
    pub subscription_info: Option<SubscriptionInfo>,
    #[serde(default)]
    pub test_url: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub vehicle_type: String,
    #[serde(flatten)]
    pub additional: Fields,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Providers<T> {
    pub providers: BTreeMap<String, T>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Rule {
    #[serde(default)]
    pub index: Option<u64>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub payload: String,
    pub proxy: String,
    #[serde(default)]
    // Mihomo uses -1 for rules whose size does not apply.
    pub size: Option<i64>,
    #[serde(default)]
    pub extra: Fields,
    #[serde(flatten)]
    pub additional: Fields,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum RuleList {
    Array(Vec<Rule>),
    Object(BTreeMap<String, Rule>),
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RulesResponse {
    pub rules: RuleList,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleProvider {
    pub name: String,
    #[serde(default)]
    pub behavior: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub rule_count: u64,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub vehicle_type: String,
    #[serde(flatten)]
    pub additional: Fields,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Delay {
    pub delay: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub download: u64,
    pub upload: u64,
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub rule_payload: String,
    pub start: String,
    pub metadata: Fields,
    #[serde(flatten)]
    pub additional: Fields,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connections {
    #[serde(default)]
    pub connections: Option<Vec<Connection>>,
    pub upload_total: u64,
    pub download_total: u64,
    #[serde(default)]
    pub memory: Option<u64>,
    #[serde(flatten)]
    pub additional: Fields,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Traffic {
    pub up: u64,
    pub down: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Memory {
    pub inuse: u64,
    #[serde(default)]
    pub oslimit: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Log {
    #[serde(rename = "type")]
    pub level: String,
    pub payload: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Silent,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Silent => "silent",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DnsAnswer {
    #[serde(rename = "TTL")]
    pub ttl: u64,
    pub data: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: u16,
    #[serde(flatten)]
    pub additional: Fields,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DnsResponse {
    pub status: u32,
    #[serde(default, rename = "Answer")]
    pub answer: Option<Vec<DnsAnswer>>,
    #[serde(flatten)]
    pub additional: Fields,
}
