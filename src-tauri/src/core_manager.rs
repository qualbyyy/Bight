use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::Arc,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
    sync::{Notify, oneshot},
    time::timeout,
};
use uuid::Uuid;
use vpn_core::{
    AppProfile, BuildOptions, ClashApiClient, ConfigError, NodeConfig, ProfileMode, TargetPlatform,
    build_config, validate_config,
};

use crate::state::{
    ActiveProcess, ConnectionStatus, LogStream, SharedAppState, StateError, append_log, now_ms,
    save_state_async, update_status,
};

const PROCESS_STOP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectOptions {
    pub node_id: Option<Uuid>,
    pub mode: Option<ProfileMode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectResult {
    pub node_id: Uuid,
    pub mode: ProfileMode,
    pub mixed_port: u16,
    pub clash_port: u16,
}

#[derive(Debug, Error)]
pub enum CoreManagerError {
    #[error("{0}")]
    Message(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("sing-box configuration error: {0}")]
    Config(#[from] ConfigError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Clash API error: {0}")]
    Clash(#[from] vpn_core::ClashApiError),
    #[error("state error: {0}")]
    State(#[from] StateError),
}

struct ChildExit {
    status: ExitStatus,
    requested_stop: bool,
}

pub async fn connect<R: Runtime>(
    app: &AppHandle<R>,
    shared: &SharedAppState,
    options: ConnectOptions,
) -> Result<ConnectResult, CoreManagerError> {
    #[cfg(target_os = "android")]
    {
        let _ = (app, shared, options);
        return Err(CoreManagerError::Message(
            "Android tunnel backend is provided by the Phase 5 VPN plugin".to_string(),
        ));
    }

    let operation_lock = shared.read().await.operation_lock();
    let _operation = operation_lock.lock().await;

    {
        let guard = shared.read().await;
        if guard.active_process.is_some() {
            return Err(CoreManagerError::Message(
                "sing-box is already running".to_string(),
            ));
        }
    }

    set_status(app, shared, ConnectionStatus::Connecting).await;

    match connect_inner(app, shared, options).await {
        Ok(result) => Ok(result),
        Err(error) => {
            append_log(
                app,
                shared,
                LogStream::System,
                format!("connect failed: {error}"),
            )
            .await;
            set_status(
                app,
                shared,
                ConnectionStatus::Error {
                    message: error.to_string(),
                },
            )
            .await;
            Err(error)
        }
    }
}

async fn connect_inner<R: Runtime>(
    app: &AppHandle<R>,
    shared: &SharedAppState,
    options: ConnectOptions,
) -> Result<ConnectResult, CoreManagerError> {
    let (profile, data_dir) = {
        let guard = shared.read().await;
        (guard.profile.clone(), guard.data_dir.clone())
    };
    let mode = options.mode.unwrap_or(profile.mode);
    let node = select_node(&profile, options.node_id.or(profile.active_node_id))?;
    let platform = target_platform()?;
    let binary = resolve_sing_box_binary(app)?;
    validate_platform_requirements(&binary, mode)?;

    let mixed_port = reserve_port()?;
    let clash_port = reserve_port()?;
    let clash_secret = Uuid::new_v4().simple().to_string();
    let config_path = data_dir.join("config.json");
    let cache_path = data_dir.join("cache.db");
    let options = BuildOptions::new(mode, platform)
        .with_ports(mixed_port, clash_port)
        .with_clash_secret(clash_secret.clone())
        .with_cache_file_path(cache_path.to_string_lossy().into_owned());
    let config = build_config(&node, options)?;
    validate_config(&config)?;
    write_json_atomic(&config_path, &config)?;
    run_preflight(app, shared, &binary, &config_path).await?;

    #[cfg(target_os = "windows")]
    let proxy_resource = if mode == ProfileMode::SystemProxy {
        Some(enable_windows_system_proxy(&data_dir, mixed_port)?)
    } else {
        None
    };

    #[cfg(not(target_os = "windows"))]
    let proxy_resource: Option<Arc<dyn Send + Sync>> = None;

    let mut command = Command::new(&binary);
    command
        .arg("run")
        .arg("-c")
        .arg(&config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(parent) = binary.parent() {
        command.current_dir(parent);
    }
    let mut child = command.spawn().map_err(|error| {
        CoreManagerError::Message(format!(
            "could not start sing-box at {}: {error}",
            binary.display()
        ))
    })?;
    let pid = child.id().ok_or_else(|| {
        CoreManagerError::Message("sing-box did not expose a process ID".to_string())
    })?;

    #[cfg(target_os = "windows")]
    let job_resource = match create_windows_job(&child) {
        Ok(job) => Some(job),
        Err(error) => {
            let _ = child.kill().await;
            return Err(error);
        }
    };

    #[cfg(not(target_os = "windows"))]
    let job_resource: Option<Arc<dyn Send + Sync>> = None;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (stop_tx, stop_rx) = oneshot::channel();
    let finished = Arc::new(Notify::new());
    let mut resources: Vec<Arc<dyn Send + Sync>> = Vec::new();
    if let Some(resource) = proxy_resource {
        resources.push(resource);
    }
    if let Some(resource) = job_resource {
        resources.push(resource);
    }
    let active_process = ActiveProcess::new(pid, stop_tx, finished.clone(), resources);

    if let Some(stdout) = stdout {
        spawn_log_reader(app.clone(), shared.clone(), LogStream::Stdout, stdout);
    }
    if let Some(stderr) = stderr {
        spawn_log_reader(app.clone(), shared.clone(), LogStream::Stderr, stderr);
    }

    let clash_api = ClashApiClient::new(clash_port, Some(clash_secret))?;
    {
        let mut guard = shared.write().await;
        guard.profile.active_node_id = Some(node.id);
        guard.profile.mode = mode;
        guard.profile.updated_at_ms = now_ms();
        guard.profile.is_deleted = false;
        guard.active_process = Some(active_process);
        guard.clash_api = Some(clash_api);
    }
    spawn_process_watcher(app.clone(), shared.clone(), pid, child, stop_rx, finished);
    if let Err(error) = save_state_async(shared).await {
        let _ = disconnect(app, shared).await;
        return Err(error.into());
    }

    let result = ConnectResult {
        node_id: node.id,
        mode,
        mixed_port,
        clash_port,
    };
    set_status(
        app,
        shared,
        ConnectionStatus::Connected {
            node_id: result.node_id,
            clash_port: result.clash_port,
            mode: result.mode,
        },
    )
    .await;
    append_log(
        app,
        shared,
        LogStream::System,
        format!("sing-box started with PID {}", pid),
    )
    .await;
    Ok(result)
}

pub async fn disconnect<R: Runtime>(
    app: &AppHandle<R>,
    shared: &SharedAppState,
) -> Result<(), CoreManagerError> {
    let operation_lock = shared.read().await.operation_lock();
    let _operation = operation_lock.lock().await;
    let process = {
        let mut guard = shared.write().await;
        guard.clash_api = None;
        guard.take_active_process()
    };

    let Some(process) = process else {
        set_status(app, shared, ConnectionStatus::Disconnected).await;
        return Ok(());
    };

    let pid = process.pid();
    let result = process.stop().await;
    if let Err(error) = result {
        set_status(
            app,
            shared,
            ConnectionStatus::Error {
                message: error.clone(),
            },
        )
        .await;
        return Err(CoreManagerError::Message(error));
    }
    append_log(
        app,
        shared,
        LogStream::System,
        format!("sing-box process {} stopped", pid),
    )
    .await;
    set_status(app, shared, ConnectionStatus::Disconnected).await;
    Ok(())
}

pub async fn set_mode<R: Runtime>(
    app: &AppHandle<R>,
    shared: &SharedAppState,
    mode: ProfileMode,
) -> Result<(), CoreManagerError> {
    let operation_lock = shared.read().await.operation_lock();
    let _operation = operation_lock.lock().await;
    {
        let mut guard = shared.write().await;
        if guard.active_process.is_some() {
            return Err(CoreManagerError::Message(
                "disconnect sing-box before changing the connection mode".to_string(),
            ));
        }
        guard.profile.mode = mode;
        guard.profile.updated_at_ms = now_ms();
    }
    save_state_async(shared).await?;
    let _ = app.emit("profile-mode-changed", mode);
    Ok(())
}

pub async fn clash_client(shared: &SharedAppState) -> Result<ClashApiClient, CoreManagerError> {
    shared
        .read()
        .await
        .clash_api
        .clone()
        .ok_or_else(|| CoreManagerError::Message("sing-box is not connected".to_string()))
}

async fn set_status<R: Runtime>(
    app: &AppHandle<R>,
    shared: &SharedAppState,
    status: ConnectionStatus,
) {
    update_status(app, shared, status.clone()).await;
    crate::tray::refresh_status(&status);
}

pub async fn cleanup_stale_system_proxy<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), CoreManagerError> {
    #[cfg(target_os = "windows")]
    {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| CoreManagerError::Message(error.to_string()))?;
        cleanup_windows_system_proxy(&data_dir)?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
    }
    Ok(())
}

fn select_node(
    profile: &AppProfile,
    requested: Option<Uuid>,
) -> Result<NodeConfig, CoreManagerError> {
    let requested = requested.or_else(|| {
        profile
            .custom_nodes
            .iter()
            .find(|node| !node.is_deleted)
            .map(|node| node.id)
            .or_else(|| {
                profile
                    .subscriptions
                    .iter()
                    .filter(|subscription| !subscription.is_deleted)
                    .flat_map(|subscription| subscription.nodes.iter())
                    .find(|node| !node.is_deleted)
                    .map(|node| node.id)
            })
    });
    let Some(node_id) = requested else {
        return Err(CoreManagerError::Message(
            "no active proxy node is configured".to_string(),
        ));
    };

    profile
        .custom_nodes
        .iter()
        .chain(
            profile
                .subscriptions
                .iter()
                .filter(|subscription| !subscription.is_deleted)
                .flat_map(|subscription| subscription.nodes.iter()),
        )
        .find(|node| node.id == node_id && !node.is_deleted)
        .cloned()
        .ok_or_else(|| CoreManagerError::Message(format!("proxy node {node_id} was not found")))
}

fn reserve_port() -> Result<u16, CoreManagerError> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

fn target_platform() -> Result<TargetPlatform, CoreManagerError> {
    #[cfg(target_os = "linux")]
    {
        return Ok(TargetPlatform::Linux);
    }
    #[cfg(target_os = "windows")]
    {
        return Ok(TargetPlatform::Windows);
    }
    #[cfg(target_os = "android")]
    {
        return Ok(TargetPlatform::Android);
    }
    #[allow(unreachable_code)]
    Err(CoreManagerError::Message(
        "this platform is not supported by the Phase 3 desktop backend".to_string(),
    ))
}

fn resolve_sing_box_binary<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, CoreManagerError> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("QONNECT_SING_BOX") {
        candidates.push(PathBuf::from(path));
    }

    let target = option_env!("TAURI_ENV_TARGET_TRIPLE").unwrap_or(std::env::consts::ARCH);
    let binary_name = if cfg!(target_os = "windows") {
        "sing-box.exe"
    } else {
        "sing-box"
    };
    let sidecar_name = if cfg!(target_os = "windows") {
        format!("sing-box-{target}.exe")
    } else {
        format!("sing-box-{target}")
    };
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("binaries").join(&sidecar_name));
        candidates.push(resource_dir.join("binaries").join(binary_name));
        candidates.push(resource_dir.join(&sidecar_name));
        candidates.push(resource_dir.join(binary_name));
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        candidates.push(parent.join(&sidecar_name));
        candidates.push(parent.join(binary_name));
        candidates.push(parent.join("resources").join(&sidecar_name));
        candidates.push(parent.join("resources").join(binary_name));
    }
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        candidates.push(directory.join(binary_name));
    }

    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            CoreManagerError::Message(format!(
                "sing-box was not found; set QONNECT_SING_BOX or install {binary_name}"
            ))
        })
}

