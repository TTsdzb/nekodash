use super::*;
use serde_json::{Value, json};
use std::future::Future;

impl App {
    pub(super) fn invoke(&mut self, action: &str, key: &str, value: &str) {
        match action {
            "endpoints" => {
                if let Some(ui) = self.ui.upgrade() {
                    let v = ui.global::<ViewData>();
                    v.set_show_connect(true);
                    v.set_edit_id("".into());
                    v.set_edit_label("".into());
                    v.set_edit_url("http://127.0.0.1:9090".into());
                    v.set_edit_secret("".into());
                }
            }
            "switch" => {
                if let Some(endpoint) = self
                    .store
                    .endpoints()
                    .iter()
                    .find(|e| e.id() == key)
                    .cloned()
                {
                    self.switch(endpoint);
                }
            }
            "edit-endpoint" => {
                if let (Some(endpoint), Some(ui)) = (
                    self.store.endpoints().iter().find(|e| e.id() == key),
                    self.ui.upgrade(),
                ) {
                    let v = ui.global::<ViewData>();
                    v.set_edit_id(endpoint.id().into());
                    v.set_edit_label(endpoint.label().into());
                    v.set_edit_url(endpoint.url().as_str().into());
                    v.set_edit_secret(endpoint.secret().into());
                }
            }
            "remove-endpoint" => self.dialog(
                action,
                key,
                "delete",
                &self.tr("operationConfirm"),
                "",
                false,
            ),
            "cancel-connect" => {
                self.session.disconnect();
                self.recovering = false;
                self.connect_candidate = None;
                self.busy = false;
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ViewData>().set_connecting(false);
                    ui.global::<ViewData>()
                        .set_status(self.tr("disconnected").into());
                }
            }
            "theme" => {
                self.settings.dark = !self.settings.dark;
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<Theme>().set_dark(self.settings.dark);
                }
                self.save();
            }
            "language" => {
                self.draft_settings.clear();
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ViewData>()
                        .set_setting_rows(model(vec![ConfigRow {
                            key: "language".into(),
                            label: self.tr("switchLanguage").into(),
                            value: self.settings.language.clone().into(),
                            kind: 2,
                            options: model(
                                ["zh", "en", "ru", "ko", "fr", "ja", "fa"]
                                    .map(SharedString::from)
                                    .to_vec(),
                            ),
                            ..ConfigRow::default()
                        }]));
                    ui.global::<ViewData>().set_settings_visible(true);
                }
            }
            "refresh" => {
                if self.stream_states.iter().all(|v| *v) {
                    self.refresh();
                } else if !self.recovering {
                    self.recover(None);
                }
            }
            "mode" => {
                let mode = match value {
                    "0" => "rule",
                    "1" => "global",
                    "2" => "direct",
                    _ => return,
                };
                self.patch("mode", Value::String(mode.into()));
            }
            "select" => {
                let group = key.to_owned();
                let node = value.to_owned();
                let ids = if self.settings.close_after_select {
                    self.data.connection_ids_through_group(&group)
                } else {
                    vec![]
                };
                self.mutate(action, move |client| async move {
                    client.select_proxy(&group, &node).await?;
                    close_connections(&client, ids).await
                });
            }
            "unfix" => {
                let group = key.to_owned();
                let ids = if self.settings.close_after_select {
                    self.data.connection_ids_through_group(&group)
                } else {
                    vec![]
                };
                self.mutate(action, move |c| async move {
                    c.unfix_proxy(&group).await?;
                    close_connections(&c, ids).await
                });
            }
            "expand" => {
                if !self.collapsed.remove(key) {
                    self.collapsed.insert(key.into());
                }
            }
            "collapse-all" => {
                if self.collapsed.is_empty() {
                    self.collapsed.extend(self.data.proxies.keys().cloned());
                    self.collapsed
                        .extend(self.data.proxy_providers.keys().cloned());
                } else {
                    self.collapsed.clear();
                }
            }
            "display-mode" => {
                self.settings.display_mode = value.parse::<usize>().unwrap_or(0).min(1);
                self.save();
            }
            "test-all" => self.test(),
            "test-node" => {
                let provider = if self.tab == 1 {
                    Some(key.to_owned())
                } else {
                    self.data
                        .proxy(value)
                        .and_then(|(provider, _)| provider.map(str::to_owned))
                };
                let test_url = if self.tab == 1 {
                    self.data
                        .proxy_providers
                        .get(key)
                        .and_then(|p| p.test_url.as_deref())
                } else {
                    self.data
                        .proxies
                        .get(key)
                        .and_then(|p| p.test_url.as_deref())
                };
                let probe = Probe {
                    node: value.to_owned(),
                    provider,
                    url: self.settings.resolve_test_url(test_url),
                    timeout_ms: self.settings.test_timeout_ms,
                };
                self.mutate(action, move |c| async move {
                    c.test_proxy(&probe).await?;
                    Ok(())
                });
            }
            "test-group" => {
                let name = key.to_owned();
                let url = self.settings.resolve_test_url(
                    self.data
                        .proxies
                        .get(key)
                        .and_then(|p| p.test_url.as_deref()),
                );
                let timeout = self.settings.test_timeout_ms;
                self.mutate(action, move |c| async move {
                    c.test_group(&name, &url, timeout).await?;
                    Ok(())
                });
            }
            "test-provider" => {
                let name = key.to_owned();
                self.mutate(action, move |c| async move {
                    c.health_check_provider(&name).await
                });
            }
            "update-provider" => {
                let key = key.to_owned();
                let rule = self.page == 2;
                self.mutate(action, move |c| async move {
                    if rule {
                        c.update_rule_provider(&key).await
                    } else {
                        c.update_proxy_provider(&key).await
                    }
                });
            }
            "update-all" => {
                let rule = self.page == 2;
                let names = if rule {
                    self.data.rule_providers.keys().cloned().collect::<Vec<_>>()
                } else {
                    self.data
                        .proxy_providers
                        .values()
                        .filter(|p| p.name != "default" && p.vehicle_type != "Compatible")
                        .map(|p| p.name.clone())
                        .collect()
                };
                self.mutate(action, move |c| async move {
                    for name in names {
                        if rule {
                            c.update_rule_provider(&name).await?;
                        } else {
                            c.update_proxy_provider(&name).await?;
                        }
                    }
                    Ok(())
                });
            }
            "filter" => self.filter = value.parse().unwrap_or(0),
            "grouping" => self.grouping = value.parse().unwrap_or(0),
            "pause" => {
                self.paused[self.page] = !self.paused[self.page];
                if self.page == 3 && self.paused[3] {
                    self.paused_connections = self.data.connections.values().cloned().collect();
                }
                if self.page == 5 && self.paused[5] {
                    self.paused_logs = self.data.logs.iter().cloned().collect();
                }
            }
            "clear" => {
                if self.page == 5 {
                    self.data.logs.clear();
                    self.paused_logs.clear();
                } else if self.page == 3 {
                    self.data.closed.clear();
                }
            }
            "row-action" => self.row_action(key),
            "close-all" => self.dialog(
                action,
                "",
                "closeAll",
                &self.tr("operationConfirm"),
                "",
                false,
            ),
            "detail" => self.details(key),
            "maintenance" => self.dialog(
                action,
                key,
                key,
                &self.tr(if key == "restartCore" {
                    "restartCoreConfirm"
                } else if key == "upgradeCore" {
                    "upgradeCoreConfirm"
                } else if key == "upgradeUI" {
                    "upgradeUIConfirm"
                } else {
                    "operationConfirm"
                }),
                "",
                key == "fetchRemoteConfig",
            ),
            "confirm" => self.confirm(value),
            "dns" => {
                let name = key.trim().to_owned();
                let kind = value.to_owned();
                if name.is_empty() {
                    self.message(&self.tr("invalidInput"), true);
                    return;
                }
                self.command(action, move |c| async move {
                    let response = c.dns_query(&name, &kind).await?;
                    Ok(json!(response))
                });
            }
            "config" => self.config(key, value),
            "page-settings" => {
                self.draft_settings.clear();
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ViewData>()
                        .set_setting_rows(model(self.page_settings_rows()));
                    ui.global::<ViewData>().set_settings_visible(true);
                }
            }
            "setting" => {
                self.draft_settings.insert(key.into(), value.into());
            }
            "save-page-settings" => self.save_page_settings(),
            "export" => self.export_rows(key),
            "export-settings" => {
                match serde_json::to_vec_pretty(
                    &json!({"schema_version":1,"settings":self.settings,"endpoints":self.store}),
                ) {
                    Ok(bytes) => self.export_dialog(bytes, "nekodash-settings.json"),
                    Err(e) => self.message(&e.to_string(), true),
                }
            }
            "import-settings" => self.dialog(
                action,
                "",
                "importSettings",
                &self.tr("importPath"),
                "",
                true,
            ),
            "reset-settings" => self.dialog(
                action,
                "",
                "resetSettings",
                &self.tr("operationConfirm"),
                "",
                false,
            ),
            _ => {}
        }
        self.mark_dirty();
    }
    fn save_page_settings(&mut self) {
        let mut candidate = self.settings.clone();
        let parsed = (|| -> AppResult<()> {
            for (key, value) in &self.draft_settings {
                match key.as_str() {
                    "density" => {
                        candidate.table_density = ["sm", "md", "lg"]
                            .iter()
                            .position(|k| self.tr(k) == *value)
                            .unwrap_or(1)
                    }
                    "language" => candidate.language = value.clone(),
                    "test-source" => candidate.use_core_test_url = value == "true",
                    "test-url" => candidate.test_url = value.clone(),
                    "test-timeout" => candidate.test_timeout_ms = value.parse()?,
                    "concurrency" => candidate.test_concurrency = value.parse()?,
                    "sort" => {
                        candidate.proxy_sort = [
                            "orderNatural",
                            "orderLatency_asc",
                            "orderLatency_desc",
                            "orderName_asc",
                            "orderName_desc",
                        ]
                        .iter()
                        .position(|k| self.tr(k) == *value)
                        .unwrap_or(0)
                    }
                    "close-after-select" => candidate.close_after_select = value == "true",
                    "log-limit" => candidate.log_limit = value.parse()?,
                    "connection-limit" => candidate.connection_limit = value.parse()?,
                    "log-level" => {
                        candidate.log_level = ["debug", "info", "warning", "error", "silent"]
                            .iter()
                            .position(|k| self.tr(k) == *value)
                            .unwrap_or(1)
                    }
                    "retention" => candidate.retention_days = value.parse()?,
                    "track" => candidate.track_traffic = value == "true",
                    _ => {}
                }
            }
            candidate.validate()
        })();
        match parsed {
            Ok(()) => {
                let logs_changed = candidate.log_level != self.settings.log_level;
                self.settings = candidate;
                self.apply_preferences();
                self.save();
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ViewData>().set_settings_visible(false);
                }
                if logs_changed {
                    self.recover(None);
                }
            }
            Err(e) => self.message(&format!("{}: {}", self.tr("invalidInput"), e), true),
        }
    }
    fn command<F, Fut>(&mut self, action: &str, run: F)
    where
        F: FnOnce(CoreClient) -> Fut + Send + 'static,
        Fut: Future<Output = nekodash_core::Result<Value>> + Send + 'static,
    {
        if self.busy || self.recovering {
            return;
        }
        let Some(context) = self.session.current().cloned() else {
            return;
        };
        self.snapshot_epoch = self.snapshot_epoch.wrapping_add(1);
        self.refreshing = false;
        self.busy = true;
        let tx = self.tx.clone();
        let action = action.to_owned();
        self.runtime.spawn(async move {
            let result = context.run(run(context.client.clone())).await;
            let _ = tx.send(Event::Command(action, result)).await;
        });
    }
    fn mutate<F, Fut>(&mut self, action: &str, run: F)
    where
        F: FnOnce(CoreClient) -> Fut + Send + 'static,
        Fut: Future<Output = nekodash_core::Result<()>> + Send + 'static,
    {
        self.command(action, move |c| async move {
            run(c).await?;
            Ok(Value::Null)
        });
    }
    fn patch(&mut self, key: &str, value: Value) {
        let mut fields = models::Fields::new();
        if let Some((parent, child)) = key.split_once('.') {
            fields.insert(parent.into(), json!({child:value}));
        } else {
            fields.insert(key.into(), value);
        }
        let close_all = key == "mode" && self.settings.close_after_select;
        self.mutate("config", move |c| async move {
            c.patch_config(fields).await?;
            if close_all {
                c.close_all_connections().await?;
            }
            Ok(())
        });
    }
    fn config(&mut self, key: &str, value: &str) {
        if self.tab == 1 {
            let previous = self.settings.clone();
            match key {
                "dark" => self.invoke("theme", "", ""),
                "track" => self.settings.track_traffic = value == "true",
                "default" => {
                    self.settings.default_page = [
                        "overview",
                        "proxies",
                        "rules",
                        "connections",
                        "traffic",
                        "logs",
                        "config",
                    ]
                    .iter()
                    .position(|key| self.tr(key) == value)
                    .unwrap_or(0)
                }
                "language" => self.settings.language = value.into(),
                "test-source" => self.settings.use_core_test_url = value == "true",
                "test-url" => self.settings.test_url = value.into(),
                "test-timeout" => match value.parse::<u32>() {
                    Ok(v) if (100..=60_000).contains(&v) => self.settings.test_timeout_ms = v,
                    _ => {
                        self.message(&self.tr("invalidInput"), true);
                        return;
                    }
                },
                "close-after-select" => self.settings.close_after_select = value == "true",
                _ => return,
            }
            if let Err(error) = self.settings.validate() {
                self.settings = previous;
                self.message(&format!("{}: {error}", self.tr("invalidInput")), true);
                return;
            }
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<I18n>()
                    .set_language(self.settings.language.clone().into());
            }
            self.save();
            return;
        }
        let existing = if let Some((parent, child)) = key.split_once('.') {
            self.data.config.0.get(parent).and_then(|v| v.get(child))
        } else {
            self.data.config.0.get(key)
        };
        let value = if existing.is_some_and(Value::is_boolean)
            || matches!(key, "tun.enable" | "dns.use-hosts")
        {
            match value {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => return,
            }
        } else if existing.is_some_and(Value::is_number) || key.ends_with("port") {
            match value.parse::<u16>() {
                Ok(v) => json!(v),
                Err(_) => {
                    self.message(&self.tr("invalidInput"), true);
                    return;
                }
            }
        } else if key == "dns.nameserver" || key == "dns.fallback" {
            json!(
                value
                    .lines()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .collect::<Vec<_>>()
            )
        } else {
            Value::String(value.into())
        };
        self.patch(key, value);
    }
    fn row_action(&mut self, key: &str) {
        match (self.page, self.tab) {
            (2, 0) => {
                if let Some(rule) = self
                    .data
                    .rules
                    .iter()
                    .find(|r| r.index.map(|i| i.to_string()).as_deref() == Some(key))
                    && let Some(index) = rule.index
                {
                    let disabled = !rule
                        .extra
                        .get("disabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    self.mutate("rule", move |c| async move {
                        c.set_rule_disabled(index, disabled).await
                    });
                }
            }
            (2, 1) => self.invoke("update-provider", key, ""),
            (3, 0) => {
                let id = key.to_owned();
                self.mutate(
                    "close",
                    move |c| async move { c.close_connection(&id).await },
                );
            }
            _ => self.details(key),
        }
    }
    fn dialog(
        &mut self,
        action: &str,
        key: &str,
        title: &str,
        body: &str,
        value: &str,
        editable: bool,
    ) {
        self.pending = Some(Pending {
            action: action.into(),
            key: key.into(),
            token: self.session.current().map(|ctx| ctx.token.clone()),
        });
        if let Some(ui) = self.ui.upgrade() {
            let v = ui.global::<ViewData>();
            v.set_dialog_title(self.tr(title).into());
            v.set_dialog_body(body.into());
            v.set_dialog_value(value.into());
            v.set_dialog_edit(editable);
            v.set_dialog_confirm(true);
            v.set_dialog_visible(true);
        }
    }
    fn confirm(&mut self, value: &str) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if pending
            .token
            .as_ref()
            .is_some_and(|token| !self.session.accepts(token))
        {
            return;
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ViewData>().set_dialog_visible(false);
        }
        match pending.action.as_str() {
            "remove-endpoint" => {
                self.store.remove(&pending.key);
                if self
                    .session
                    .current()
                    .is_some_and(|ctx| ctx.token.endpoint_id == pending.key)
                {
                    self.save_usage();
                    self.session.disconnect();
                    self.data = Data::default();
                    self.usage_ready = false;
                    if let Some(ui) = self.ui.upgrade() {
                        let view = ui.global::<ViewData>();
                        view.set_connected(false);
                        view.set_connecting(false);
                        view.set_status(self.tr("disconnected").into());
                    }
                }
                self.save();
                if let Some(ui) = self.ui.upgrade() {
                    self.render_endpoints(&ui);
                }
            }
            "close-all" => self.mutate(
                "close-all",
                |c| async move { c.close_all_connections().await },
            ),
            "maintenance" => match pending.key.as_str() {
                "restartCore" => self.recover(Some(MaintenanceAction::Restart)),
                "upgradeCore" => self.recover(Some(MaintenanceAction::UpgradeCore)),
                "upgradeUI" => self.mutate("upgradeUI", |c| async move {
                    c.maintenance(MaintenanceAction::UpgradeHostedUi).await
                }),
                "flushFakeIP" => self.mutate("flushFakeIP", |c| async move {
                    c.maintenance(MaintenanceAction::FlushFakeIp).await
                }),
                "flushDNSCache" => self.mutate("flushDNSCache", |c| async move {
                    c.maintenance(MaintenanceAction::FlushDns).await
                }),
                "updateGEODatabases" => self.mutate("updateGEODatabases", |c| async move {
                    c.maintenance(MaintenanceAction::UpdateGeo).await
                }),
                "reloadConfig" => {
                    self.mutate("reloadConfig", |c| async move { c.reload_config().await })
                }
                "fetchRemoteConfig" => {
                    let url = value.trim().to_owned();
                    self.mutate("fetchRemoteConfig", |c| async move {
                        c.load_config_url(&url).await
                    });
                }
                _ => {}
            },
            "export-file" => {
                if let Some(bytes) = self.export.take() {
                    if value.trim().is_empty() {
                        self.message(&self.tr("invalidInput"), true);
                        return;
                    }
                    if let Err(e) = self
                        .saves
                        .try_send(Save::Export(PathBuf::from(value.trim()), bytes))
                    {
                        self.message(&e.to_string(), true);
                    }
                }
            }
            "import-settings" => {
                let path = PathBuf::from(value.trim());
                let tx = self.tx.clone();
                self.runtime.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        settings::read_bounded(&path, 4 * 1024 * 1024)
                    })
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })
                    .and_then(|r| r);
                    let _ = tx.send(Event::Imported("settings".into(), result)).await;
                });
            }
            "reset-settings" => {
                self.settings = Settings::default();
                self.save();
                self.apply_preferences();
            }
            _ => {}
        }
    }
    fn apply_preferences(&mut self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<I18n>()
                .set_language(self.settings.language.clone().into());
            ui.global::<Theme>().set_dark(self.settings.dark);
        }
        self.mark_dirty();
    }
    pub(super) fn imported(&mut self, kind: &str, result: AppResult<Vec<u8>>) {
        #[derive(serde::Deserialize)]
        struct Backup {
            schema_version: u32,
            settings: Settings,
            endpoints: EndpointStore,
        }
        let result = result.and_then(|bytes| {
            let backup: Backup = serde_json::from_slice(&bytes)?;
            if backup.schema_version != 1 {
                return Err("Unknown backup version".into());
            }
            backup.settings.validate()?;
            backup.endpoints.validate()?;
            Ok(backup)
        });
        match result {
            Ok(backup) if kind == "settings" => {
                self.settings = backup.settings;
                self.store = backup.endpoints;
                self.save();
                self.apply_preferences();
                if let Some(ui) = self.ui.upgrade() {
                    self.render_endpoints(&ui);
                }
                self.message(&self.tr("saved"), false);
            }
            Ok(_) => {}
            Err(e) => self.message(&e.to_string(), true),
        }
    }
    fn details(&mut self, key: &str) {
        let value = match self.page {
            2 if self.tab == 0 => self
                .data
                .rules
                .iter()
                .find(|r| r.index.map(|n| n.to_string()).as_deref() == Some(key))
                .map(|v| json!(v)),
            2 => self.data.rule_providers.get(key).map(|v| json!(v)),
            3 => self
                .data
                .connections
                .get(key)
                .or_else(|| self.data.closed.iter().find(|v| v.connection.id == key))
                .map(|v| json!(v.connection)),
            5 => self
                .data
                .logs
                .iter()
                .find(|v| v.sequence.to_string() == key)
                .map(|v| json!({"time":v.time,"type":v.log.level,"payload":v.log.payload})),
            _ => None,
        };
        if let (Some(value), Some(ui)) = (value, self.ui.upgrade()) {
            match serde_json::to_string_pretty(&value) {
                Ok(text) => {
                    let v = ui.global::<ViewData>();
                    v.set_dialog_title(self.tr("details").into());
                    v.set_dialog_body("".into());
                    v.set_dialog_value(text.into());
                    v.set_dialog_edit(false);
                    v.set_dialog_confirm(false);
                    v.set_dialog_visible(true);
                }
                Err(e) => self.message(&e.to_string(), true),
            }
        }
    }
    fn export_dialog(&mut self, bytes: Vec<u8>, name: &str) {
        self.export = Some(bytes);
        let path = self
            .folder
            .join("exports")
            .join(name)
            .to_string_lossy()
            .into_owned();
        self.dialog(
            "export-file",
            "",
            "exportSettings",
            &self.tr("exportPath"),
            &path,
            true,
        );
    }
    fn export_rows(&mut self, format: &str) {
        let rows: Vec<_> = self
            .rows
            .iter()
            .map(|row| row.cells.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .collect();
        if format == "csv" {
            let headers = self
                .ui
                .upgrade()
                .map(|ui| {
                    ui.global::<ViewData>()
                        .get_columns()
                        .iter()
                        .map(|v| v.title.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let csv = std::iter::once(headers)
                .chain(rows)
                .map(|row| {
                    row.iter()
                        .map(|cell| format!("\"{}\"", cell.replace('"', "\"\"")))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .collect::<Vec<_>>()
                .join("\r\n");
            self.export_dialog(
                csv.into_bytes(),
                &format!(
                    "nekodash-{}.csv",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                ),
            );
        } else {
            let records: Vec<Value> = self.rows.iter().filter_map(|row| {
                let key=row.key.as_str();
                match self.page {
                    3 => {
                        let connection = if self.tab == 0 && self.paused[3] {self.paused_connections.iter().find(|c| c.connection.id==key)} else {self.data.connections.get(key).or_else(||self.data.closed.iter().find(|c|c.connection.id==key))};
                        connection.map(|c|json!(c.connection))
                    }
                    5 => {
                        let log = if self.paused[5] {self.paused_logs.iter().find(|l|l.sequence.to_string()==key)} else {self.data.logs.iter().find(|l|l.sequence.to_string()==key)};
                        log.map(|l|json!({"id":l.sequence,"time":l.time,"type":l.log.level,"payload":l.log.payload}))
                    }
                    _ => Some(json!(row.cells.iter().map(|s|s.to_string()).collect::<Vec<_>>())),
                }
            }).collect();
            match serde_json::to_vec_pretty(&records) {
                Ok(bytes) => self.export_dialog(
                    bytes,
                    &format!(
                        "nekodash-{}.json",
                        chrono::Local::now().format("%Y%m%d-%H%M%S")
                    ),
                ),
                Err(e) => self.message(&e.to_string(), true),
            }
        }
    }
    fn test(&mut self) {
        if let Some(cancel) = &self.batch_cancel {
            cancel.cancel();
            return;
        }
        let Some(mut context) = self.session.current().cloned() else {
            return;
        };
        context.cancel = context.cancel.child_token();
        let mut probes = Vec::new();
        if self.tab == 1 {
            for provider in self
                .data
                .proxy_providers
                .values()
                .filter(|p| p.name != "default" && p.vehicle_type != "Compatible")
            {
                for proxy in &provider.proxies {
                    probes.push(Probe {
                        node: proxy.name.clone(),
                        provider: Some(provider.name.clone()),
                        url: self.settings.resolve_test_url(provider.test_url.as_deref()),
                        timeout_ms: self.settings.test_timeout_ms,
                    });
                }
            }
        } else {
            let mut tested = BTreeSet::new();
            for group in self
                .data
                .proxies
                .values()
                .filter(|p| !p.hidden && !p.all.is_empty())
            {
                let url = self.settings.resolve_test_url(group.test_url.as_deref());
                for name in &group.all {
                    if tested.insert((name.clone(), url.clone())) {
                        probes.push(Probe {
                            node: name.clone(),
                            provider: self
                                .data
                                .proxy(name)
                                .and_then(|(provider, _)| provider.map(str::to_owned)),
                            url: url.clone(),
                            timeout_ms: self.settings.test_timeout_ms,
                        });
                    }
                }
            }
        }
        let event = context.batch_tests(probes, self.settings.test_concurrency);
        match event.result {
            Ok(mut batch) => {
                self.batch_cancel = Some(context.cancel);
                let tx = self.tx.clone();
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ViewData>().set_testing(true);
                    ui.global::<ViewData>().set_progress("".into());
                }
                self.runtime.spawn(async move {
                    while let Some(event) = batch.recv().await {
                        if tx.send(Event::Batch(event)).await.is_err() {
                            break;
                        }
                    }
                });
            }
            Err(e) => self.error(&e),
        }
    }
}

async fn close_connections(client: &CoreClient, ids: Vec<String>) -> nekodash_core::Result<()> {
    let mut first_error = None;
    for id in ids {
        if let Err(error) = client.close_connection(&id).await {
            first_error.get_or_insert(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
