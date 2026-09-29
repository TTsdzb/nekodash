mod commands;
mod data;
mod i18n;
mod log_inbox;
mod presentation;
mod settings;
pub(super) use settings::data_dir as application_data_dir;

use crate::generated::*;
use data::Data;
use i18n::Translations;
use nekodash_core::{models::LogLevel, *};
use settings::{AppResult, Settings};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{runtime::Handle, sync::mpsc};

#[derive(Clone)]
struct Pending {
    action: String,
    key: String,
    token: Option<SessionToken>,
}
enum Event {
    Usage(SessionToken, AppResult<BTreeMap<String, data::Usage>>),
    Recovery(SessionEvent<RecoveryReport>),
    Snapshot(u64, SessionEvent<CoreSnapshot>),
    Stream(StreamKind, SessionEvent<StreamEvent>),
    Command(String, SessionEvent<serde_json::Value>),
    Batch(SessionEvent<BatchEvent>),
    Storage(String),
    Imported(String, AppResult<Vec<u8>>),
}
enum Save {
    LoadUsage(String, SessionToken),
    Flush(tokio::sync::oneshot::Sender<std::result::Result<(), String>>),
    Preferences(EndpointStore, Settings),
    Usage(String, BTreeMap<String, data::Usage>),
    Export(PathBuf, Vec<u8>),
}
struct App {
    ui: slint::Weak<AppWindow>,
    runtime: Handle,
    tx: mpsc::Sender<Event>,
    logs_inbox: Arc<log_inbox::LogInbox>,
    saves: mpsc::Sender<Save>,
    session: Session,
    store: EndpointStore,
    settings: Settings,
    draft_settings: BTreeMap<String, String>,
    translations: Arc<Translations>,
    data: Data,
    usage_ready: bool,
    page: usize,
    tab: usize,
    filter: usize,
    grouping: usize,
    search: String,
    sort: Option<usize>,
    descending: bool,
    collapsed: BTreeSet<String>,
    config_rows_model: Rc<VecModel<ConfigRow>>,
    rows: Rc<VecModel<DataRow>>,
    groups: Rc<VecModel<GroupRow>>,
    stats: Rc<VecModel<Stat>>,
    charts: Rc<VecModel<ChartRow>>,
    pending: Option<Pending>,
    batch_cancel: Option<CancellationToken>,
    snapshot_epoch: u64,
    recovering: bool,
    refreshing: bool,
    busy: bool,
    paused: [bool; 7],
    paused_connections: Vec<data::LiveConnection>,
    paused_logs: Vec<data::LogEntry>,
    stream_states: [bool; 4],
    stream_seen: [bool; 4],
    last_refresh: Instant,
    last_save: Instant,
    last_render: Instant,
    dirty: bool,
    dirty_pages: Cell<u8>,
    notice_until: Cell<Option<Instant>>,
    folder: PathBuf,
    export: Option<Vec<u8>>,
    connect_candidate: Option<Endpoint>,
}