fn validate_platform_requirements(
    binary: &Path,
    mode: ProfileMode,
) -> Result<(), CoreManagerError> {
    if mode != ProfileMode::Tun {
        #[cfg(target_os = "windows")]
        if mode == ProfileMode::SystemProxy {
            return Ok(());
        }
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        ensure_linux_tun_capability(binary)?;
    }
    #[cfg(target_os = "windows")]
    {
        ensure_windows_tun_requirements(binary)?;
    }
    #[cfg(target_os = "android")]
    {
        let _ = binary;
        return Err(CoreManagerError::Message(
            "Android TUN requires the Phase 5 VpnService backend".to_string(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_linux_tun_capability(binary: &Path) -> Result<(), CoreManagerError> {
    let output = std::process::Command::new("getcap")
        .arg(binary)
        .output()
        .map_err(|error| {
            CoreManagerError::Message(format!(
                "cannot verify CAP_NET_ADMIN: getcap is unavailable ({error})"
            ))
        })?;
    let capabilities = String::from_utf8_lossy(&output.stdout);
    if capabilities.contains("cap_net_admin") {
        return Ok(());
    }
    Err(CoreManagerError::Message(format!(
        "TUN mode requires CAP_NET_ADMIN on {}; grant it once with: pkexec setcap cap_net_admin,cap_net_bind_service,cap_dac_read_search=+ep {}",
        binary.display(),
        binary.display()
    )))
}

#[cfg(target_os = "windows")]
fn ensure_windows_tun_requirements(binary: &Path) -> Result<(), CoreManagerError> {
    use windows_sys::Win32::UI::Shell::IsUserAnAdmin;

    if unsafe { IsUserAnAdmin() } == 0 {
        return Err(CoreManagerError::Message(
            "TUN mode on Windows requires administrator privileges; restart Qonnect as administrator or use System Proxy".to_string(),
        ));
    }
    let wintun = binary
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("wintun.dll");
    if !wintun.is_file() {
        return Err(CoreManagerError::Message(format!(
            "TUN mode requires wintun.dll next to {}; expected {}",
            binary.display(),
            wintun.display()
        )));
    }
    Ok(())
}

async fn run_preflight<R: Runtime>(
    app: &AppHandle<R>,
    shared: &SharedAppState,
    binary: &Path,
    config: &Path,
) -> Result<(), CoreManagerError> {
    let output = Command::new(binary)
        .arg("check")
        .arg("-c")
        .arg(config)
        .output()
        .await?;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        append_log(app, shared, LogStream::Stdout, line.to_string()).await;
    }
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        append_log(app, shared, LogStream::Stderr, line.to_string()).await;
    }
    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CoreManagerError::Message(format!(
            "sing-box pre-flight check failed{}",
            if details.is_empty() {
                String::new()
            } else {
                format!(": {details}")
            }
        )));
    }
    Ok(())
}

fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<(), CoreManagerError> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let parent = path.parent().ok_or_else(|| {
        CoreManagerError::Message("configuration path has no parent directory".to_string())
    })?;
    fs::create_dir_all(parent)?;
    let temp = path.with_file_name("config.json.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    crate::state::atomic_replace(&temp, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn spawn_log_reader<R, T>(app: AppHandle<R>, shared: SharedAppState, stream: LogStream, reader: T)
where
    R: Runtime,
    T: AsyncRead + Unpin + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            append_log(&app, &shared, stream.clone(), line).await;
        }
    });
}

