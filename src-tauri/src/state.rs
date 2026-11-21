use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use thiserror::Error;
use tokio::{
    sync::{Notify, RwLock, oneshot},
    time::{Duration, timeout},
};
use uuid::Uuid;
use vpn_core::{AppProfile, ClashApiClient, ProfileMode};
use vpn_sync::RunningSyncHost;

pub type SharedAppState = Arc<RwLock<AppState>>;

const PROFILE_FILE: &str = "profile.json";
const PROFILE_BACKUP_FILE: &str = "profile.json.bak";
const PROFILE_TEMP_FILE: &str = "profile.json.tmp";
const MAX_LOG_LINES: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected {
        node_id: Uuid,
        clash_port: u16,
        mode: ProfileMode,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp_ms: i64,
    pub stream: LogStream,
    pub line: String,
}

#[derive(Debug, Default)]
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
}

impl LogBuffer {
    pub fn push(&mut self, entry: LogEntry) {
        if self.entries.len() == MAX_LOG_LINES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn snapshot(&self, limit: Option<usize>) -> Vec<LogEntry> {
        let requested = limit.unwrap_or(MAX_LOG_LINES).min(MAX_LOG_LINES);
        self.entries
            .iter()
            .rev()
            .take(requested)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

pub struct ActiveProcess {
    pid: u32,
    stop_signal: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    finished: Arc<Notify>,
    _resources: Vec<Arc<dyn Send + Sync>>,
}

impl ActiveProcess {
    pub(crate) fn new(
        pid: u32,
        stop_signal: oneshot::Sender<()>,
        finished: Arc<Notify>,
        resources: Vec<Arc<dyn Send + Sync>>,
    ) -> Self {
        Self {
            pid,
            stop_signal: Arc::new(Mutex::new(Some(stop_signal))),
            finished,
            _resources: resources,
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub async fn stop(self) -> Result<(), String> {
        send_stop_signal(&self.stop_signal);
        timeout(Duration::from_secs(6), self.finished.notified())
            .await
            .map_err(|_| {
                format!(
                    "sing-box process {} did not stop within 6 seconds",
                    self.pid
                )
            })?;
        Ok(())
    }
}

impl Drop for ActiveProcess {
    fn drop(&mut self) {
        send_stop_signal(&self.stop_signal);
    }
}

fn send_stop_signal(signal: &Arc<Mutex<Option<oneshot::Sender<()>>>>) {
    if let Ok(mut guard) = signal.lock()
        && let Some(sender) = guard.take()
    {
        let _ = sender.send(());
    }
}

#[derive(Default)]
pub struct SyncRuntimeState {
    pub(crate) host: Option<RunningSyncHost>,
    pub(crate) session_id: Option<Uuid>,
    pub(crate) expires_at_ms: Option<i64>,
}

pub struct AppState {
    pub profile: AppProfile,
    pub status: ConnectionStatus,
    pub active_process: Option<ActiveProcess>,
    pub clash_api: Option<ClashApiClient>,
    pub sync: SyncRuntimeState,
    pub logs: LogBuffer,
    pub data_dir: PathBuf,
    pub(crate) operation_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("Tauri path error: {0}")]
    TauriPath(String),
    #[error("profile I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("profile JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("profile is corrupted and backup could not be loaded: {0}")]
    CorruptBackup(String),
}

impl AppState {
    pub fn new(profile: AppProfile, data_dir: PathBuf) -> Self {
        Self {
            profile,
            status: ConnectionStatus::Disconnected,
            active_process: None,
            clash_api: None,
            sync: SyncRuntimeState::default(),
            logs: LogBuffer::default(),
            data_dir,
            operation_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn profile_path(&self) -> PathBuf {
        self.data_dir.join(PROFILE_FILE)
    }

    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn cache_path(&self) -> PathBuf {
        self.data_dir.join("cache.db")
    }

    pub fn operation_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.operation_lock.clone()
    }

    pub fn take_active_process(&mut self) -> Option<ActiveProcess> {
        self.active_process.take()
    }

    pub fn take_sync_host(&mut self) -> Option<RunningSyncHost> {
        self.sync.session_id = None;
        self.sync.expires_at_ms = None;
        self.sync.host.take()
    }

    pub fn has_sync_session(&self, session_id: Uuid) -> bool {
        self.sync.session_id == Some(session_id) && self.sync.host.is_some()
    }
}

pub fn default_profile() -> AppProfile {
    AppProfile {
        id: Uuid::new_v4(),
        name: "Default".to_string(),
        subscriptions: Vec::new(),
        custom_nodes: Vec::new(),
        routing_rules: Vec::new(),
        active_node_id: None,
        mode: ProfileMode::SystemProxy,
        updated_at_ms: now_ms(),
        is_deleted: false,
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_millis()
        .try_into()
        .expect("Unix timestamp does not fit in i64")
}

pub fn load_profile_from_path(path: &Path) -> Result<AppProfile, StateError> {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<AppProfile>(&bytes) {
            Ok(profile) => Ok(profile),
            Err(primary_error) => load_backup(path, primary_error),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => load_missing_primary(path),
        Err(error) => Err(error.into()),
    }
}

fn load_missing_primary(path: &Path) -> Result<AppProfile, StateError> {
    let backup = backup_path(path);
    match fs::read(backup) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(default_profile()),
        Err(error) => Err(error.into()),
    }
}

fn load_backup(path: &Path, primary_error: serde_json::Error) -> Result<AppProfile, StateError> {
    let backup = backup_path(path);
    match fs::read(&backup) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|backup_error| {
            StateError::CorruptBackup(format!("primary: {primary_error}; backup: {backup_error}"))
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(StateError::CorruptBackup(
            format!("primary: {primary_error}; backup is missing"),
        )),
        Err(error) => Err(StateError::CorruptBackup(format!(
            "primary: {primary_error}; backup I/O: {error}"
        ))),
    }
}

pub fn save_profile_atomic(path: &Path, profile: &AppProfile) -> Result<(), StateError> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "profile path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;

    let temp_path = path.with_file_name(PROFILE_TEMP_FILE);
    let backup = backup_path(path);
    let bytes = serde_json::to_vec_pretty(profile)?;
    let mut temp = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp_path)?;
    temp.write_all(&bytes)?;
    temp.sync_all()?;
    drop(temp);

    if path.exists() {
        fs::copy(path, &backup)?;
        File::open(&backup)?.sync_all()?;
    }

    atomic_replace(&temp_path, path)?;
    sync_directory(parent)?;
    Ok(())
}

pub async fn save_state_async(shared: &SharedAppState) -> Result<(), StateError> {
    let (path, profile) = {
        let guard = shared.read().await;
        (guard.profile_path(), guard.profile.clone())
    };
    tokio::task::spawn_blocking(move || save_profile_atomic(&path, &profile))
        .await
        .map_err(|error| io::Error::other(error.to_string()))??;
    Ok(())
}

pub fn load_app_state<R: Runtime>(app: &AppHandle<R>) -> Result<SharedAppState, StateError> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| StateError::TauriPath(error.to_string()))?;
    let profile = load_profile_from_path(&data_dir.join(PROFILE_FILE))?;
    Ok(Arc::new(RwLock::new(AppState::new(profile, data_dir))))
}

pub async fn append_log<R: Runtime>(
    app: &AppHandle<R>,
    shared: &SharedAppState,
    stream: LogStream,
    line: impl Into<String>,
) {
    let entry = LogEntry {
        timestamp_ms: now_ms(),
        stream,
        line: line.into(),
    };
    {
        let mut guard = shared.write().await;
        guard.logs.push(entry.clone());
    }
    let _ = app.emit("sing-box-log", entry);
}

pub async fn update_status<R: Runtime>(
    app: &AppHandle<R>,
    shared: &SharedAppState,
    status: ConnectionStatus,
) {
    {
        let mut guard = shared.write().await;
        guard.status = status.clone();
    }
    let _ = app.emit("connection-status", status);
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_file_name(PROFILE_BACKUP_FILE)
}

pub(crate) fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };

        let source = wide_path(source);
        let destination = wide_path(destination);
        let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
        let result = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        return Ok(());

        fn wide_path(path: &Path) -> Vec<u16> {
            OsStr::new(path)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }
    }

    #[cfg(not(windows))]
    {
        fs::rename(source, destination)
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}
