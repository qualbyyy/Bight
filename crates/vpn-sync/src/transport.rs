use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{Mutex, oneshot},
    task::JoinHandle,
};
use tokio_tungstenite::{connect_async, tungstenite::Message as TungsteniteMessage};
use uuid::Uuid;

use crate::discovery::MdnsRegistration;
use crate::{
    CryptoError, EncryptedEnvelope, EphemeralKeyPair, MergeError, PairingError, PairingPayload,
    SyncPayload, SyncSession, merge_profiles, now_ms,
};
use vpn_core::AppProfile;

const HEALTH_PATH: &str = "/v1/health";
const SYNC_PATH: &str = "/v1/sync";
const WEBSOCKET_PATH: &str = "/v1/ws";

#[derive(Debug, Error)]
pub enum SyncTransportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("pairing error: {0}")]
    Pairing(#[from] PairingError),
    #[error("cryptographic error: {0}")]
    Crypto(#[from] CryptoError),
    #[error("merge error: {0}")]
    Merge(#[from] MergeError),
    #[error("HTTP client error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("request timed out")]
    Timeout,
    #[error("remote returned HTTP {status}: {message}")]
    Remote { status: u16, message: String },
    #[error("sync session was already used")]
    SessionUsed,
    #[error("server task failed: {0}")]
    ServerTask(String),
    #[error("invalid sync request: {0}")]
    InvalidRequest(String),
}

#[derive(Debug, Clone)]
pub struct SyncHostConfig {
    pub bind_addr: SocketAddr,
    pub session_ttl: Duration,
    pub ips: Vec<IpAddr>,
    pub advertise_mdns: bool,
    pub instance_name: String,
}

impl SyncHostConfig {
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            session_ttl: crate::DEFAULT_SESSION_TTL,
            ips: Vec::new(),
            advertise_mdns: false,
            instance_name: "qonnect".to_string(),
        }
    }

    pub fn with_ips(mut self, ips: Vec<IpAddr>) -> Self {
        self.ips = ips;
        self
    }

    pub fn with_session_ttl(mut self, session_ttl: Duration) -> Self {
        self.session_ttl = session_ttl;
        self
    }

    pub fn with_mdns(mut self, instance_name: impl Into<String>) -> Self {
        self.advertise_mdns = true;
        self.instance_name = instance_name.into();
        self
    }
}

#[derive(Debug, Clone)]
pub struct SyncClientConfig {
    pub timeout: Duration,
}

impl Default for SyncClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncRequest {
    session_id: Uuid,
    pin: String,
    client_public_key: String,
    envelope: EncryptedEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncResponse {
    envelope: EncryptedEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthBody {
    pub ok: bool,
    pub session_id: Uuid,
    pub host_time_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct HealthQuery {
    sid: Uuid,
}

#[derive(Clone)]
struct HostState {
    profile: Arc<Mutex<AppProfile>>,
    session: Arc<SyncSession>,
    used: Arc<AtomicBool>,
}

pub struct SyncHost;

pub struct RunningSyncHost {
    address: SocketAddr,
    pairing: PairingPayload,
    profile: Arc<Mutex<AppProfile>>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), std::io::Error>>>,
    mdns: Option<MdnsRegistration>,
}

impl SyncHost {
    pub async fn start(
        profile: AppProfile,
        config: SyncHostConfig,
    ) -> Result<RunningSyncHost, SyncTransportError> {
        let listener = TcpListener::bind(config.bind_addr).await?;
        let address = listener.local_addr()?;
        let ips = if config.ips.is_empty() {
            if address.ip().is_unspecified() {
                return Err(SyncTransportError::InvalidRequest(
                    "host IP list is required when binding to an unspecified address".to_string(),
                ));
            }
            vec![address.ip()]
        } else {
            config.ips.clone()
        };
        let session = Arc::new(SyncSession::try_new(config.session_ttl)?);
        let pairing = session.pairing_payload(ips, address.port());
        let profile = Arc::new(Mutex::new(profile));
        let state = HostState {
            profile: profile.clone(),
            session,
            used: Arc::new(AtomicBool::new(false)),
        };
        let app = Router::new()
            .route(HEALTH_PATH, get(health_handler))
            .route(SYNC_PATH, post(sync_handler))
            .route(WEBSOCKET_PATH, get(websocket_handler))
            .with_state(state);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
        });
        let mdns = if config.advertise_mdns {
            Some(
                MdnsRegistration::register(&pairing, &config.instance_name).map_err(|error| {
                    SyncTransportError::InvalidRequest(format!("mDNS registration failed: {error}"))
                })?,
            )
        } else {
            None
        };
        Ok(RunningSyncHost {
            address,
            pairing,
            profile,
            shutdown: Some(shutdown_tx),
            task: Some(task),
            mdns,
        })
    }
}

impl RunningSyncHost {
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn pairing(&self) -> &PairingPayload {
        &self.pairing
    }

    pub async fn profile(&self) -> AppProfile {
        self.profile.lock().await.clone()
    }