pub struct Application {
    _app: Rc<RefCell<App>>,
    _timer: slint::Timer,
}
impl Application {
    pub fn new(ui: &AppWindow, runtime: &Handle, folder: PathBuf) -> AppResult<Self> {
        let mut warnings = Vec::new();
        let settings = match Settings::load(&folder.join("settings.json")) {
            Ok(v) => v,
            Err(e) => {
                warnings.push(e.to_string());
                Settings::default()
            }
        };
        let store = match EndpointStore::load(folder.join("endpoints.json")) {
            Ok(v) => v,
            Err(e) => {
                warnings.push(e.to_string());
                EndpointStore::default()
            }
        };
        let translations = Arc::new(Translations::load()?);
        let translated = translations.clone();
        ui.global::<I18n>()
            .on_translate(move |key, lang| translated.text(&key, &lang).into());
        ui.global::<I18n>()
            .set_language(settings.language.clone().into());
        ui.global::<Theme>().set_dark(settings.dark);
        let (tx, mut rx) = mpsc::channel(256);
        let (saves, mut save_rx) = mpsc::channel::<Save>(16);
        let save_folder = folder.clone();
        let saved = tx.clone();
        runtime.spawn(async move {
            let mut failures = Vec::new();
            while let Some(save) = save_rx.recv().await {
                if let Save::Flush(done) = save {
                    let result = if failures.is_empty() {
                        Ok(())
                    } else {
                        Err(std::mem::take(&mut failures).join("; "))
                    };
                    let _ = done.send(result);
                    continue;
                }
                if let Save::LoadUsage(id, token) = save {
                    let path = save_folder.join(format!("usage-{}.json", file_id(&id)));
                    let result = tokio::task::spawn_blocking(
                        move || -> AppResult<BTreeMap<String, data::Usage>> {
                            match settings::read_bounded(&path, 32 * 1024 * 1024) {
                                Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
                                Err(e)
                                    if e.downcast_ref::<std::io::Error>().is_some_and(|e| {
                                        e.kind() == std::io::ErrorKind::NotFound
                                    }) =>
                                {
                                    Ok(BTreeMap::new())
                                }
                                Err(e) => Err(e),
                            }
                        },
                    )
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })
                    .and_then(|v| v);
                    if saved.send(Event::Usage(token, result)).await.is_err() {
                        break;
                    }
                    continue;
                }
                let directory = save_folder.clone();
                let is_export = matches!(save, Save::Export(_, _));
                let result = tokio::task::spawn_blocking(move || -> AppResult<()> {
                    match save {
                        Save::Flush(_) | Save::LoadUsage(_, _) => {}
                        Save::Preferences(store, settings) => {
                            store.save(directory.join("endpoints.json"))?;
                            settings::write_atomic(
                                &directory.join("settings.json"),
                                &serde_json::to_vec_pretty(&settings)?,
                            )?;
                        }
                        Save::Usage(id, values) => settings::write_atomic(
                            &directory.join(format!("usage-{}.json", file_id(&id))),
                            &serde_json::to_vec(&values)?,
                        )?,
                        Save::Export(path, bytes) => settings::write_atomic(&path, &bytes)?,
                    }
                    Ok(())
                })
                .await;
                let failed = !matches!(&result, Ok(Ok(())));
                let message = match result {
                    Ok(Ok(())) if is_export => Some("saved".into()),
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(e.to_string()),
                    Err(e) => Some(e.to_string()),
                };
                if let Some(message) = message {
                    if failed {
                        failures.push(message.clone());
                    }
                    if saved.send(Event::Storage(message)).await.is_err() {
                        break;
                    }
                }
            }
        });
        let app = Rc::new(RefCell::new(App {
            ui: ui.as_weak(),
            runtime: runtime.clone(),
            tx,
            saves,
            logs_inbox: Arc::new(log_inbox::LogInbox::default()),
            session: Session::default(),
            store,
            page: settings.default_page,
            settings,
            draft_settings: BTreeMap::new(),
            translations,
            data: Data::default(),
            usage_ready: false,
            tab: 0,
            filter: 0,
            grouping: 0,
            search: String::new(),
            sort: None,
            descending: false,
            collapsed: BTreeSet::new(),
            config_rows_model: Rc::new(VecModel::default()),
            rows: Rc::new(VecModel::default()),
            groups: Rc::new(VecModel::default()),
            stats: Rc::new(VecModel::default()),
            charts: Rc::new(VecModel::default()),
            pending: None,
            batch_cancel: None,
            snapshot_epoch: 0,
            recovering: false,
            refreshing: false,
            busy: false,
            paused: [false; 7],
            paused_connections: vec![],
            paused_logs: vec![],
            stream_states: [false; 4],
            stream_seen: [false; 4],
            last_refresh: Instant::now(),
            last_save: Instant::now(),
            last_render: Instant::now(),
            dirty: true,
            dirty_pages: Cell::new(0x7f),
            notice_until: Cell::new(None),
            folder,
            export: None,
            connect_candidate: None,
        }));
        {
            let state = app.borrow();
            let view = ui.global::<ViewData>();
            view.set_config_rows(state.config_rows_model.clone().into());
            view.set_rows(state.rows.clone().into());
            view.set_groups(state.groups.clone().into());
            view.set_stats(state.stats.clone().into());
            view.set_charts(state.charts.clone().into());
            view.set_status(state.tr("disconnected").into());
            state.render_endpoints(ui);
            state.render(ui);
            if !warnings.is_empty() {
                state.message(&warnings.join("\n"), true);
            }
        }
        let target = app.clone();
        ui.global::<Actions>()
            .on_connect(move |id, label, url, secret| {
                target.borrow_mut().connect(&id, &label, &url, &secret)
            });
        let target = app.clone();
        ui.global::<Actions>()
            .on_invoke(move |action, key, value| target.borrow_mut().invoke(&action, &key, &value));
        let target = app.clone();
        ui.global::<Actions>().on_navigate(move |page| {
            let mut app = target.borrow_mut();
            app.navigate(page as usize);
        });
        let target = app.clone();
        ui.global::<Actions>().on_search(move |text| {
            let mut app = target.borrow_mut();
            app.search = text.into();
            app.mark_dirty();
        });
        let target = app.clone();
        ui.global::<Actions>().on_tab(move |tab| {
            let mut app = target.borrow_mut();
            app.tab = tab.max(0) as usize;
            app.filter = 0;
            app.grouping = 0;
            app.search.clear();
            app.sort = None;
            app.mark_dirty();
        });
        let target = app.clone();
        ui.global::<Actions>().on_sort(move |index| {
            let mut app = target.borrow_mut();
            let Ok(column) = usize::try_from(index) else {
                return;
            };
            if !app
                .ui
                .upgrade()
                .is_some_and(|ui| column < ui.global::<ViewData>().get_columns().row_count())
            {
                return;
            }
            app.descending = app.sort == Some(column) && !app.descending;
            app.sort = Some(column);
            app.mark_dirty();
        });
        let target = app.clone();
        ui.global::<Actions>().on_resize(move |_| {
            if let Ok(mut app) = target.try_borrow_mut() {
                app.mark_dirty();
            }
        });
        let target = app.clone();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(50),
            move || {
                let mut app = target.borrow_mut();
                for _ in 0..128 {
                    match rx.try_recv() {
                        Ok(event) => app.event(event),
                        Err(_) => break,
                    }
                }
                app.drain_logs();
                if app
                    .notice_until
                    .get()
                    .is_some_and(|until| Instant::now() >= until)
                {
                    app.notice_until.set(None);
                    if let Some(ui) = app.ui.upgrade() {
                        ui.global::<ViewData>().set_message("".into());
                    }
                }
                if app.session.current().is_some()
                    && !app.recovering
                    && !app.busy
                    && app.last_refresh.elapsed() > Duration::from_secs(15)
                {
                    app.refresh();
                }
                if app.last_save.elapsed() > Duration::from_secs(60) {
                    app.save_usage();
                }
                if app.dirty && app.last_render.elapsed() >= Duration::from_millis(100) {
                    if let Some(ui) = app.ui.upgrade() {
                        app.render(&ui);
                    }
                    app.dirty = false;
                    app.last_render = Instant::now();
                }
            },
        );
        let selected = app.borrow().store.selected().cloned();
        if let Some(endpoint) = selected {
            app.borrow_mut().switch(endpoint);
        }
        Ok(Self {
            _app: app,
            _timer: timer,
        })
    }
}
impl Application {
    pub fn shutdown(&self, runtime: &tokio::runtime::Runtime) -> AppResult<()> {
        self._timer.stop();
        let sender = {
            let mut app = self._app.borrow_mut();
            app.save_usage();
            app.session.disconnect();
            app.saves.clone()
        };
        runtime.block_on(async move {
            let (done, finished) = tokio::sync::oneshot::channel();
            sender.send(Save::Flush(done)).await?;
            tokio::time::timeout(Duration::from_secs(10), finished).await???;
            Ok(())
        })
    }
}
fn file_id(id: &str) -> String {
    // Stable filename independent of arbitrary endpoint IDs in imported settings.
    let hash = id.bytes().fold(0xcbf29ce484222325_u64, |hash, b| {
        (hash ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}
fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(items)).into()
}
fn replace<T: Clone + PartialEq + 'static>(model: &VecModel<T>, items: Vec<T>) {
    let count = items.len();
    if count == 0 {
        if model.row_count() != 0 {
            model.set_vec(items);
        }
        return;
    }
    for (index, row) in items.into_iter().enumerate() {
        if index >= model.row_count() {
            model.push(row);
        } else if model.row_data(index).as_ref() != Some(&row) {
            model.set_row_data(index, row);
        }
    }
    while model.row_count() > count {
        model.remove(model.row_count() - 1);
    }
}
impl App {
    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.dirty_pages.set(0x7f);
    }
    fn tr(&self, key: &str) -> String {
        self.translations.text(key, &self.settings.language)
    }
    fn message(&self, message: &str, error: bool) {
        self.notice_until.set(if error {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(4))
        });
        if let Some(ui) = self.ui.upgrade() {
            let view = ui.global::<ViewData>();
            view.set_message(message.into());
            view.set_message_error(error);
        }
    }
    fn error(&self, error: &Error) {
        if error.kind == ErrorKind::Cancelled {
            return;
        }
        let label = match error.kind {
            ErrorKind::Unauthorized => "unauthorized",
            ErrorKind::Transport => "unavailable",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::InvalidInput => "invalidInput",
            _ => "error",
        };
        self.message(&format!("{}\n{}", self.tr(label), error), true);
    }
    fn save(&self) {
        if let Err(e) = self
            .saves
            .try_send(Save::Preferences(self.store.clone(), self.settings.clone()))
        {
            self.message(&e.to_string(), true);
        }
    }
    fn save_usage(&mut self) {
        if !self.usage_ready {
            return;
        }
        self.last_save = Instant::now();
        self.data.prune_usage(self.settings.retention_days);
        if let Some(context) = self.session.current()
            && let Err(e) = self.saves.try_send(Save::Usage(
                context.token.endpoint_id.clone(),
                self.data.usage.clone(),
            ))
        {
            self.message(&e.to_string(), true);
        }
    }
    fn navigate(&mut self, page: usize) {
        if page > 6 {
            return;
        }
        self.page = page;
        self.tab = 0;
        self.filter = 0;
        self.grouping = 0;
        self.search.clear();
        self.sort = None;
        self.mark_dirty();
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ViewData>().set_settings_visible(false);
        }
    }
    fn connect(&mut self, id: &str, label: &str, url: &str, secret: &str) {
        let id = if id.is_empty() {
            format!("{}", chrono::Utc::now().timestamp_micros())
        } else {
            id.into()
        };
        let label = if label.trim().is_empty() {
            url.trim()
        } else {
            label.trim()
        };
        match Endpoint::new(id, label, url.trim(), secret) {
            Ok(endpoint) => self.switch(endpoint),
            Err(e) => self.error(&e),
        }
    }
    fn switch(&mut self, endpoint: Endpoint) {
        self.save_usage();
        match self
            .session
            .switch(endpoint.clone(), ClientOptions::default())
        {
            Ok(_) => {
                self.data = Data::default();
                self.usage_ready = false;
                self.paused = [false; 7];
                self.paused_connections.clear();
                self.paused_logs.clear();
                self.collapsed.clear();
                self.pending = None;
                self.busy = false;
                self.refreshing = false;
                self.stream_states = [false; 4];
                self.stream_seen = [false; 4];
                self.connect_candidate = Some(endpoint);
                if let Some(cancel) = self.batch_cancel.take() {
                    cancel.cancel();
                }
                if let Some(ui) = self.ui.upgrade() {
                    let v = ui.global::<ViewData>();
                    v.set_dialog_visible(false);
                    v.set_settings_visible(false);
                    v.set_connected(false);
                    v.set_testing(false);
                    v.set_dns_result("".into());
                    v.set_message("".into());
                }
                self.recover(None);
            }
            Err(e) => self.error(&e),
        }
    }
    fn recover(&mut self, action: Option<MaintenanceAction>) {
        let operation = match action {
            Some(action) => self.session.maintain(action, RecoveryOptions::default()),
            None => self.session.recover(RecoveryOptions {
                timeout: Duration::from_secs(12),
                ..RecoveryOptions::default()
            }),
        };
        match operation {
            Ok(operation) => {
                self.last_refresh = Instant::now();
                self.recovering = true;
                self.refreshing = false;
                self.busy = action.is_some();
                self.stream_states = [false; 4];
                self.stream_seen = [false; 4];
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ViewData>().set_connecting(true);
                    ui.global::<ViewData>()
                        .set_status(self.tr("connecting").into());
                }
                let tx = self.tx.clone();
                self.runtime.spawn(async move {
                    let _ = tx.send(Event::Recovery(operation.run().await)).await;
                });
            }
            Err(e) => self.error(&e),
        }
        self.mark_dirty();
    }
    fn refresh(&mut self) {
        if self.refreshing || self.recovering {
            return;
        }
        let Some(ctx) = self.session.current().cloned() else {
            return;
        };
        self.snapshot_epoch = self.snapshot_epoch.wrapping_add(1);
        let epoch = self.snapshot_epoch;
        self.refreshing = true;
        self.last_refresh = Instant::now();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let event = ctx.run(snapshot(&ctx.client)).await;
            let _ = tx.send(Event::Snapshot(epoch, event)).await;
        });
    }
    fn streams(&self) {
        let Some(context) = self.session.current() else {
            return;
        };
        self.logs_inbox.activate(context.token.clone());
        let level = [
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warning,
            LogLevel::Error,
            LogLevel::Silent,
        ]
        .get(self.settings.log_level)
        .copied()
        .unwrap_or(LogLevel::Info);
        for kind in [
            StreamKind::Traffic,
            StreamKind::Memory,
            StreamKind::Connections,
            StreamKind::Logs(level),
        ] {
            let event = context.subscribe(
                &self.runtime,
                kind,
                StreamOptions {
                    queue_capacity: 128,
                    ..StreamOptions::default()
                },
            );
            match event.result {
                Ok(mut subscription) => {
                    let tx = self.tx.clone();
                    let logs = self.logs_inbox.clone();
                    self.runtime.spawn(async move {
                        loop {
                            let event = subscription.recv().await;
                            if matches!(kind, StreamKind::Logs(_)) {
                                match &event.result {
                                    Ok(StreamEvent::Data(value)) => {
                                        if let StreamData::Log(log) = value.as_ref() {
                                            logs.push(&event.token, log.clone());
                                            continue;
                                        }
                                    }
                                    Err(error) if error.kind == ErrorKind::Lagged => {
                                        logs.gap(&event.token, error.to_string());
                                        continue;
                                    }
                                    _ => {}
                                }
                            }
                            let finished = event
                                .result
                                .as_ref()
                                .is_err_and(|e| e.kind == ErrorKind::Cancelled);
                            if tx.send(Event::Stream(kind, event)).await.is_err() || finished {
                                break;
                            }
                        }
                    });
                }
                Err(e) => self.error(&e),
            }
        }
    }
    fn drain_logs(&mut self) {
        let Some(batch) = self.logs_inbox.take(self.settings.log_limit) else {
            return;
        };
        if !self.session.accepts(&batch.token) {
            return;
        }
        let retained = batch.logs.len();
        for log in batch.logs {
            self.data.push_log(log, self.settings.log_limit);
        }
        if batch.dropped > 0 || batch.gap.is_some() {
            self.data.push_log(
                models::Log {
                    level: "warning".into(),
                    payload: format!(
                        "{}: {}{}",
                        self.tr("logsSkipped"),
                        batch
                            .dropped
                            .saturating_add(u64::from(retained >= self.settings.log_limit)),
                        batch.gap.map(|v| format!("; {v}")).unwrap_or_default()
                    ),
                },
                self.settings.log_limit,
            );
        }
        self.dirty_pages.set(self.dirty_pages.get() | (1 << 5));
        self.dirty = true;
    }
    fn event(&mut self, event: Event) {
        let pending_pages = self.dirty_pages.get();
        self.mark_dirty();
        match event {
            Event::Usage(token, result) => {
                if !self.session.accepts(&token) {
                    return;
                }
                match result {
                    Ok(mut history) => {
                        for (key, delta) in std::mem::take(&mut self.data.usage) {
                            let entry = history.entry(key).or_insert_with(|| data::Usage {
                                upload: 0,
                                download: 0,
                                count: 0,
                                ..delta.clone()
                            });
                            entry.upload = entry.upload.saturating_add(delta.upload);
                            entry.download = entry.download.saturating_add(delta.download);
                            entry.count = entry.count.saturating_add(delta.count);
                        }
                        self.data.usage = history;
                        self.usage_ready = true;
                        self.data.prune_usage(self.settings.retention_days);
                    }
                    Err(error) => self.message(&error.to_string(), true),
                }
            }
            Event::Recovery(event) => {
                if !self.session.accepts(&event.token) {
                    return;
                }
                self.recovering = false;
                self.busy = false;
                if let Some(ui) = self.ui.upgrade() {
                    ui.global::<ViewData>().set_connecting(false);
                }
                let report = match event.result {
                    Ok(v) => v,
                    Err(e) => {
                        self.error(&e);
                        return;
                    }
                };
                match report.snapshot {
                    Ok(snapshot) => {
                        let version = snapshot.version.version.clone();
                        for error in self
                            .data
                            .apply_snapshot(snapshot, self.settings.connection_limit)
                        {
                            self.error(&error);
                        }
                        if let Some(endpoint) = self.connect_candidate.take() {
                            let id = endpoint.id().to_owned();
                            if let Err(e) = self
                                .store
                                .upsert(endpoint)
                                .and_then(|()| self.store.select(&id))
                            {
                                self.error(&e);
                            } else {
                                self.save();
                            }
                            if let Err(e) = self
                                .saves
                                .try_send(Save::LoadUsage(id, event.token.clone()))
                            {
                                self.message(&e.to_string(), true);
                            }
                        }
                        if let Some(ui) = self.ui.upgrade() {
                            let v = ui.global::<ViewData>();
                            v.set_connected(true);
                            v.set_show_connect(false);
                            v.set_version(format!("Mihomo {version}").into());
                            v.set_status(self.tr("connected").into());
                            self.render_endpoints(&ui);
                        }
                        self.streams();
                        self.last_refresh = Instant::now();
                    }
                    Err(e) => {
                        self.error(&e);
                        let new_connection = self.connect_candidate.take().is_some();
                        if new_connection || e.kind == ErrorKind::Unauthorized {
                            self.session.disconnect();
                        }
                        if let Some(ui) = self.ui.upgrade() {
                            let view = ui.global::<ViewData>();
                            if new_connection || e.kind == ErrorKind::Unauthorized {
                                view.set_connected(false);
                            }
                            view.set_status(self.tr("disconnected").into());
                        }
                    }
                }
                match report.command {
                    Some(CommandOutcome::Acknowledged) => self.message(&self.tr("success"), false),
                    Some(CommandOutcome::ReportedFailure(e)) => self.error(&e),
                    Some(CommandOutcome::Indeterminate(_)) => {
                        self.message(&self.tr("interrupted"), true)
                    }
                    None => {}
                }
            }
            Event::Snapshot(epoch, event) => {
                if epoch != self.snapshot_epoch || !self.session.accepts(&event.token) {
                    return;
                }
                self.refreshing = false;
                match event.result {
                    Ok(mut snapshot) => {
                        snapshot.connections = Ok(None);
                        if let Some(ui) = self.ui.upgrade() {
                            ui.global::<ViewData>()
                                .set_version(format!("Mihomo {}", snapshot.version.version).into());
                        }
                        if self.stream_seen.iter().all(|v| !*v) {
                            self.streams();
                        }
                        for e in self
                            .data
                            .apply_snapshot(snapshot, self.settings.connection_limit)
                        {
                            self.error(&e);
                        }
                    }
                    Err(e) => self.error(&e),
                }
            }
            Event::Stream(kind, event) => {
                let affected = match kind {
                    StreamKind::Traffic | StreamKind::Memory => 1,
                    StreamKind::Connections => 1 | (1 << 3) | (1 << 4),
                    StreamKind::Logs(_) => 1 << 5,
                };
                self.dirty_pages.set(pending_pages | affected);
                if !self.session.accepts(&event.token) {
                    return;
                }
                match event.result {
                    Ok(StreamEvent::Data(value)) => self.data.apply_stream(
                        &value,
                        self.settings.log_limit,
                        self.settings.connection_limit,
                        self.settings.track_traffic,
                    ),
                    Ok(StreamEvent::State(state)) => {
                        let index = match kind {
                            StreamKind::Traffic => 0,
                            StreamKind::Memory => 1,
                            StreamKind::Connections => 2,
                            StreamKind::Logs(_) => 3,
                        };
                        let connected = matches!(state, StreamState::Connected);
                        let reconnect =
                            connected && self.stream_seen[index] && !self.stream_states[index];
                        self.stream_states[index] = connected;
                        self.stream_seen[index] |= connected;
                        if reconnect {
                            self.refresh();
                        }
                        if let Some(ui) = self.ui.upgrade() {
                            ui.global::<ViewData>().set_status(
                                self.tr(if self.stream_states.iter().all(|v| *v) {
                                    "connected"
                                } else {
                                    "reconnecting"
                                })
                                .into(),
                            );
                        }
                    }
                    Ok(StreamEvent::Error(error)) | Err(error) => self.error(&error),
                }
            }
            Event::Command(action, event) => {
                if !self.session.accepts(&event.token) {
                    return;
                }
                self.busy = false;
                match event.result {
                    Ok(value) if action == "dns" => {
                        if let Some(ui) = self.ui.upgrade() {
                            match serde_json::to_string_pretty(&value) {
                                Ok(text) => ui.global::<ViewData>().set_dns_result(text.into()),
                                Err(e) => self.message(&e.to_string(), true),
                            }
                        }
                    }
                    Ok(_) => {
                        self.message(&self.tr("success"), false);
                        self.refresh();
                    }
                    Err(error) => {
                        self.error(&error);
                        self.refresh();
                    }
                }
            }
            Event::Batch(event) => {
                if !self.session.accepts(&event.token) {
                    return;
                }
                match event.result {
                    Ok(BatchEvent::Completed {
                        completed,
                        total,
                        result,
                    }) => {
                        if let Err(error) = &result.result
                            && !matches!(error.kind, ErrorKind::Timeout | ErrorKind::Cancelled)
                        {
                            self.error(error);
                        }
                        let delay = result.result.as_ref().map(|v| v.delay).unwrap_or(0);
                        let proxy = match &result.probe.provider {
                            Some(provider) => {
                                self.data.proxy_providers.get_mut(provider).and_then(|p| {
                                    p.proxies.iter_mut().find(|p| p.name == result.probe.node)
                                })
                            }
                            None => self.data.proxies.get_mut(&result.probe.node),
                        };
                        if let Some(proxy) = proxy {
                            proxy.history.push(models::DelayHistory {
                                time: chrono::Utc::now().to_rfc3339(),
                                delay,
                            });
                        }
                        if let Some(ui) = self.ui.upgrade() {
                            ui.global::<ViewData>()
                                .set_progress(format!("{completed}/{total}").into());
                        }
                    }
                    Ok(BatchEvent::Finished { .. }) => {
                        self.batch_cancel = None;
                        if let Some(ui) = self.ui.upgrade() {
                            ui.global::<ViewData>().set_testing(false);
                        }
                    }
                    Err(e) => self.error(&e),
                }
            }
            Event::Storage(text) => self.message(&self.tr(&text), text != "saved"),
            Event::Imported(kind, result) => self.imported(&kind, result),
        }
        self.dirty = true;
    }
}
async fn snapshot(client: &CoreClient) -> nekodash_core::Result<CoreSnapshot> {
    let version = client.version().await?;
    let (config, proxies, proxy_providers, rules, rule_providers, connections) = tokio::join!(
        client.config(),
        client.proxies(),
        client.proxy_providers(),
        client.rules(),
        client.rule_providers(),
        client.connections()
    );
    Ok(CoreSnapshot {
        version,
        config,
        proxies,
        proxy_providers,
        rules,
        rule_providers,
        connections,
    })
}
