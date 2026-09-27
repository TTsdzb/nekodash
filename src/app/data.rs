use nekodash_core::{CoreSnapshot, StreamData, models::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone)]
pub struct LogEntry {
    pub sequence: u64,
    pub time: String,
    pub log: Log,
}
#[derive(Clone)]
pub struct LiveConnection {
    pub connection: Connection,
    pub up_rate: u64,
    pub down_rate: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub hour: i64,
    pub source: String,
    pub host: String,
    pub process: String,
    pub proxy: String,
    pub upload: u64,
    pub download: u64,
    pub count: u64,
}
#[derive(Default)]
pub struct Data {
    pub config: Config,
    pub proxies: BTreeMap<String, Proxy>,
    pub proxy_providers: BTreeMap<String, ProxyProvider>,
    pub rules: Vec<Rule>,
    pub rule_providers: BTreeMap<String, RuleProvider>,
    pub connections: BTreeMap<String, LiveConnection>,
    pub closed: VecDeque<LiveConnection>,
    pub logs: VecDeque<LogEntry>,
    pub next_log: u64,
    pub usage: BTreeMap<String, Usage>,
    pub upload: u64,
    pub download: u64,
    pub upload_total: u64,
    pub download_total: u64,
    pub memory: u64,
    pub rates: VecDeque<(u64, u64)>,
    pub memories: VecDeque<u64>,
    pub counts: VecDeque<u64>,
    pub last_connections: Option<std::time::Instant>,
}
impl Data {
    pub fn connection_ids_through_group(&self, group: &str) -> Vec<String> {
        self.connections
            .values()
            .filter(|v| v.connection.chains.iter().any(|name| name == group))
            .map(|v| v.connection.id.clone())
            .collect()
    }

