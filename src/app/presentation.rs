use super::*;
use data::{bytes, host, metadata, process, value_text};
use serde_json::Value;

fn strings(values: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    model(values.into_iter().map(Into::into).collect())
}
fn row(key: impl ToString, cells: Vec<String>, active: bool) -> DataRow {
    DataRow {
        key: key.to_string().into(),
        cells: strings(cells),
        active,
    }
}
impl App {
    pub(super) fn render_endpoints(&self, ui: &AppWindow) {
        let selected = self.session.current().map(|c| c.token.endpoint_id.as_str());
        ui.global::<ViewData>().set_endpoints(model(
            self.store
                .endpoints()
                .iter()
                .map(|endpoint| EndpointRow {
                    id: endpoint.id().into(),
                    label: endpoint.label().into(),
                    url: endpoint.url().as_str().into(),
                    selected: selected == Some(endpoint.id()),
                })
                .collect(),
        ));
        if let Some(context) = self.session.current() {
            ui.global::<ViewData>()
                .set_endpoint_url(context.client.endpoint().url().as_str().into());
            ui.global::<ViewData>()
                .set_endpoint_label(context.client.endpoint().label().into());
        }
    }
    pub(super) fn render(&self, ui: &AppWindow) {
        let v = ui.global::<ViewData>();
        let redraw_page = self.dirty_pages.get() & (1 << self.page) != 0;
        self.dirty_pages
            .set(self.dirty_pages.get() & !(1 << self.page));
        v.set_table_row_height(
            [36., 44., 52.]
                .get(self.settings.table_density)
                .copied()
                .unwrap_or(44.),
        );
        v.set_page(self.page as i32);
        v.set_tab(self.tab as i32);
        v.set_search(self.search.clone().into());
        v.set_busy(self.busy || self.recovering);
        v.set_paused(self.paused[self.page]);
        v.set_display_mode(self.settings.display_mode as i32);
        v.set_filter_index(self.filter as i32);
        v.set_group_index(self.grouping as i32);
        v.set_mode_index(
            match self.data.config.0.get("mode").and_then(Value::as_str) {
                Some("global") => 1,
                Some("direct") => 2,
                _ => 0,
            },
        );
        let stats = [
            ("upload", format!("{}/s", bytes(self.data.upload)), 8),
            ("download", format!("{}/s", bytes(self.data.download)), 9),
            ("uploadTotal", bytes(self.data.upload_total), 8),
            ("downloadTotal", bytes(self.data.download_total), 9),
            (
                "activeConnections",
                self.data.connections.len().to_string(),
                3,
            ),
            ("memoryUsage", bytes(self.data.memory), 4),
        ]
        .map(|(label, value, icon)| Stat {
            label: self.tr(label).into(),
            value: value.into(),
            icon,
        })
        .to_vec();
        replace(&self.stats, stats);
        if self.page == 0 && redraw_page {
            self.render_charts();
        }
        if self.page == 1 && redraw_page {
            self.render_groups();
        }
        if (2..=5).contains(&self.page) && redraw_page {
            self.render_table(ui);
        }
        if self.page == 6 && redraw_page {
            let mut rows = self.config_rows();
            for (index, row) in rows.iter_mut().enumerate() {
                if let Some(old) = self.config_rows_model.row_data(index)
                    && old.options.iter().eq(row.options.iter())
                {
                    row.options = old.options;
                }
            }
            replace(&self.config_rows_model, rows);
        }
        let filters: &[&str] = match self.page {
            2 if self.tab == 0 => &["all", "enabled", "disabled"],
            3 => &["all", "TCP", "UDP"],
            4 => &["lastHour", "lastDay", "lastWeek", "lastMonth", "all"],
            5 => &["all", "debug", "info", "warning", "error"],
            _ => &[],
        };
        let filters: Vec<SharedString> = filters.iter().map(|key| self.tr(key).into()).collect();
        if !v.get_filter_options().iter().eq(filters.iter().cloned()) {
            v.set_filter_options(model(filters));
        }
        let groups: &[&str] = match self.page {
            3 => &["none", "host", "process", "sourceIP", "chains"],
            4 => &["host", "process", "sourceIP", "chains"],
            _ => &[],
        };
        let groups: Vec<SharedString> = groups.iter().map(|key| self.tr(key).into()).collect();
        if !v.get_group_options().iter().eq(groups.iter().cloned()) {
            v.set_group_options(model(groups));
        }
        v.set_count(
            match self.page {
                1 => self
                    .data
                    .proxies
                    .values()
                    .filter(|p| !p.hidden && !p.all.is_empty())
                    .count(),
                2 => self.data.rules.len(),
                3 => self.data.connections.len(),
                5 => self.data.logs.len(),
                _ => self.rows.row_count(),
            }
            .to_string()
            .into(),
        );
        v.set_secondary_count(
            match self.page {
                1 => self
                    .data
                    .proxy_providers
                    .values()
                    .filter(|p| p.name != "default" && p.vehicle_type != "Compatible")
                    .count(),
                2 => self.data.rule_providers.len(),
                3 => self.data.closed.len(),
                _ => 0,
            }
            .to_string()
            .into(),
        );
    }
    fn render_charts(&self) {
        let max_rate = self
            .data
            .rates
            .iter()
            .map(|(a, b)| (*a).max(*b))
            .max()
            .unwrap_or(1);
        let max_mem = self.data.memories.iter().copied().max().unwrap_or(1);
        let max_count = self.data.counts.iter().copied().max().unwrap_or(1);
        let chart =
            |title: &str, primary: String, secondary: String, maximum: String, footer: String| {
                ChartRow {
                    kind: 0,
                    title: self.tr(title).into(),
                    primary: primary.into(),
                    secondary: secondary.into(),
                    maximum: maximum.into(),
                    footer: footer.into(),
                }
            };
        let footer = chrono::Local::now().format("%H:%M:%S").to_string();
        let mut charts = vec![
            chart(
                "traffic",
                data::path(self.data.rates.iter().map(|(a, _)| *a), max_rate),
                data::path(self.data.rates.iter().map(|(_, b)| *b), max_rate),
                bytes(max_rate),
                format!(
                    "{} / {}  ·  {}",
                    self.tr("upload"),
                    self.tr("download"),
                    footer
                ),
            ),
            chart(
                "memory",
                data::path(self.data.memories.iter().copied(), max_mem),
                String::new(),
                bytes(max_mem),
                footer.clone(),
            ),
            chart(
                "connections",
                data::path(self.data.counts.iter().copied(), max_count),
                String::new(),
                max_count.to_string(),
                footer,
            ),
        ];
        let pie = |title: &str, a: u64, b: u64, footer: String| {
            let sum = a.saturating_add(b);
            let fraction = if sum == 0 { 0.0 } else { a as f64 / sum as f64 };
            ChartRow {
                kind: 1,
                title: self.tr(title).into(),
                primary: wedge(0.0, fraction).into(),
                secondary: wedge(fraction, if sum == 0 { 0.0 } else { 1.0 }).into(),
                maximum: "".into(),
                footer: footer.into(),
            }
        };
        charts.insert(
            1,
            pie(
                "total",
                self.data.upload_total,
                self.data.download_total,
                format!(
                    "{} {}  /  {} {}",
                    self.tr("upload"),
                    bytes(self.data.upload_total),
                    self.tr("download"),
                    bytes(self.data.download_total)
                ),
            ),
        );
        let tcp = self
            .data
            .connections
            .values()
            .filter(|c| metadata(&c.connection, "network").eq_ignore_ascii_case("tcp"))
            .count() as u64;
        let udp = self
            .data
            .connections
            .values()
            .filter(|c| metadata(&c.connection, "network").eq_ignore_ascii_case("udp"))
            .count() as u64;
        charts.push(pie(
            "networkType",
            tcp,
            udp,
            format!("TCP {tcp}  /  UDP {udp}"),
        ));
        let mut policies: BTreeMap<String, u64> = BTreeMap::new();
        for c in self.data.connections.values() {
            if let Some(name) = c.connection.chains.last() {
                let total = policies.entry(name.clone()).or_default();
                *total =
                    total.saturating_add(c.connection.upload.saturating_add(c.connection.download));
            }
        }
        let mut policies: Vec<_> = policies.into_iter().collect();
        policies.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        policies.truncate(5);
        let max = policies.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1) as f64;
        let bars = policies
            .iter()
            .enumerate()
            .map(|(i, (_, n))| {
                let y = i * 30;
                let x = *n as f64 / max * 590.0;
                format!("M 0 {y} H {x} V {} H 0 Z", y + 18)
            })
            .collect::<Vec<_>>()
            .join(" ");
        charts.push(ChartRow {
            kind: 2,
            title: self.tr("topProxies").into(),
            primary: bars.into(),
            secondary: "".into(),
            maximum: "".into(),
            footer: policies
                .iter()
                .map(|(name, n)| format!("{name}: {}", bytes(*n)))
                .collect::<Vec<_>>()
                .join(" · ")
                .into(),
        });
        replace(&self.charts, charts);
    }
    fn render_groups(&self) {
        let query = self.search.to_lowercase();
        let mut groups = Vec::new();
        let make_node = |name: &str, proxy: Option<&models::Proxy>, current: &str| {
            let delay = proxy.and_then(|p| p.history.last()).map(|h| h.delay);
            NodeRow {
                name: name.into(),
                kind: proxy.map(|p| p.kind.as_str()).unwrap_or("").into(),
                delay: delay
                    .map(|v| {
                        if v == 0 {
                            "Timeout".into()
                        } else {
                            format!("{v} ms")
                        }
                    })
                    .unwrap_or_else(|| "—".into())
                    .into(),
                latency: delay.map(|v| v.min(i32::MAX as u64) as i32).unwrap_or(-1),
                selected: name == current,
                udp: proxy.is_some_and(|p| p.udp),
            }
        };
        if self.tab == 0 {
            // GLOBAL's member order is Mihomo's order for named policy groups.
            let order: Vec<_> = self
                .data
                .proxies
                .get("GLOBAL")
                .into_iter()
                .flat_map(|p| p.all.iter())
                .filter_map(|name| self.data.proxies.get(name))
                .chain(self.data.proxies.values())
                .collect();
            let mut seen = BTreeSet::new();
            for group in order {
                if group.all.is_empty() || group.hidden || !seen.insert(group.name.clone()) {
                    continue;
                }
                let matches = group.name.to_lowercase().contains(&query);
                let mut nodes: Vec<_> = group
                    .all
                    .iter()
                    .filter(|name| matches || name.to_lowercase().contains(&query))
                    .map(|name| make_node(name, self.data.proxies.get(name), &group.now))
                    .collect();
                if !query.is_empty() && !matches && nodes.is_empty() {
                    continue;
                }
                self.sort_nodes(&mut nodes);
                groups.push(GroupRow {
                    name: group.name.clone().into(),
                    kind: group.kind.clone().into(),
                    current: group.now.clone().into(),
                    fixed: group.fixed.as_ref().is_some_and(|v| !v.is_empty()),
                    expanded: !self.collapsed.contains(&group.name),
                    count: group.all.len().to_string().into(),
                    nodes: model(nodes),
                });
            }
        } else {
            for provider in self
                .data
                .proxy_providers
                .values()
                .filter(|p| p.name != "default" && p.vehicle_type != "Compatible")
            {
                let matches = provider.name.to_lowercase().contains(&query);
                let mut nodes: Vec<_> = provider
                    .proxies
                    .iter()
                    .filter(|p| matches || p.name.to_lowercase().contains(&query))
                    .map(|proxy| make_node(&proxy.name, Some(proxy), ""))
                    .collect();
                if !query.is_empty() && !matches && nodes.is_empty() {
                    continue;
                }
                self.sort_nodes(&mut nodes);
                let current = provider
                    .subscription_info
                    .as_ref()
                    .map(|info| {
                        format!(
                            "{} / {}",
                            bytes(info.upload.saturating_add(info.download).max(0) as u64),
                            bytes(info.total.max(0) as u64)
                        )
                    })
                    .unwrap_or_else(|| provider.updated_at.clone().unwrap_or_default());
                groups.push(GroupRow {
                    name: provider.name.clone().into(),
                    kind: provider.vehicle_type.clone().into(),
                    current: current.into(),
                    fixed: false,
                    expanded: !self.collapsed.contains(&provider.name),
                    count: provider.proxies.len().to_string().into(),
                    nodes: model(nodes),
                });
            }
        }
        // Preserve nested models on unchanged snapshots so a traffic tick doesn't recreate node cards.
        for (index, group) in groups.iter_mut().enumerate() {
            if let Some(old) = self.groups.row_data(index)
                && old.nodes.iter().eq(group.nodes.iter())
            {
                group.nodes = old.nodes;
            }
        }
        replace(&self.groups, groups);
    }
    fn sort_nodes(&self, nodes: &mut [NodeRow]) {
        match self.settings.proxy_sort {
            1 => nodes.sort_by_key(|p| if p.latency <= 0 { i32::MAX } else { p.latency }),
            2 => nodes.sort_by_key(|p| std::cmp::Reverse(p.latency)),
            3 => nodes.sort_by_key(|a| a.name.to_lowercase()),
            4 => nodes.sort_by_key(|a| std::cmp::Reverse(a.name.to_lowercase())),
            _ => {}
        }
    }
    fn render_table(&self, ui: &AppWindow) {
        let (headers, mut rows, action): (Vec<(&str, f32)>, Vec<DataRow>, &str) =
            match (self.page, self.tab) {
                (2, 0) => {
                    let rows = self
                        .data
                        .rules
                        .iter()
                        .filter(|rule| {
                            let disabled = rule
                                .extra
                                .get("disabled")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            self.filter == 0
                                || (self.filter == 1 && !disabled)
                                || (self.filter == 2 && disabled)
                        })
                        .map(|rule| {
                            let disabled = rule
                                .extra
                                .get("disabled")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            row(
                                rule.index.unwrap_or(0),
                                vec![
                                    rule.index.unwrap_or(0).to_string(),
                                    self.tr(if disabled { "disabled" } else { "enabled" }),
                                    rule.kind.clone(),
                                    rule.payload.clone(),
                                    rule.proxy.clone(),
                                    rule.size.map(|n| n.to_string()).unwrap_or_default(),
                                    rule.extra
                                        .get("hitCount")
                                        .map(value_text)
                                        .unwrap_or_else(|| "0".into()),
                                    rule.extra.get("hitAt").map(value_text).unwrap_or_default(),
                                ],
                                !disabled,
                            )
                        })
                        .collect();
                    (
                        vec![
                            ("ID", 60.),
                            ("status", 90.),
                            ("type", 150.),
                            ("payload", 300.),
                            ("proxies", 180.),
                            ("size", 90.),
                            ("hitCount", 100.),
                            ("lastMatchedAt", 160.),
                        ],
                        rows,
                        "⇄",
                    )
                }
                (2, _) => {
                    let rows = self
                        .data
                        .rule_providers
                        .values()
                        .map(|p| {
                            row(
                                &p.name,
                                vec![
                                    p.name.clone(),
                                    p.vehicle_type.clone(),
                                    p.behavior.clone(),
                                    p.format.clone(),
                                    p.rule_count.to_string(),
                                    p.updated_at.clone().unwrap_or_default(),
                                ],
                                true,
                            )
                        })
                        .collect();
                    (
                        vec![
                            ("name", 240.),
                            ("type", 100.),
                            ("behavior", 120.),
                            ("format", 90.),
                            ("rules", 100.),
                            ("updated", 260.),
                        ],
                        rows,
                        "update",
                    )
                }
                (3, _) => {
                    let connections: Vec<_> = if self.tab == 0 && self.paused[3] {
                        self.paused_connections.iter().collect()
                    } else if self.tab == 0 {
                        self.data.connections.values().collect()
                    } else {
                        self.data.closed.iter().collect()
                    };
                    let rows = connections
                        .into_iter()
                        .filter(|c| {
                            self.filter == 0
                                || metadata(&c.connection, "network").eq_ignore_ascii_case(
                                    if self.filter == 1 { "tcp" } else { "udp" },
                                )
                        })
                        .map(|entry| {
                            let c = &entry.connection;
                            row(
                                &c.id,
                                vec![
                                    format!("{} ({})", metadata(c, "type"), metadata(c, "network")),
                                    process(c),
                                    format!("{}:{}", host(c), metadata(c, "destinationPort")),
                                    self.source_name(&metadata(c, "sourceIP")),
                                    metadata(c, "destinationIP"),
                                    format!("{}/s", bytes(entry.down_rate)),
                                    format!("{}/s", bytes(entry.up_rate)),
                                    bytes(c.download),
                                    bytes(c.upload),
                                    format!("{} {}", c.rule, c.rule_payload),
                                    c.chains.join(" › "),
                                    c.start.clone(),
                                ],
                                true,
                            )
                        })
                        .collect();
                    (
                        vec![
                            ("type", 160.),
                            ("process", 170.),
                            ("host", 270.),
                            ("sourceIP", 170.),
                            ("destination", 180.),
                            ("dlSpeed", 100.),
                            ("ulSpeed", 100.),
                            ("dl", 100.),
                            ("ul", 100.),
                            ("rules", 240.),
                            ("chains", 250.),
                            ("connectTime", 250.),
                        ],
                        rows,
                        if self.tab == 0 { "close" } else { "" },
                    )
                }
                (4, _) => {
                    let hours: i64 = match self.filter {
                        0 => 1,
                        1 => 24,
                        2 => 168,
                        3 => 720,
                        _ => i64::MAX,
                    };
                    let cutoff = (chrono::Utc::now().timestamp() / 3600)
                        .saturating_sub(hours.saturating_sub(1));
                    let mut grouped: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
                    for v in self.data.usage.values().filter(|v| v.hour >= cutoff) {
                        let key = match self.grouping {
                            1 => v.process.clone(),
                            2 => self.source_name(&v.source),
                            3 => v.proxy.clone(),
                            _ => v.host.clone(),
                        };
                        let entry = grouped.entry(key).or_default();
                        entry.0 = entry.0.saturating_add(v.upload);
                        entry.1 = entry.1.saturating_add(v.download);
                        entry.2 = entry.2.saturating_add(v.count);
                    }
                    let rows = grouped
                        .into_iter()
                        .map(|(key, (up, down, count))| {
                            row(
                                &key,
                                vec![
                                    key.clone(),
                                    bytes(up),
                                    bytes(down),
                                    bytes(up.saturating_add(down)),
                                    count.to_string(),
                                ],
                                true,
                            )
                        })
                        .collect();
                    (
                        vec![
                            ("name", 320.),
                            ("upload", 140.),
                            ("download", 140.),
                            ("total", 140.),
                            ("connections", 120.),
                        ],
                        rows,
                        "",
                    )
                }
                (5, _) => {
                    let level = ["", "debug", "info", "warning", "error"]
                        .get(self.filter)
                        .copied()
                        .unwrap_or("");
                    let logs: Vec<_> = if self.paused[5] {
                        self.paused_logs.iter().collect()
                    } else {
                        self.data.logs.iter().collect()
                    };
                    let rows = logs
                        .into_iter()
                        .rev()
                        .filter(|v| level.is_empty() || v.log.level == level)
                        .map(|v| {
                            row(
                                v.sequence,
                                vec![
                                    v.sequence.to_string(),
                                    v.time.clone(),
                                    v.log.level.clone(),
                                    v.log.payload.clone(),
                                ],
                                true,
                            )
                        })
                        .collect();
                    (
                        vec![
                            ("ID", 70.),
                            ("time", 140.),
                            ("type", 100.),
                            ("payload", 1100.),
                        ],
                        rows,
                        "",
                    )
                }
                _ => (vec![], vec![], ""),
            };
        let query = self.search.to_lowercase();
        if !query.is_empty() {
            rows.retain(|row| row.cells.iter().any(|v| v.to_lowercase().contains(&query)));
        }
        if self.page == 3 && self.grouping > 0 {
            let index = match self.grouping {
                1 => 2,
                2 => 1,
                3 => 3,
                _ => 10,
            };
            let mut groups: BTreeMap<String, Vec<DataRow>> = BTreeMap::new();
            for item in rows {
                let key = item.cells.row_data(index).unwrap_or_default().to_string();
                groups.entry(key).or_default().push(item);
            }
            rows = groups.into_values().flatten().collect();
        }
        if let Some(column) = self.sort {
            rows.sort_by(|a, b| {
                let a = a.cells.row_data(column).unwrap_or_default();
                let b = b.cells.row_data(column).unwrap_or_default();
                let order = compare(&a, &b);
                if self.descending {
                    order.reverse()
                } else {
                    order
                }
            });
        }
        let columns: Vec<_> = headers
            .into_iter()
            .map(|(key, size)| Column {
                title: self.tr(key).into(),
                size,
            })
            .collect();
        let v = ui.global::<ViewData>();
        v.set_table_width(columns.iter().map(|c| c.size).sum::<f32>() + 72.);
        if !v.get_columns().iter().eq(columns.iter().cloned()) {
            v.set_columns(model(columns));
        }
        v.set_row_action(if action.is_empty() {
            "".into()
        } else {
            self.tr(action).into()
        });
        v.set_table_status(format!("{} {}", rows.len(), self.tr("total")).into());
        for (index, item) in rows.iter_mut().enumerate() {
            if let Some(old) = self.rows.row_data(index)
                && old.cells.iter().eq(item.cells.iter())
            {
                item.cells = old.cells;
            }
        }
        replace(&self.rows, rows);
    }
    fn source_name(&self, source: &str) -> String {
        self.settings
            .source_tags
            .get(source)
            .cloned()
            .unwrap_or_else(|| source.into())
    }
    pub(super) fn page_settings_rows(&self) -> Vec<ConfigRow> {
        let edit = |key: &str, label: &str, value: String| ConfigRow {
            key: key.into(),
            label: self.tr(label).into(),
            value: value.into(),
            kind: 1,
            ..ConfigRow::default()
        };
        let check = |key: &str, label: &str, checked: bool| ConfigRow {
            key: key.into(),
            label: self.tr(label).into(),
            checked,
            kind: 0,
            ..ConfigRow::default()
        };
        let choice = |key: &str, label: &str, selected: usize, options: &[&str]| ConfigRow {
            key: key.into(),
            label: self.tr(label).into(),
            value: self.tr(options.get(selected).copied().unwrap_or("")).into(),
            kind: 2,
            options: strings(options.iter().map(|key| self.tr(key))),
            ..ConfigRow::default()
        };
        let mut rows = match self.page {
            1 => vec![
                check(
                    "test-source",
                    "latencyTestUrlSourceCore",
                    self.settings.use_core_test_url,
                ),
                edit(
                    "test-url",
                    "urlForLatencyTest",
                    self.settings.test_url.clone(),
                ),
                edit(
                    "test-timeout",
                    "latencyTestTimeoutDuration",
                    self.settings.test_timeout_ms.to_string(),
                ),
                edit(
                    "concurrency",
                    "testConcurrency",
                    self.settings.test_concurrency.to_string(),
                ),
                choice(
                    "sort",
                    "proxiesSorting",
                    self.settings.proxy_sort,
                    &[
                        "orderNatural",
                        "orderLatency_asc",
                        "orderLatency_desc",
                        "orderName_asc",
                        "orderName_desc",
                    ],
                ),
                check(
                    "close-after-select",
                    "autoCloseConns",
                    self.settings.close_after_select,
                ),
            ],
            3 => vec![edit(
                "connection-limit",
                "connectionHistoryLimit",
                self.settings.connection_limit.to_string(),
            )],
            4 => vec![
                check(
                    "track",
                    "enableDataUsageTracking",
                    self.settings.track_traffic,
                ),
                edit(
                    "retention",
                    "retentionDays",
                    self.settings.retention_days.to_string(),
                ),
            ],
            5 => vec![
                edit(
                    "log-limit",
                    "logMaxRows",
                    self.settings.log_limit.to_string(),
                ),
                choice(
                    "log-level",
                    "logLevel",
                    self.settings.log_level,
                    &["debug", "info", "warning", "error", "silent"],
                ),
            ],
            _ => vec![],
        };
        if (2..=5).contains(&self.page) {
            rows.insert(
                0,
                choice(
                    "density",
                    "tableSize",
                    self.settings.table_density,
                    &["sm", "md", "lg"],
                ),
            );
        }
        rows
    }
    fn config_rows(&self) -> Vec<ConfigRow> {
        let text = |key: &str,
                    label: &str,
                    value: String,
                    kind: i32,
                    checked: bool,
                    options: Vec<String>| ConfigRow {
            key: key.into(),
            label: self.tr(label).into(),
            value: value.into(),
            kind,
            checked,
            options: strings(options),
        };
        if self.tab == 1 {
            return vec![
                text(
                    "dark",
                    "switchTheme",
                    String::new(),
                    0,
                    self.settings.dark,
                    vec![],
                ),
                text(
                    "language",
                    "switchLanguage",
                    self.settings.language.clone(),
                    2,
                    false,
                    ["zh", "en", "ru", "ko", "fr", "ja", "fa"]
                        .map(String::from)
                        .to_vec(),
                ),
                text(
                    "track",
                    "enableDataUsageTracking",
                    String::new(),
                    0,
                    self.settings.track_traffic,
                    vec![],
                ),
                text(
                    "default",
                    "defaultPage",
                    self.tr([
                        "overview",
                        "proxies",
                        "rules",
                        "connections",
                        "traffic",
                        "logs",
                        "config",
                    ]
                    .get(self.settings.default_page)
                    .copied()
                    .unwrap_or("overview")),
                    2,
                    false,
                    [
                        "overview",
                        "proxies",
                        "rules",
                        "connections",
                        "traffic",
                        "logs",
                        "config",
                    ]
                    .map(|k| self.tr(k))
                    .to_vec(),
                ),
                text(
                    "test-source",
                    "latencyTestUrlSourceCore",
                    String::new(),
                    0,
                    self.settings.use_core_test_url,
                    vec![],
                ),
                text(
                    "test-url",
                    "urlForLatencyTest",
                    self.settings.test_url.clone(),
                    1,
                    false,
                    vec![],
                ),
                text(
                    "test-timeout",
                    "latencyTestTimeoutDuration",
                    self.settings.test_timeout_ms.to_string(),
                    1,
                    false,
                    vec![],
                ),
                text(
                    "close-after-select",
                    "autoCloseConns",
                    String::new(),
                    0,
                    self.settings.close_after_select,
                    vec![],
                ),
            ];
        }
        let fields = [
            ("allow-lan", "allowLan", 0),
            ("mode", "runningMode", 2),
            ("log-level", "logLevel", 2),
            ("ipv6", "IPv6", 0),
            ("unified-delay", "unifiedDelay", 0),
            ("tcp-concurrent", "TCP Concurrent", 0),
            ("interface-name", "outboundInterfaceName", 1),
            ("tun.enable", "enableTunDevice", 0),
            ("tun.stack", "tunModeStack", 2),
            ("tun.device", "tunDeviceName", 1),
            ("mixed-port", "Mixed Port", 1),
            ("port", "HTTP Port", 1),
            ("socks-port", "SOCKS Port", 1),
            ("redir-port", "Redirect Port", 1),
            ("tproxy-port", "TProxy Port", 1),
            ("dns.enhanced-mode", "dnsEnhancedMode", 2),
            ("dns.fake-ip-range", "dnsFakeIpRange", 1),
            ("dns.use-hosts", "dnsUseHosts", 0),
        ];
        fields
            .into_iter()
            .map(|(key, label, kind)| {
                let value = if let Some((parent, child)) = key.split_once('.') {
                    self.data.config.0.get(parent).and_then(|v| v.get(child))
                } else {
                    self.data.config.0.get(key)
                };
                let options: Vec<String> = match key {
                    "mode" => vec!["rule", "global", "direct"],
                    "log-level" => vec!["debug", "info", "warning", "error", "silent"],
                    "tun.stack" => vec!["Mixed", "System", "gVisor"],
                    "dns.enhanced-mode" => vec!["fake-ip", "redir-host"],
                    _ => vec![],
                }
                .into_iter()
                .map(String::from)
                .collect();
                text(
                    key,
                    label,
                    value.map(value_text).unwrap_or_default(),
                    kind,
                    value.and_then(Value::as_bool).unwrap_or(false),
                    options,
                )
            })
            .collect()
    }
}
fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    fn number(value: &str) -> Option<f64> {
        let mut words = value.split_whitespace();
        let num = words.next()?.parse::<f64>().ok()?;
        let unit = words.next().unwrap_or("").trim_end_matches("/s");
        let scale = match unit {
            "KB" => 1024.,
            "MB" => 1024f64.powi(2),
            "GB" => 1024f64.powi(3),
            "TB" => 1024f64.powi(4),
            _ => 1.,
        };
        Some(num * scale)
    }
    match (number(a), number(b)) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

fn wedge(start: f64, end: f64) -> String {
    if end <= start {
        return String::new();
    }
    let point = |fraction: f64| {
        let angle = fraction * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
        (300.0 + 68.0 * angle.cos(), 80.0 + 68.0 * angle.sin())
    };
    let (x, y) = point(start);
    let (end_x, end_y) = point(end);
    if end - start >= 0.999999 {
        "M 300 12 A 68 68 0 1 1 300 148 A 68 68 0 1 1 300 12 Z".into()
    } else {
        format!(
            "M 300 80 L {x} {y} A 68 68 0 {} 1 {end_x} {end_y} Z",
            if end - start > 0.5 { 1 } else { 0 }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sorts_formatted_traffic_numerically() {
        assert!(compare("900 KB/s", "1.0 MB/s").is_lt());
        assert!(compare("2", "10").is_lt());
        assert!(compare("0 B", "20 B").is_lt());
    }
    #[test]
    fn pie_paths_handle_empty_and_single_category() {
        assert!(wedge(0., 0.).is_empty());
        assert!(wedge(0., 1.).matches("A 68").count() == 2);
        assert!(wedge(0., 0.25).contains("0 0 1"));
    }
}
