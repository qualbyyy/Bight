use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use if_addrs::get_if_addrs;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::{sync::Semaphore, task::JoinSet, time::sleep};
use uuid::Uuid;
use vpn_core::{
    AppProfile, NodeConfig, NodeProtocol, ProfileMode, RoutingAction, RoutingMatch, RoutingRule,
    Subscription, decode_subscription, parse_node_uri, parse_subscription_userinfo,
};
use vpn_sync::{
    DEFAULT_SESSION_TTL, MdnsPeer, MergeReport, PairingPayload, SyncClient, SyncClientConfig,
    SyncHost, SyncHostConfig, merge_profiles, now_ms,
};

use crate::{
    core_manager::{self, ConnectOptions},
    state::{ConnectionStatus, LogEntry, SharedAppState, save_state_async},
};

const DEFAULT_TEST_URL: &str = "https://www.gstatic.com/generate_204";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInput {
    pub name: String,
    pub server: String,
    pub server_port: u16,
    pub protocol: NodeProtocol,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRuleInput {
    pub enabled: bool,
    pub match_type: RoutingMatch,
    pub value: String,
    pub action: RoutingAction,
    pub comment: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PingResult {
    pub node_id: Uuid,
    pub node_name: String,
    pub delay_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncHostInfo {
    pub pairing: PairingPayload,
    pub qr_payload: String,
    pub pin: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncPeer {
    pub instance_name: String,
    pub pairing: PairingPayload,
}

#[tauri::command]
pub async fn get_profile(state: State<'_, SharedAppState>) -> Result<AppProfile, String> {
    Ok(state.read().await.profile.clone())
}

#[tauri::command]
pub async fn list_subscriptions(
    state: State<'_, SharedAppState>,
) -> Result<Vec<Subscription>, String> {
    Ok(state
        .read()
        .await
        .profile
        .subscriptions
        .iter()
        .filter(|subscription| !subscription.is_deleted)
        .cloned()
        .collect())
}

#[tauri::command]
pub async fn import_subscription(
    url: String,
    name: Option<String>,
    state: State<'_, SharedAppState>,
) -> Result<Subscription, String> {
    let (nodes, usage) = fetch_subscription(&url).await?;
    let timestamp = now_ms();
    let subscription = Subscription {
        id: Uuid::new_v4(),
        name: name
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| url.clone()),
        url,
        nodes: nodes
            .into_iter()
            .map(|mut node| {
                node.updated_at_ms = timestamp;
                node.is_deleted = false;
                node
            })
            .collect(),
        traffic_used: usage.as_ref().and_then(|value| {
            value
                .upload_bytes
                .zip(value.download_bytes)
                .map(|(upload, download)| upload.saturating_add(download))
        }),
        traffic_total: usage.as_ref().and_then(|value| value.total_bytes),
        expire_at: usage.as_ref().and_then(|value| value.expire_at_ms),
        last_fetched_at: Some(timestamp),
        updated_at_ms: timestamp,
        is_deleted: false,
    };
    let result = mutate_profile(&state, |profile| {
        profile.subscriptions.push(subscription.clone());
        Ok(subscription.clone())
    })
    .await?;
    Ok(result)
}

#[tauri::command]
pub async fn refresh_subscription(
    subscription_id: Uuid,
    state: State<'_, SharedAppState>,
) -> Result<Subscription, String> {
    let current = state
        .read()
        .await
        .profile
        .subscriptions
        .iter()
        .find(|subscription| subscription.id == subscription_id && !subscription.is_deleted)
        .cloned()
        .ok_or_else(|| format!("subscription {subscription_id} was not found"))?;
    let (nodes, usage) = fetch_subscription(&current.url).await?;
    let timestamp = now_ms();
    let refreshed = Subscription {
        id: current.id,
        name: current.name,
        url: current.url,
        nodes: nodes
            .into_iter()
            .map(|mut node| {
                node.updated_at_ms = timestamp;
                node.is_deleted = false;
                node
            })
            .collect(),
        traffic_used: usage.as_ref().and_then(|value| {
            value
                .upload_bytes
                .zip(value.download_bytes)
                .map(|(upload, download)| upload.saturating_add(download))
        }),
        traffic_total: usage.as_ref().and_then(|value| value.total_bytes),
        expire_at: usage.as_ref().and_then(|value| value.expire_at_ms),
        last_fetched_at: Some(timestamp),
        updated_at_ms: timestamp,
        is_deleted: false,
    };
    mutate_profile(&state, |profile| {
        let subscription = profile
            .subscriptions
            .iter_mut()
            .find(|subscription| subscription.id == subscription_id)
            .ok_or_else(|| format!("subscription {subscription_id} was not found"))?;
        *subscription = refreshed.clone();
        Ok(refreshed.clone())
    })
    .await
}

#[tauri::command]
pub async fn delete_subscription(
    subscription_id: Uuid,
    state: State<'_, SharedAppState>,
) -> Result<(), String> {
    mutate_profile(&state, |profile| {
        let subscription = profile
            .subscriptions
            .iter_mut()
            .find(|subscription| subscription.id == subscription_id && !subscription.is_deleted)
            .ok_or_else(|| format!("subscription {subscription_id} was not found"))?;
        subscription.is_deleted = true;
        subscription.updated_at_ms = now_ms();
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn import_node_uri(
    uri: String,
    name: Option<String>,
    state: State<'_, SharedAppState>,
) -> Result<NodeConfig, String> {
    let mut node = parse_node_uri(&uri).map_err(|error| error.to_string())?;
    if let Some(name) = name.filter(|value| !value.trim().is_empty()) {
        node.name = name;
    }
    node.updated_at_ms = now_ms();
    mutate_profile(&state, |profile| {
        profile.custom_nodes.push(node.clone());
        Ok(node.clone())
    })
    .await
}

#[tauri::command]
pub async fn list_nodes(state: State<'_, SharedAppState>) -> Result<Vec<NodeConfig>, String> {
    let profile = state.read().await.profile.clone();
    let mut nodes = profile
        .custom_nodes
        .into_iter()
        .filter(|node| !node.is_deleted)
        .collect::<Vec<_>>();
    nodes.extend(
        profile
            .subscriptions
            .into_iter()
            .filter(|subscription| !subscription.is_deleted)
            .flat_map(|subscription| subscription.nodes)
            .filter(|node| !node.is_deleted),
    );
    nodes.sort_by_key(|node| node.id);
    Ok(nodes)
}

#[tauri::command]
pub async fn create_node(
    input: NodeInput,
    state: State<'_, SharedAppState>,
) -> Result<NodeConfig, String> {
    validate_node_input(&input)?;
    let node = NodeConfig::new(input.name, input.server, input.server_port, input.protocol);
    mutate_profile(&state, |profile| {
        profile.custom_nodes.push(node.clone());
        Ok(node.clone())
    })
    .await
}

#[tauri::command]
pub async fn update_node(
    mut node: NodeConfig,
    state: State<'_, SharedAppState>,
) -> Result<NodeConfig, String> {
    if node.id.is_nil() {
        return Err("node ID must not be nil".to_string());
    }
    if node.server.trim().is_empty() || node.server_port == 0 {
        return Err("node server and port are required".to_string());
    }
    node.updated_at_ms = now_ms();
    node.is_deleted = false;
    mutate_profile(&state, |profile| {
        let current = profile
            .custom_nodes
            .iter_mut()
            .find(|current| current.id == node.id)
            .ok_or_else(|| format!("custom node {} was not found", node.id))?;
        *current = node.clone();
        Ok(node.clone())
    })
    .await
}

#[tauri::command]
pub async fn delete_node(node_id: Uuid, state: State<'_, SharedAppState>) -> Result<(), String> {
    mutate_profile(&state, |profile| {
        let node = profile
            .custom_nodes
            .iter_mut()
            .find(|node| node.id == node_id && !node.is_deleted)
            .ok_or_else(|| format!("custom node {node_id} was not found"))?;
        node.is_deleted = true;
        node.updated_at_ms = now_ms();
        if profile.active_node_id == Some(node_id) {
            profile.active_node_id = None;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn list_rules(state: State<'_, SharedAppState>) -> Result<Vec<RoutingRule>, String> {
    Ok(state
        .read()
        .await
        .profile
        .routing_rules
        .iter()
        .filter(|rule| !rule.is_deleted)
        .cloned()
        .collect())
}

#[tauri::command]
pub async fn create_rule(
    input: RoutingRuleInput,
    state: State<'_, SharedAppState>,
) -> Result<RoutingRule, String> {
    validate_rule_input(&input)?;
    let rule = RoutingRule {
        id: Uuid::new_v4(),
        enabled: input.enabled,
        match_type: input.match_type,
        value: input.value,
        action: input.action,
        comment: input.comment,
        updated_at_ms: now_ms(),
        is_deleted: false,
    };
    mutate_profile(&state, |profile| {
        profile.routing_rules.push(rule.clone());
        Ok(rule.clone())
    })
    .await
}

#[tauri::command]
pub async fn update_rule(
    mut rule: RoutingRule,
    state: State<'_, SharedAppState>,
) -> Result<RoutingRule, String> {
    if rule.id.is_nil() || rule.value.trim().is_empty() {
        return Err("rule ID and value are required".to_string());
    }
    rule.updated_at_ms = now_ms();
    rule.is_deleted = false;
    mutate_profile(&state, |profile| {
        let current = profile
            .routing_rules
            .iter_mut()
            .find(|current| current.id == rule.id)
            .ok_or_else(|| format!("routing rule {} was not found", rule.id))?;
        *current = rule.clone();
        Ok(rule.clone())
    })
    .await
}

#[tauri::command]
pub async fn delete_rule(rule_id: Uuid, state: State<'_, SharedAppState>) -> Result<(), String> {
    mutate_profile(&state, |profile| {
        let rule = profile
            .routing_rules
            .iter_mut()
            .find(|rule| rule.id == rule_id && !rule.is_deleted)
            .ok_or_else(|| format!("routing rule {rule_id} was not found"))?;
        rule.is_deleted = true;
        rule.updated_at_ms = now_ms();
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn connect(
    app: AppHandle,
    state: State<'_, SharedAppState>,
    node_id: Option<Uuid>,
    mode: Option<ProfileMode>,
) -> Result<core_manager::ConnectResult, String> {
    core_manager::connect(&app, state.inner(), ConnectOptions { node_id, mode })
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn disconnect(app: AppHandle, state: State<'_, SharedAppState>) -> Result<(), String> {
    core_manager::disconnect(&app, state.inner())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_status(state: State<'_, SharedAppState>) -> Result<ConnectionStatus, String> {
    Ok(state.read().await.status.clone())
}

#[tauri::command]
pub async fn get_logs(
    state: State<'_, SharedAppState>,
    limit: Option<usize>,
) -> Result<Vec<LogEntry>, String> {
    Ok(state.read().await.logs.snapshot(limit))
}

#[tauri::command]
pub async fn set_mode(
    app: AppHandle,
    state: State<'_, SharedAppState>,
    mode: ProfileMode,
) -> Result<(), String> {
    core_manager::set_mode(&app, state.inner(), mode)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn ping_node(
    app: AppHandle,
    state: State<'_, SharedAppState>,
    node_id: Uuid,
    test_url: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<PingResult, String> {
    let profile = state.read().await.profile.clone();
    let node = find_node(&profile, node_id)
        .ok_or_else(|| format!("proxy node {node_id} was not found"))?;
    if profile.active_node_id != Some(node_id) {
        return Err(format!("proxy node {} is not active", node.name));
    }
    let client = core_manager::clash_client(state.inner())
        .await
        .map_err(|error| error.to_string())?;
    let result = client
        .check_delay(
            "proxy",
            test_url.as_deref().unwrap_or(DEFAULT_TEST_URL),
            timeout_ms.unwrap_or(2_500),
        )
        .await;
    let ping = match result {
        Ok(delay_ms) => PingResult {
            node_id,
            node_name: node.name,
            delay_ms: Some(delay_ms),
            error: None,
        },
        Err(error) => PingResult {
            node_id,
            node_name: node.name,
            delay_ms: None,
            error: Some(error.to_string()),
        },
    };
    let _ = app.emit("ping-result", ping.clone());
    Ok(ping)
}

#[tauri::command]
pub async fn ping_all(
    app: AppHandle,
    state: State<'_, SharedAppState>,
    test_url: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<Vec<PingResult>, String> {
    let profile = state.read().await.profile.clone();
    let nodes = all_nodes(&profile);
    let active_node_id = profile.active_node_id;
    let client = core_manager::clash_client(state.inner())
        .await
        .map_err(|error| error.to_string())?;
    let semaphore = Arc::new(Semaphore::new(12));
    let mut tasks = JoinSet::new();
    for node in nodes {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| error.to_string())?;
        let client = client.clone();
        let app = app.clone();
        let url = test_url
            .clone()
            .unwrap_or_else(|| DEFAULT_TEST_URL.to_string());
        let timeout_ms = timeout_ms.unwrap_or(2_500);
        tasks.spawn(async move {
            let result = if Some(node.id) != active_node_id {
                PingResult {
                    node_id: node.id,
                    node_name: node.name,
                    delay_ms: None,
                    error: Some("node is not active in the current sing-box profile".to_string()),
                }
            } else {
                match client.check_delay("proxy", &url, timeout_ms).await {
                    Ok(delay_ms) => PingResult {
                        node_id: node.id,
                        node_name: node.name,
                        delay_ms: Some(delay_ms),
                        error: None,
                    },
                    Err(error) => PingResult {
                        node_id: node.id,
                        node_name: node.name,
                        delay_ms: None,
                        error: Some(error.to_string()),
                    },
                }
            };
            drop(permit);
            let _ = app.emit("ping-result", result.clone());
            result
        });
    }
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        results.push(result.map_err(|error| error.to_string())?);
    }
    results.sort_by_key(|result| result.node_id);
    Ok(results)
}

#[tauri::command]
pub async fn get_active_connections(
    state: State<'_, SharedAppState>,
) -> Result<vpn_core::ClashConnections, String> {
    core_manager::clash_client(state.inner())
        .await
        .map_err(|error| error.to_string())?
        .active_connections()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn sync_start_host(
    app: AppHandle,
    state: State<'_, SharedAppState>,
) -> Result<SyncHostInfo, String> {
    stop_existing_sync_host(&app, state.inner()).await?;
    let ips = discover_lan_ips()?;
    if ips.is_empty() {
        return Err("no valid private LAN IPv4 address was found".to_string());
    }
    let profile = state.read().await.profile.clone();
    let config = SyncHostConfig::new(SocketAddr::from(([0, 0, 0, 0], 0)))
        .with_ips(ips)
        .with_session_ttl(DEFAULT_SESSION_TTL)
        .with_mdns("qonnect");
    let host = SyncHost::start(profile, config)
        .await
        .map_err(|error| error.to_string())?;
    let pairing = host.pairing().clone();
    let qr_payload = pairing.to_uri().map_err(|error| error.to_string())?;
    let session_id = pairing.session_id;
    let expires_at_ms = pairing.expires_at_ms;
    {
        let mut guard = state.write().await;
        guard.sync.host = Some(host);
        guard.sync.session_id = Some(session_id);
        guard.sync.expires_at_ms = Some(expires_at_ms);
    }
    spawn_sync_expiry(app, state.inner().clone(), session_id, expires_at_ms);
    Ok(SyncHostInfo {
        pin: pairing.pin.clone(),
        pairing,
        qr_payload,
    })
}

#[tauri::command]
pub async fn sync_stop_host(
    app: AppHandle,
    state: State<'_, SharedAppState>,
) -> Result<(), String> {
    stop_existing_sync_host(&app, state.inner()).await
}

#[tauri::command]
pub async fn sync_discover_peers(timeout_ms: Option<u64>) -> Result<Vec<SyncPeer>, String> {
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(3_000).min(30_000));
    vpn_sync::discover_mdns(timeout)
        .await
        .map_err(|error| error.to_string())
        .map(|peers| peers.into_iter().map(sync_peer).collect())
}

#[tauri::command]
pub async fn sync_connect_peer(
    pairing_uri: String,
    state: State<'_, SharedAppState>,
) -> Result<MergeReport, String> {
    let pairing = if pairing_uri.trim_start().starts_with('{') {
        PairingPayload::from_json(&pairing_uri)
    } else {
        PairingPayload::from_uri(&pairing_uri)
    }
    .map_err(|error| error.to_string())?;
    let client = SyncClient::new(SyncClientConfig {
        timeout: Duration::from_millis(5_000),
    })
    .map_err(|error| error.to_string())?;
    let mut profile = state.read().await.profile.clone();
    let report = client
        .exchange(&pairing, &mut profile)
        .await
        .map_err(|error| error.to_string())?;
    {
        let mut guard = state.write().await;
        guard.profile = profile;
    }
    save_state_async(state.inner())
        .await
        .map_err(|error| error.to_string())?;
    Ok(report)
}

async fn fetch_subscription(
    url: &str,
) -> Result<(Vec<NodeConfig>, Option<vpn_core::SubscriptionUsage>), String> {
    if url.trim().is_empty() {
        return Err("subscription URL is empty".to_string());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "subscription server returned HTTP {}",
            response.status()
        ));
    }
    let usage = response
        .headers()
        .get("subscription-userinfo")
        .map(|value| {
            value
                .to_str()
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    parse_subscription_userinfo(value).map_err(|error| error.to_string())
                })
        })
        .transpose()?;
    let body = response.bytes().await.map_err(|error| error.to_string())?;
    let text = String::from_utf8(body.to_vec()).map_err(|error| error.to_string())?;
    let decoded = decode_subscription(&text).map_err(|error| error.to_string())?;
    Ok((decoded.nodes, usage))
}

async fn mutate_profile<T, F>(state: &SharedAppState, mutate: F) -> Result<T, String>
where
    F: FnOnce(&mut AppProfile) -> Result<T, String>,
{
    let operation_lock = state.read().await.operation_lock();
    let _operation = operation_lock.lock().await;
    let result = {
        let mut guard = state.write().await;
        let result = mutate(&mut guard.profile)?;
        guard.profile.updated_at_ms = now_ms();
        guard.profile.is_deleted = false;
        result
    };
    save_state_async(state)
        .await
        .map_err(|error| error.to_string())?;
    Ok(result)
}

fn validate_node_input(input: &NodeInput) -> Result<(), String> {
    if input.name.trim().is_empty() || input.server.trim().is_empty() || input.server_port == 0 {
        return Err("node name, server and non-zero port are required".to_string());
    }
    Ok(())
}

fn validate_rule_input(input: &RoutingRuleInput) -> Result<(), String> {
    if input.value.trim().is_empty() {
        return Err("routing rule value is required".to_string());
    }
    Ok(())
}

fn find_node(profile: &AppProfile, node_id: Uuid) -> Option<NodeConfig> {
    all_nodes(profile)
        .into_iter()
        .find(|node| node.id == node_id && !node.is_deleted)
}

fn all_nodes(profile: &AppProfile) -> Vec<NodeConfig> {
    let mut seen = HashSet::new();
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
        .filter(|node| !node.is_deleted && seen.insert(node.id))
        .cloned()
        .collect()
}

fn discover_lan_ips() -> Result<Vec<IpAddr>, String> {
    let mut ips = Vec::new();
    for interface in get_if_addrs().map_err(|error| error.to_string())? {
        let IpAddr::V4(ip) = interface.ip() else {
            continue;
        };
        if is_valid_lan_interface(&interface.name, ip) {
            ips.push(IpAddr::V4(ip));
        }
    }
    ips.sort();
    ips.dedup();
    Ok(ips)
}

fn is_valid_lan_interface(name: &str, ip: Ipv4Addr) -> bool {
    if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || !ip.is_private() {
        return false;
    }
    let octets = ip.octets();
    if octets[0] == 172 && octets[1] == 19 {
        return false;
    }
    let name = name.to_ascii_lowercase();
    [
        "docker",
        "br-",
        "veth",
        "virbr",
        "tun",
        "tap",
        "wg",
        "utun",
        "tailscale",
        "vethernet",
        "wsl",
        "vbox",
        "vmnet",
    ]
    .iter()
    .all(|prefix| !name.contains(prefix))
}

async fn stop_existing_sync_host(app: &AppHandle, state: &SharedAppState) -> Result<(), String> {
    let host = {
        let mut guard = state.write().await;
        guard.take_sync_host()
    };
    let Some(host) = host else {
        return Ok(());
    };
    let host_profile = host.profile().await;
    host.shutdown().await.map_err(|error| error.to_string())?;
    {
        let mut guard = state.write().await;
        merge_profiles(&mut guard.profile, &host_profile).map_err(|error| error.to_string())?;
    }
    save_state_async(state)
        .await
        .map_err(|error| error.to_string())?;
    let _ = app.emit("sync-host-stopped", ());
    Ok(())
}

fn spawn_sync_expiry(app: AppHandle, state: SharedAppState, session_id: Uuid, expires_at_ms: i64) {
    tauri::async_runtime::spawn(async move {
        let delay_ms = expires_at_ms.saturating_sub(now_ms()).max(0) as u64;
        sleep(Duration::from_millis(delay_ms)).await;
        let host = {
            let mut guard = state.write().await;
            if guard.has_sync_session(session_id) {
                guard.take_sync_host()
            } else {
                None
            }
        };
        let Some(host) = host else {
            return;
        };
        let profile = host.profile().await;
        let _ = host.shutdown().await;
        {
            let merged = {
                let mut guard = state.write().await;
                merge_profiles(&mut guard.profile, &profile).is_ok()
            };
            if merged {
                let _ = save_state_async(&state).await;
            }
        }
        let _ = app.emit("sync-host-expired", session_id);
    });
}

fn sync_peer(peer: MdnsPeer) -> SyncPeer {
    SyncPeer {
        instance_name: peer.instance_name,
        pairing: peer.pairing,
    }
}
