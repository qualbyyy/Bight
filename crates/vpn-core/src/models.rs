use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileMode {
    Tun,
    SystemProxy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransportConfig {
    #[default]
    Tcp,
    WebSocket {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        host: Option<String>,
    },
    Grpc {
        service_name: String,
    },
    HttpUpgrade {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        host: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealitySettings {
    pub public_key: String,
    pub short_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spider_x: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TlsSettings {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub insecure: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reality: Option<RealitySettings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HysteriaObfs {
    pub kind: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "snake_case")]
pub enum NodeProtocol {
    Vless {
        uuid: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        flow: Option<String>,
        #[serde(default)]
        tls: TlsSettings,
        #[serde(default)]
        transport: TransportConfig,
    },
    Hysteria2 {
        password: String,
        #[serde(default)]
        tls: TlsSettings,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfs: Option<HysteriaObfs>,
        #[serde(skip_serializing_if = "Option::is_none")]
        up_mbps: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        down_mbps: Option<u32>,
    },
    Trojan {
        password: String,
        #[serde(default)]
        tls: TlsSettings,
        #[serde(default)]
        transport: TransportConfig,
    },
    Shadowsocks {
        method: String,
        password: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        plugin: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        plugin_opts: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeConfig {
    pub id: Uuid,
    pub name: String,
    pub server: String,
    pub server_port: u16,
    #[serde(flatten)]
    pub protocol: NodeProtocol,
    pub updated_at_ms: i64,
    pub is_deleted: bool,
}

impl NodeConfig {
    pub fn new(name: String, server: String, server_port: u16, protocol: NodeProtocol) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            server,
            server_port,
            protocol,
            updated_at_ms: 0,
            is_deleted: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub name: String,
    pub url: String,
    pub nodes: Vec<NodeConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traffic_used: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traffic_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fetched_at: Option<i64>,
    pub updated_at_ms: i64,
    pub is_deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMatch {
    Domain,
    DomainSuffix,
    DomainKeyword,
    IpCidr,
    RuleSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingAction {
    Proxy,
    Direct,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingRule {
    pub id: Uuid,
    pub enabled: bool,
    pub match_type: RoutingMatch,
    pub value: String,
    pub action: RoutingAction,
    pub comment: String,
    pub updated_at_ms: i64,
    pub is_deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppProfile {
    pub id: Uuid,
    pub name: String,
    pub subscriptions: Vec<Subscription>,
    pub custom_nodes: Vec<NodeConfig>,
    pub routing_rules: Vec<RoutingRule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_node_id: Option<Uuid>,
    pub mode: ProfileMode,
    pub updated_at_ms: i64,
    pub is_deleted: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}