    pub async fn shutdown(mut self) -> Result<(), SyncTransportError> {
        let _ = self.mdns.take();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.await
                .map_err(|error| SyncTransportError::ServerTask(error.to_string()))??;
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct SyncClient {
    client: reqwest::Client,
    timeout: Duration,
}

impl SyncClient {
    pub fn new(config: SyncClientConfig) -> Result<Self, SyncTransportError> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(config.timeout)
            .build()?;
        Ok(Self {
            client,
            timeout: config.timeout,
        })
    }

    pub async fn exchange(
        &self,
        pairing: &PairingPayload,
        profile: &mut AppProfile,
    ) -> Result<crate::MergeReport, SyncTransportError> {
        pairing.validate_at(now_ms())?;
        let key_pair = EphemeralKeyPair::generate();
        let request = build_request(pairing, profile, &key_pair)?;
        let mut last_error = None;
        for ip in &pairing.ips {
            match self.exchange_http_at(*ip, pairing.port, &request).await {
                Ok(response) => return apply_response(pairing, profile, &key_pair, response),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            SyncTransportError::InvalidRequest("pairing payload has no IP addresses".to_string())
        }))
    }

    pub async fn exchange_websocket(
        &self,
        pairing: &PairingPayload,
        profile: &mut AppProfile,
    ) -> Result<crate::MergeReport, SyncTransportError> {
        pairing.validate_at(now_ms())?;
        let key_pair = EphemeralKeyPair::generate();
        let request = build_request(pairing, profile, &key_pair)?;
        let mut last_error = None;
        for ip in &pairing.ips {
            match self
                .exchange_websocket_at(*ip, pairing.port, &request)
                .await
            {
                Ok(response) => return apply_response(pairing, profile, &key_pair, response),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            SyncTransportError::InvalidRequest("pairing payload has no IP addresses".to_string())
        }))
    }

    pub async fn health(&self, pairing: &PairingPayload) -> Result<HealthBody, SyncTransportError> {
        pairing.validate_at(now_ms())?;
        let mut last_error = None;
        for ip in &pairing.ips {
            let url = http_url(*ip, pairing.port, HEALTH_PATH);
            match self
                .client
                .get(url)
                .query(&[("sid", pairing.session_id)])
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    return Ok(response.json().await?);
                }
                Ok(response) => {
                    last_error = Some(remote_error(response).await);
                }
                Err(error) => last_error = Some(SyncTransportError::Http(error)),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            SyncTransportError::InvalidRequest("pairing payload has no IP addresses".to_string())
        }))
    }

    async fn exchange_http_at(
        &self,
        ip: IpAddr,
        port: u16,
        request: &SyncRequest,
    ) -> Result<SyncResponse, SyncTransportError> {
        let response = self
            .client
            .post(http_url(ip, port, SYNC_PATH))
            .json(request)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(remote_error(response).await);
        }
        Ok(response.json().await?)
    }

    async fn exchange_websocket_at(
        &self,
        ip: IpAddr,
        port: u16,
        request: &SyncRequest,
    ) -> Result<SyncResponse, SyncTransportError> {
        let url = websocket_url(ip, port, WEBSOCKET_PATH);
        let connect = connect_async(url);
        let (mut socket, _) = tokio::time::timeout(self.timeout, connect)
            .await
            .map_err(|_| SyncTransportError::Timeout)??;
        socket
            .send(TungsteniteMessage::Text(
                serde_json::to_string(request)?.into(),
            ))
            .await?;
        let message = tokio::time::timeout(self.timeout, socket.next())
            .await
            .map_err(|_| SyncTransportError::Timeout)?
            .ok_or_else(|| {
                SyncTransportError::InvalidRequest("WebSocket closed without response".to_string())
            })??;
        match message {
            TungsteniteMessage::Text(text) => Ok(serde_json::from_str(text.as_ref())?),
            TungsteniteMessage::Close(_) => Err(SyncTransportError::InvalidRequest(
                "WebSocket closed before sync response".to_string(),
            )),
            _ => Err(SyncTransportError::InvalidRequest(
                "unexpected WebSocket response".to_string(),
            )),
        }
    }
}

fn build_request(
    pairing: &PairingPayload,
    profile: &AppProfile,
    key_pair: &EphemeralKeyPair,
) -> Result<SyncRequest, SyncTransportError> {
    let cipher = key_pair.derive_cipher(pairing.public_key_bytes()?, pairing.session_id)?;
    let payload = serde_json::to_vec(&SyncPayload::from_profile(profile))?;
    let envelope = cipher.encrypt(pairing.session_id, &payload)?;
    Ok(SyncRequest {
        session_id: pairing.session_id,
        pin: pairing.pin.clone(),
        client_public_key: key_pair.public_key_base64(),
        envelope,
    })
}

fn apply_response(
    pairing: &PairingPayload,
    profile: &mut AppProfile,
    key_pair: &EphemeralKeyPair,
    response: SyncResponse,
) -> Result<crate::MergeReport, SyncTransportError> {
    let cipher = key_pair.derive_cipher(pairing.public_key_bytes()?, pairing.session_id)?;
    let plaintext = cipher.decrypt(pairing.session_id, &response.envelope)?;
    let remote: SyncPayload = serde_json::from_slice(&plaintext)?;
    Ok(merge_profiles(profile, &remote.profile)?)
}