    pub fn apply_snapshot(
        &mut self,
        snapshot: CoreSnapshot,
        limit: usize,
    ) -> Vec<nekodash_core::Error> {
        let mut errors = vec![];
        macro_rules! set {
            ($field:ident, $result:expr) => {
                match $result {
                    Ok(v) => self.$field = v,
                    Err(e) => errors.push(e),
                }
            };
        }
        set!(config, snapshot.config);
        set!(proxies, snapshot.proxies.map(|v| v.proxies));
        set!(
            proxy_providers,
            snapshot.proxy_providers.map(|v| v.providers)
        );
        set!(rules, snapshot.rules);
        set!(rule_providers, snapshot.rule_providers.map(|v| v.providers));
        match snapshot.connections {
            Ok(Some(v)) => self.update_connections(v, limit, false),
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
        errors
    }
    pub fn apply_stream(
        &mut self,
        value: &StreamData,
        log_limit: usize,
        connection_limit: usize,
        track: bool,
    ) {
        match value {
            StreamData::Traffic(v) => {
                self.upload = v.up;
                self.download = v.down;
                self.rates.push_back((v.up, v.down));
                trim(&mut self.rates, 120);
            }
            StreamData::Memory(v) => {
                self.memory = v.inuse;
                self.memories.push_back(v.inuse);
                trim(&mut self.memories, 120);
            }
            StreamData::Connections(Some(v)) => {
                self.update_connections(v.clone(), connection_limit, track)
            }
            StreamData::Connections(None) => {
                self.update_connections(Connections::default(), connection_limit, track)
            }
            StreamData::Log(v) => {
                self.next_log = self.next_log.saturating_add(1);
                self.logs.push_back(LogEntry {
                    sequence: self.next_log,
                    time: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
                    log: v.clone(),
                });
                trim(&mut self.logs, log_limit);
            }
        }
    }
    pub fn update_connections(&mut self, value: Connections, limit: usize, track: bool) {
        self.upload_total = value.upload_total;
        self.download_total = value.download_total;
        if let Some(memory) = value.memory {
            self.memory = memory;
        }
        let now = std::time::Instant::now();
        let elapsed = self
            .last_connections
            .map(|v| now.duration_since(v).as_secs_f64())
            .unwrap_or(1.0)
            .max(0.001);
        let hour = chrono::Utc::now().timestamp() / 3600;
        let mut next = BTreeMap::new();
        for connection in value.connections.unwrap_or_default() {
            let previous = self.connections.remove(&connection.id);
            let (up, down) = previous
                .as_ref()
                .map(|p| {
                    (
                        connection.upload.saturating_sub(p.connection.upload),
                        connection.download.saturating_sub(p.connection.download),
                    )
                })
                .unwrap_or((0, 0));
            if track {
                let source = metadata(&connection, "sourceIP");
                let host = host(&connection);
                let process = process(&connection);
                let proxy = connection.chains.last().cloned().unwrap_or_default();
                // Length-prefixed components avoid collisions from delimiters in host/process names.
                let key = format!(
                    "{hour}:{}:{source}{}:{host}{}:{process}{}:{proxy}",
                    source.len(),
                    host.len(),
                    process.len(),
                    proxy.len()
                );
                let usage = self.usage.entry(key).or_insert_with(|| Usage {
                    hour,
                    source,
                    host,
                    process,
                    proxy,
                    ..Usage::default()
                });
                usage.upload = usage.upload.saturating_add(if previous.is_some() {
                    up
                } else {
                    connection.upload
                });
                usage.download = usage.download.saturating_add(if previous.is_some() {
                    down
                } else {
                    connection.download
                });
                if previous.is_none() {
                    usage.count = usage.count.saturating_add(1);
                }
            }
            let entry = LiveConnection {
                up_rate: (up as f64 / elapsed) as u64,
                down_rate: (down as f64 / elapsed) as u64,
                connection,
            };
            next.insert(entry.connection.id.clone(), entry);
        }
        for (_, mut connection) in std::mem::take(&mut self.connections) {
            connection.up_rate = 0;
            connection.down_rate = 0;
            self.closed.push_front(connection);
        }
        while self.closed.len() > limit {
            self.closed.pop_back();
        }
        self.connections = next;
        self.last_connections = Some(now);
        self.counts.push_back(self.connections.len() as u64);
        trim(&mut self.counts, 120);
    }
    pub fn prune_usage(&mut self, days: u32) {
        if days > 0 {
            let earliest = chrono::Utc::now().timestamp() / 3600 - i64::from(days) * 24;
            self.usage.retain(|_, row| row.hour >= earliest);
        }
    }
}
fn trim<T>(values: &mut VecDeque<T>, limit: usize) {
    while values.len() > limit {
        values.pop_front();
    }
}
pub fn metadata(connection: &Connection, key: &str) -> String {
    connection
        .metadata
        .get(key)
        .map(value_text)
        .unwrap_or_default()
}
pub fn value_text(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
pub fn host(connection: &Connection) -> String {
    let name = metadata(connection, "host");
    if name.is_empty() {
        metadata(connection, "destinationIP")
    } else {
        name
    }
}
pub fn process(connection: &Connection) -> String {
    let name = metadata(connection, "process");
    if name.is_empty() {
        metadata(connection, "processPath")
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .into()
    } else {
        name
    }
}
pub fn bytes(value: u64) -> String {
    let mut scaled = value as f64;
    let mut unit = "B";
    for name in ["KB", "MB", "GB", "TB", "PB"] {
        if scaled < 1024.0 {
            break;
        }
        scaled /= 1024.0;
        unit = name;
    }
    if unit == "B" {
        format!("{value} B")
    } else {
        format!("{scaled:.1} {unit}")
    }
}
pub fn path(values: impl IntoIterator<Item = u64>, maximum: u64) -> String {
    let values: Vec<_> = values.into_iter().collect();
    let width = values.len().saturating_sub(1).max(1) as f64;
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            format!(
                "{} {:.2} {:.2}",
                if i == 0 { "M" } else { "L" },
                i as f64 * 600.0 / width,
                158.0 - *v as f64 / maximum.max(1) as f64 * 156.0
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn snapshot(id: &str, upload: u64, download: u64) -> Result<Connections, serde_json::Error> {
        serde_json::from_value(
            json!({"uploadTotal":upload,"downloadTotal":download,"connections":[{"id":id,"upload":upload,"download":download,"chains":["DIRECT","Proxy"],"start":"2026-09-28T00:00:00Z","metadata":{"host":"example.test","sourceIP":"127.0.0.2","network":"tcp"}}]}),
        )
    }
    #[test]
    fn tracks_deltas_without_counting_initial_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let mut data = Data::default();
        data.update_connections(snapshot("a", 1000, 2000)?, 100, false);
        data.update_connections(snapshot("a", 1100, 2500)?, 100, true);
        let usage = data.usage.values().next().ok_or("missing usage")?;
        assert_eq!((usage.upload, usage.download, usage.count), (100, 500, 0));
        data.update_connections(snapshot("a", 50, 100)?, 100, true);
        let usage = data.usage.values().next().ok_or("missing usage")?;
        assert_eq!((usage.upload, usage.download), (100, 500));
        Ok(())
    }
    #[test]
    fn retains_newest_closed_connections_and_full_first_sample()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut data = Data::default();
        for id in ["a", "b", "c", "d"] {
            data.update_connections(snapshot(id, 20, 40)?, 2, true);
        }
        assert_eq!(
            data.closed
                .iter()
                .map(|v| v.connection.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "b"]
        );
        let usage = data.usage.values().next().ok_or("missing usage")?;
        assert_eq!((usage.upload, usage.download, usage.count), (80, 160, 4));
        Ok(())
    }
    #[test]
    fn group_selection_targets_only_matching_connections() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut data = Data::default();
        let mut values = snapshot("matching", 0, 0)?;
        let mut unrelated = snapshot("unrelated", 0, 0)?
            .connections
            .ok_or("missing fixture connection")?;
        for connection in &mut unrelated {
            connection.chains = vec!["DIRECT".into()];
        }
        values
            .connections
            .as_mut()
            .ok_or("missing fixture connections")?
            .extend(unrelated);
        data.update_connections(values, 100, false);
        assert_eq!(data.connection_ids_through_group("Proxy"), vec!["matching"]);
        assert!(data.connection_ids_through_group("Other").is_empty());
        Ok(())
    }
    #[test]
    fn charts_remain_finite_for_empty_and_zero_series() {
        assert_eq!(path([], 0), "");
        assert!(!path([0, 0, 0], 0).contains("NaN"));
        assert_eq!(bytes(1024), "1.0 KB");
    }
}