fn spawn_process_watcher<R: Runtime>(
    app: AppHandle<R>,
    shared: SharedAppState,
    pid: u32,
    child: Child,
    stop_rx: oneshot::Receiver<()>,
    finished: Arc<Notify>,
) {
    tauri::async_runtime::spawn(async move {
        let exit = wait_for_child(child, stop_rx).await;
        finished.notify_waiters();
        let status = match exit {
            Ok(exit) if exit.requested_stop => ConnectionStatus::Disconnected,
            Ok(exit) if exit.status.success() => ConnectionStatus::Disconnected,
            Ok(exit) => ConnectionStatus::Error {
                message: format!("sing-box exited with status {}", exit.status),
            },
            Err(error) => ConnectionStatus::Error {
                message: format!("sing-box process watcher failed: {error}"),
            },
        };
        let should_emit = {
            let mut guard = shared.write().await;
            if guard
                .active_process
                .as_ref()
                .is_some_and(|process| process.pid() == pid)
            {
                guard.active_process = None;
                guard.clash_api = None;
                guard.status = status.clone();
                true
            } else {
                false
            }
        };
        if should_emit {
            let _ = app.emit("connection-status", status.clone());
            crate::tray::refresh_status(&status);
            append_log(
                &app,
                &shared,
                LogStream::System,
                format!("sing-box process {} exited", pid),
            )
            .await;
        }
    });
}