async fn health_handler(
    State(state): State<HostState>,
    Query(query): Query<HealthQuery>,
) -> Response {
    if query.sid != state.session.session_id() {
        return error_response(StatusCode::FORBIDDEN, "unknown session");
    }
    let now = now_ms();
    if now >= state.session.expires_at_ms() {
        return error_response(StatusCode::GONE, "sync session expired");
    }
    Json(HealthBody {
        ok: true,
        session_id: state.session.session_id(),
        host_time_ms: now,
        expires_at_ms: state.session.expires_at_ms(),
    })
    .into_response()
}

async fn sync_handler(
    State(state): State<HostState>,
    Json(request): Json<SyncRequest>,
) -> Response {
    match process_request(&state, request).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => transport_error_response(error),
    }
}

async fn websocket_handler(
    State(state): State<HostState>,
    websocket: WebSocketUpgrade,
) -> Response {
    websocket
        .on_upgrade(move |socket| websocket_session(socket, state))
        .into_response()
}

async fn websocket_session(mut socket: WebSocket, state: HostState) {
    let Some(Ok(Message::Text(text))) = socket.next().await else {
        return;
    };
    let result = match serde_json::from_str::<SyncRequest>(text.as_ref()) {
        Ok(request) => process_request(&state, request).await,
        Err(error) => Err(SyncTransportError::Json(error)),
    };
    match result {
        Ok(response) => {
            if let Ok(text) = serde_json::to_string(&response) {
                let _ = socket.send(Message::Text(text.into())).await;
            }
        }
        Err(error) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({"error": error.to_string()})
                        .to_string()
                        .into(),
                ))
                .await;
        }
    }
}

async fn process_request(
    state: &HostState,
    request: SyncRequest,
) -> Result<SyncResponse, SyncTransportError> {
    if request.session_id != state.session.session_id() {
        return Err(SyncTransportError::Remote {
            status: StatusCode::FORBIDDEN.as_u16(),
            message: "unknown session".to_string(),
        });
    }
    state
        .session
        .validate_pin(&request.pin, now_ms())
        .map_err(SyncTransportError::Pairing)?;
    let peer_public_key = crate::crypto::decode_public_key(&request.client_public_key)?;
    let cipher = state.session.derive_cipher(peer_public_key, now_ms())?;
    let plaintext = cipher.decrypt(request.session_id, &request.envelope)?;
    let remote: SyncPayload = serde_json::from_slice(&plaintext)?;
    if state.used.swap(true, Ordering::AcqRel) {
        return Err(SyncTransportError::SessionUsed);
    }
    let mut local = state.profile.lock().await;
    merge_profiles(&mut local, &remote.profile)?;
    let response_payload = serde_json::to_vec(&SyncPayload::from_profile(&local))?;
    let envelope = cipher.encrypt(request.session_id, &response_payload)?;
    Ok(SyncResponse { envelope })
}

fn http_url(ip: IpAddr, port: u16, path: &str) -> String {
    match ip {
        IpAddr::V4(ip) => format!("http://{ip}:{port}{path}"),
        IpAddr::V6(ip) => format!("http://[{ip}]:{port}{path}"),
    }
}

fn websocket_url(ip: IpAddr, port: u16, path: &str) -> String {
    match ip {
        IpAddr::V4(ip) => format!("ws://{ip}:{port}{path}"),
        IpAddr::V6(ip) => format!("ws://[{ip}]:{port}{path}"),
    }
}

async fn remote_error(response: reqwest::Response) -> SyncTransportError {
    let status = response.status().as_u16();
    let message = response
        .json::<ErrorBody>()
        .await
        .map(|body| body.error)
        .unwrap_or_else(|_| "remote request failed".to_string());
    if status == StatusCode::CONFLICT.as_u16() {
        SyncTransportError::SessionUsed
    } else {
        SyncTransportError::Remote { status, message }
    }
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.to_string(),
        }),
    )
        .into_response()
}

fn transport_error_response(error: SyncTransportError) -> Response {
    let status = match error {
        SyncTransportError::Pairing(PairingError::Expired) => StatusCode::GONE,
        SyncTransportError::Pairing(_) | SyncTransportError::Remote { .. } => StatusCode::FORBIDDEN,
        SyncTransportError::Crypto(_) => StatusCode::UNAUTHORIZED,
        SyncTransportError::SessionUsed => StatusCode::CONFLICT,
        SyncTransportError::InvalidRequest(_) | SyncTransportError::Json(_) => {
            StatusCode::BAD_REQUEST
        }
        SyncTransportError::Merge(_)
        | SyncTransportError::Io(_)
        | SyncTransportError::Http(_)
        | SyncTransportError::WebSocket(_)
        | SyncTransportError::Timeout
        | SyncTransportError::ServerTask(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error_response(status, &error.to_string())
}

impl Drop for RunningSyncHost {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