async fn wait_for_child(
    mut child: Child,
    mut stop_rx: oneshot::Receiver<()>,
) -> Result<ChildExit, io::Error> {
    tokio::select! {
        result = child.wait() => Ok(ChildExit { status: result?, requested_stop: false }),
        _ = &mut stop_rx => {
            let status = terminate_child(&mut child).await?;
            Ok(ChildExit { status, requested_stop: true })
        }
    }
}

async fn terminate_child(child: &mut Child) -> Result<ExitStatus, io::Error> {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        let _ = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }

    #[cfg(windows)]
    child.start_kill()?;

    match timeout(PROCESS_STOP_TIMEOUT, child.wait()).await {
        Ok(result) => result,
        Err(_) => {
            child.kill().await?;
            child.wait().await
        }
    }
}

#[cfg(windows)]
struct WindowsJob {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
unsafe impl Send for WindowsJob {}

#[cfg(windows)]
unsafe impl Sync for WindowsJob {}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                let _ = windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(windows)]
fn create_windows_job(child: &Child) -> Result<Arc<dyn Send + Sync>, CoreManagerError> {
    use std::mem::size_of;
    use windows_sys::Win32::{
        Foundation::HANDLE,
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
    };

    let process_handle = child.raw_handle().ok_or_else(|| {
        CoreManagerError::Message("could not get sing-box Windows process handle".to_string())
    })? as HANDLE;
    let job_handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job_handle.is_null() {
        return Err(CoreManagerError::Io(io::Error::last_os_error()));
    }
    let job = Arc::new(WindowsJob { handle: job_handle });
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            job_handle,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        return Err(CoreManagerError::Io(io::Error::last_os_error()));
    }
    let assigned = unsafe { AssignProcessToJobObject(job_handle, process_handle) };
    if assigned == 0 {
        return Err(CoreManagerError::Io(io::Error::last_os_error()));
    }
    Ok(job)
}

#[cfg(windows)]
#[derive(Debug, Serialize, Deserialize)]
struct WindowsProxyMarker {
    port: u16,
    previous_enabled: u32,
    previous_server: Option<String>,
}

#[cfg(windows)]
struct WindowsProxyGuard {
    data_dir: PathBuf,
    port: u16,
}

#[cfg(windows)]
unsafe impl Send for WindowsProxyGuard {}

#[cfg(windows)]
unsafe impl Sync for WindowsProxyGuard {}

#[cfg(windows)]
impl Drop for WindowsProxyGuard {
    fn drop(&mut self) {
        let _ = disable_windows_system_proxy(&self.data_dir, self.port);
    }
}

#[cfg(windows)]
fn proxy_marker_path(data_dir: &Path) -> PathBuf {
    data_dir.join("system-proxy.json")
}

#[cfg(windows)]
fn enable_windows_system_proxy(
    data_dir: &Path,
    port: u16,
) -> Result<Arc<dyn Send + Sync>, CoreManagerError> {
    let key = open_internet_settings()?;
    let previous_enabled = read_registry_dword(key.handle, "ProxyEnable")?.unwrap_or(0);
    let previous_server = read_registry_string(key.handle, "ProxyServer")?;
    set_registry_string(key.handle, "ProxyServer", &format!("127.0.0.1:{port}"))?;
    set_registry_dword(key.handle, "ProxyEnable", 1)?;
    notify_internet_settings()?;
    drop(key);

    let marker = WindowsProxyMarker {
        port,
        previous_enabled,
        previous_server,
    };
    write_json_atomic(
        &proxy_marker_path(data_dir),
        &serde_json::to_value(&marker)?,
    )?;
    Ok(Arc::new(WindowsProxyGuard {
        data_dir: data_dir.to_path_buf(),
        port,
    }))
}

#[cfg(windows)]
fn cleanup_windows_system_proxy(data_dir: &Path) -> Result<(), CoreManagerError> {
    let marker_path = proxy_marker_path(data_dir);
    if !marker_path.is_file() {
        return Ok(());
    }
    let marker: WindowsProxyMarker = serde_json::from_slice(&fs::read(&marker_path)?)?;
    disable_windows_system_proxy(data_dir, marker.port)
}

#[cfg(windows)]
fn disable_windows_system_proxy(data_dir: &Path, port: u16) -> Result<(), CoreManagerError> {
    let marker_path = proxy_marker_path(data_dir);
    let marker = match fs::read(&marker_path) {
        Ok(bytes) => serde_json::from_slice::<WindowsProxyMarker>(&bytes).ok(),
        Err(_) => None,
    };
    let expected = format!("127.0.0.1:{port}");
    let key = open_internet_settings()?;
    let current_server = read_registry_string(key.handle, "ProxyServer")?;
    if current_server.as_deref() == Some(expected.as_str()) {
        if let Some(marker) = marker {
            match marker.previous_server {
                Some(server) => set_registry_string(key.handle, "ProxyServer", &server)?,
                None => delete_registry_value(key.handle, "ProxyServer")?,
            }
            set_registry_dword(key.handle, "ProxyEnable", marker.previous_enabled)?;
        } else {
            set_registry_dword(key.handle, "ProxyEnable", 0)?;
            delete_registry_value(key.handle, "ProxyServer")?;
        }
        notify_internet_settings()?;
    }
    let _ = fs::remove_file(marker_path);
    Ok(())
}

#[cfg(windows)]
struct RegistryKey {
    handle: windows_sys::Win32::System::Registry::HKEY,
}

#[cfg(windows)]
impl Drop for RegistryKey {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                let _ = windows_sys::Win32::System::Registry::RegCloseKey(self.handle);
            }
        }
    }
}

#[cfg(windows)]
fn open_internet_settings() -> Result<RegistryKey, CoreManagerError> {
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, RegOpenKeyExW,
    };
    let path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");
    let mut handle = std::ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut handle,
        )
    };
    if status != 0 {
        return Err(CoreManagerError::Io(io::Error::from_raw_os_error(
            status as i32,
        )));
    }
    Ok(RegistryKey { handle })
}

#[cfg(windows)]
fn read_registry_dword(
    key: windows_sys::Win32::System::Registry::HKEY,
    name: &str,
) -> Result<Option<u32>, CoreManagerError> {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{REG_DWORD, RegQueryValueExW},
    };
    let name = wide(name);
    let mut kind = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let mut value = 0_u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            &mut kind,
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(CoreManagerError::Io(io::Error::from_raw_os_error(
            status as i32,
        )));
    }
    if kind != REG_DWORD || size < std::mem::size_of::<u32>() as u32 {
        return Err(CoreManagerError::Message(format!(
            "registry value {name:?} is not a DWORD"
        )));
    }
    Ok(Some(value))
}

#[cfg(windows)]
fn read_registry_string(
    key: windows_sys::Win32::System::Registry::HKEY,
    name: &str,
) -> Result<Option<String>, CoreManagerError> {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{REG_SZ, RegQueryValueExW},
    };
    let name_w = wide(name);
    let mut kind = 0;
    let mut size = 0_u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            name_w.as_ptr(),
            std::ptr::null(),
            &mut kind,
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(CoreManagerError::Io(io::Error::from_raw_os_error(
            status as i32,
        )));
    }
    if kind != REG_SZ {
        return Err(CoreManagerError::Message(format!(
            "registry value {name:?} is not a string"
        )));
    }
    let mut bytes = vec![0_u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            key,
            name_w.as_ptr(),
            std::ptr::null(),
            &mut kind,
            bytes.as_mut_ptr(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(CoreManagerError::Io(io::Error::from_raw_os_error(
            status as i32,
        )));
    }
    let units =
        unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u16>(), size as usize / 2) };
    Ok(Some(
        String::from_utf16_lossy(units)
            .trim_end_matches('\0')
            .to_string(),
    ))
}

#[cfg(windows)]
fn set_registry_dword(
    key: windows_sys::Win32::System::Registry::HKEY,
    name: &str,
    value: u32,
) -> Result<(), CoreManagerError> {
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{REG_DWORD, RegSetValueExW},
    };
    let name = wide(name);
    let status = unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_DWORD,
            (&value as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(CoreManagerError::Io(io::Error::from_raw_os_error(
            status as i32,
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn set_registry_string(
    key: windows_sys::Win32::System::Registry::HKEY,
    name: &str,
    value: &str,
) -> Result<(), CoreManagerError> {
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{REG_SZ, RegSetValueExW},
    };
    let name = wide(name);
    let value = wide(value);
    let status = unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr().cast(),
            (value.len() * std::mem::size_of::<u16>()) as u32,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(CoreManagerError::Io(io::Error::from_raw_os_error(
            status as i32,
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn delete_registry_value(
    key: windows_sys::Win32::System::Registry::HKEY,
    name: &str,
) -> Result<(), CoreManagerError> {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::RegDeleteValueW,
    };
    let name = wide(name);
    let status = unsafe { RegDeleteValueW(key, name.as_ptr()) };
    if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
        return Err(CoreManagerError::Io(io::Error::from_raw_os_error(
            status as i32,
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn notify_internet_settings() -> Result<(), CoreManagerError> {
    use windows_sys::Win32::Networking::WinInet::{
        INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED, InternetSetOptionW,
    };
    let first = unsafe {
        InternetSetOptionW(
            std::ptr::null(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null(),
            0,
        )
    };
    let second = unsafe {
        InternetSetOptionW(
            std::ptr::null(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null(),
            0,
        )
    };
    if first == 0 || second == 0 {
        return Err(CoreManagerError::Io(io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
