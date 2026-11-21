use std::{net::IpAddr, path::Path};

use serde_json::{Map, Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::models::{NodeConfig, NodeProtocol, ProfileMode, TlsSettings, TransportConfig};

use super::schema::{GEOIP_RU_URL, GEOSITE_RU_URL, LOCAL_ROUTE_EXCLUDES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPlatform {
    Linux,
    Windows,
    Android,
}

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub mode: ProfileMode,
    pub platform: TargetPlatform,
    pub mixed_port: u16,
    pub clash_api_port: u16,
    pub clash_secret: String,
    pub cache_file_path: String,
}

impl BuildOptions {
    pub fn new(mode: ProfileMode, platform: TargetPlatform) -> Self {
        Self {
            mode,
            platform,
            mixed_port: 2080,
            clash_api_port: 9090,
            clash_secret: Uuid::new_v4().simple().to_string(),
            cache_file_path: "cache.db".to_string(),
        }
    }

    pub fn with_ports(mut self, mixed_port: u16, clash_api_port: u16) -> Self {
        self.mixed_port = mixed_port;
        self.clash_api_port = clash_api_port;
        self
    }

    pub fn with_clash_secret(mut self, clash_secret: String) -> Self {
        self.clash_secret = clash_secret;
        self
    }

    pub fn with_cache_file_path(mut self, cache_file_path: String) -> Self {
        self.cache_file_path = cache_file_path;
        self
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid build options: {0}")]
    InvalidOptions(String),
    #[error("invalid node: {0}")]
    InvalidNode(String),
    #[error("invalid sing-box schema: {0}")]
    InvalidSchema(String),
    #[error("could not serialize sing-box config: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub fn build_config(node: &NodeConfig, options: BuildOptions) -> Result<Value, ConfigError> {
    validate_options(&options)?;
    if node.server.trim().is_empty() {
        return Err(ConfigError::InvalidNode(
            "proxy server is empty".to_string(),
        ));
    }
    if node.server_port == 0 {
        return Err(ConfigError::InvalidNode(
            "proxy server port is zero".to_string(),
        ));
    }

    let mut inbounds = Vec::new();
    if options.mode == ProfileMode::Tun {
        let mut tun = json!({
            "type": "tun",
            "tag": "tun-in",
            "address": ["172.19.0.1/30"],
            "mtu": 1400,
            "auto_route": true,
            "strict_route": false,
            "route_exclude_address": LOCAL_ROUTE_EXCLUDES.to_vec(),
        });
        if options.platform == TargetPlatform::Linux {
            tun["auto_redirect"] = json!(true);
        }
        inbounds.push(tun);
    }
    inbounds.push(json!({
        "type": "mixed",
        "tag": "mixed-in",
        "listen": "127.0.0.1",
        "listen_port": options.mixed_port,
    }));

    let mut route_rules = vec![
        json!({"action": "sniff"}),
        json!({"protocol": "dns", "action": "hijack-dns"}),
        proxy_bypass_rule(&node.server),
        json!({"ip_is_private": true, "outbound": "direct"}),
        json!({
            "rule_set": ["geoip-ru", "geosite-ru-direct"],
            "outbound": "direct"
        }),
    ];
    route_rules.retain(|rule| !rule.is_null());

    let mut route = json!({
        "default_domain_resolver": "local-dns",
        "rules": route_rules,
        "rule_set": [
            {
                "tag": "geoip-ru",
                "type": "remote",
                "format": "binary",
                "url": GEOIP_RU_URL,
                "download_detour": "proxy"
            },
            {
                "tag": "geosite-ru-direct",
                "type": "remote",
                "format": "binary",
                "url": GEOSITE_RU_URL,
                "download_detour": "proxy"
            }
        ],
        "final": "proxy"
    });
    if options.platform != TargetPlatform::Android {
        route["auto_detect_interface"] = json!(true);
    }

    let config = json!({
        "log": {
            "level": "info",
            "timestamp": true
        },
        "dns": {
            "servers": [
                {
                    "tag": "remote-dns",
                    "type": "https",
                    "server": "1.1.1.1",
                    "detour": "proxy"
                },
                {
                    "tag": "local-dns",
                    "type": "local"
                }
            ],
            "rules": [
                {
                    "rule_set": "geosite-ru-direct",
                    "server": "local-dns"
                }
            ],
            "final": "remote-dns",
            "strategy": "prefer_ipv4"
        },
        "inbounds": inbounds,
        "outbounds": [node_outbound(node)?, {"type": "direct", "tag": "direct"}],
        "route": route,
        "experimental": {
            "cache_file": {
                "enabled": true,
                "path": options.cache_file_path,
                "store_dns": true
            },
            "clash_api": {
                "external_controller": format!("127.0.0.1:{}", options.clash_api_port),
                "secret": options.clash_secret
            }
        }
    });

    validate_config(&config)?;
    Ok(config)
}

pub fn build_config_json(node: &NodeConfig, options: BuildOptions) -> Result<String, ConfigError> {
    Ok(serde_json::to_string_pretty(&build_config(node, options)?)?)
}

pub fn validate_config(config: &Value) -> Result<(), ConfigError> {
    let object = config
        .as_object()
        .ok_or_else(|| ConfigError::InvalidSchema("config must be a JSON object".to_string()))?;
    for key in [
        "log",
        "dns",
        "inbounds",
        "outbounds",
        "route",
        "experimental",
    ] {
        if !object.contains_key(key) {
            return Err(ConfigError::InvalidSchema(format!(
                "missing top-level field: {key}"
            )));
        }
    }
    reject_legacy_keys(config)?;

    let dns = object["dns"]
        .as_object()
        .ok_or_else(|| ConfigError::InvalidSchema("dns must be an object".to_string()))?;
    let dns_servers = dns["servers"]
        .as_array()
        .ok_or_else(|| ConfigError::InvalidSchema("dns.servers must be an array".to_string()))?;
    let dns_tags = dns_servers
        .iter()
        .map(|server| {
            let server = server.as_object().ok_or_else(|| {
                ConfigError::InvalidSchema("each dns server must be an object".to_string())
            })?;
            if server.contains_key("address") {
                return Err(ConfigError::InvalidSchema(
                    "dns.servers must use server, not address".to_string(),
                ));
            }
            let tag = server.get("tag").and_then(Value::as_str).ok_or_else(|| {
                ConfigError::InvalidSchema("dns server tag is missing".to_string())
            })?;
            let kind = server.get("type").and_then(Value::as_str).ok_or_else(|| {
                ConfigError::InvalidSchema("dns server type is missing".to_string())
            })?;
            if kind == "https" && server.get("server").and_then(Value::as_str).is_none() {
                return Err(ConfigError::InvalidSchema(
                    "HTTPS DNS server must contain server".to_string(),
                ));
            }
            Ok(tag.to_string())
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;
    let dns_final = dns
        .get("final")
        .and_then(Value::as_str)
        .ok_or_else(|| ConfigError::InvalidSchema("dns.final is missing".to_string()))?;
    if !dns_tags.iter().any(|tag| tag == dns_final) {
        return Err(ConfigError::InvalidSchema(
            "dns.final references an unknown DNS server".to_string(),
        ));
    }

    let outbounds = object["outbounds"]
        .as_array()
        .ok_or_else(|| ConfigError::InvalidSchema("outbounds must be an array".to_string()))?;
    let outbound_tags = outbounds
        .iter()
        .map(|outbound| {
            let outbound = outbound.as_object().ok_or_else(|| {
                ConfigError::InvalidSchema("each outbound must be an object".to_string())
            })?;
            let kind = outbound
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ConfigError::InvalidSchema("outbound type is missing".to_string())
                })?;
            if matches!(kind, "dns" | "block") {
                return Err(ConfigError::InvalidSchema(format!(
                    "forbidden legacy outbound type: {kind}"
                )));
            }
            let tag = outbound
                .get("tag")
                .and_then(Value::as_str)
                .ok_or_else(|| ConfigError::InvalidSchema("outbound tag is missing".to_string()))?;
            Ok(tag.to_string())
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;
    for required in ["proxy", "direct"] {
        if !outbound_tags.iter().any(|tag| tag == required) {
            return Err(ConfigError::InvalidSchema(format!(
                "missing required outbound: {required}"
            )));
        }
    }

    let inbounds = object["inbounds"]
        .as_array()
        .ok_or_else(|| ConfigError::InvalidSchema("inbounds must be an array".to_string()))?;
    for inbound in inbounds {
        let inbound = inbound.as_object().ok_or_else(|| {
            ConfigError::InvalidSchema("each inbound must be an object".to_string())
        })?;
        if inbound.get("type").and_then(Value::as_str) == Some("tun") {
            let excludes = inbound
                .get("route_exclude_address")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
                .ok_or_else(|| {
                    ConfigError::InvalidSchema(
                        "tun inbound must exclude local/private subnets".to_string(),
                    )
                })?;
            if excludes.iter().any(|item| item.as_str().is_none()) {
                return Err(ConfigError::InvalidSchema(
                    "tun route exclusions must be CIDR strings".to_string(),
                ));
            }
        }
    }

    let route = object["route"]
        .as_object()
        .ok_or_else(|| ConfigError::InvalidSchema("route must be an object".to_string()))?;
    let final_outbound = route
        .get("final")
        .and_then(Value::as_str)
        .ok_or_else(|| ConfigError::InvalidSchema("route.final is missing".to_string()))?;
    if !outbound_tags.iter().any(|tag| tag == final_outbound) {
        return Err(ConfigError::InvalidSchema(
            "route.final references an unknown outbound".to_string(),
        ));
    }

    let experimental = object["experimental"]
        .as_object()
        .ok_or_else(|| ConfigError::InvalidSchema("experimental must be an object".to_string()))?;
    let cache_path = experimental
        .get("cache_file")
        .and_then(Value::as_object)
        .and_then(|cache| cache.get("path"))
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty());
    if cache_path.is_none() {
        return Err(ConfigError::InvalidSchema(
            "experimental.cache_file.path is missing".to_string(),
        ));
    }
    let clash_api = experimental
        .get("clash_api")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ConfigError::InvalidSchema("experimental.clash_api is missing".to_string())
        })?;
    if clash_api
        .get("external_controller")
        .and_then(Value::as_str)
        .is_none()
        || clash_api.get("secret").and_then(Value::as_str).is_none()
    {
        return Err(ConfigError::InvalidSchema(
            "Clash API controller and secret are required".to_string(),
        ));
    }

    Ok(())
}

fn validate_options(options: &BuildOptions) -> Result<(), ConfigError> {
    if options.mixed_port == 0 {
        return Err(ConfigError::InvalidOptions(
            "mixed port is zero".to_string(),
        ));
    }
    if options.clash_api_port == 0 {
        return Err(ConfigError::InvalidOptions(
            "Clash API port is zero".to_string(),
        ));
    }
    if options.clash_secret.trim().is_empty() {
        return Err(ConfigError::InvalidOptions(
            "Clash API secret is empty".to_string(),
        ));
    }
    if options.cache_file_path.trim().is_empty() {
        return Err(ConfigError::InvalidOptions(
            "cache file path is empty".to_string(),
        ));
    }
    if options.platform == TargetPlatform::Android
        && !Path::new(&options.cache_file_path).is_absolute()
    {
        return Err(ConfigError::InvalidOptions(
            "Android cache file path must be absolute".to_string(),
        ));
    }
    Ok(())
}

fn node_outbound(node: &NodeConfig) -> Result<Value, ConfigError> {
    let mut outbound = Map::new();
    outbound.insert("tag".to_string(), json!("proxy"));
    outbound.insert("server".to_string(), json!(node.server));
    outbound.insert("server_port".to_string(), json!(node.server_port));

    match &node.protocol {
        NodeProtocol::Vless {
            uuid,
            flow,
            tls,
            transport,
        } => {
            outbound.insert("type".to_string(), json!("vless"));
            outbound.insert("uuid".to_string(), json!(uuid));
            if let Some(flow) = flow {
                outbound.insert("flow".to_string(), json!(flow));
            }
            if let Some(tls) = tls_value(tls) {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(transport) = transport_value(transport) {
                outbound.insert("transport".to_string(), transport);
            }
        }
        NodeProtocol::Hysteria2 {
            password,
            tls,
            obfs,
            up_mbps,
            down_mbps,
        } => {
            outbound.insert("type".to_string(), json!("hysteria2"));
            outbound.insert("password".to_string(), json!(password));
            if let Some(tls) = tls_value(tls) {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(obfs) = obfs {
                outbound.insert(
                    "obfs".to_string(),
                    json!({"type": obfs.kind, "password": obfs.password}),
                );
            }
            if let Some(up_mbps) = up_mbps {
                outbound.insert("up_mbps".to_string(), json!(up_mbps));
            }
            if let Some(down_mbps) = down_mbps {
                outbound.insert("down_mbps".to_string(), json!(down_mbps));
            }
        }
        NodeProtocol::Trojan {
            password,
            tls,
            transport,
        } => {
            outbound.insert("type".to_string(), json!("trojan"));
            outbound.insert("password".to_string(), json!(password));
            if let Some(tls) = tls_value(tls) {
                outbound.insert("tls".to_string(), tls);
            }
            if let Some(transport) = transport_value(transport) {
                outbound.insert("transport".to_string(), transport);
            }
        }
        NodeProtocol::Shadowsocks {
            method,
            password,
            plugin,
            plugin_opts,
        } => {
            outbound.insert("type".to_string(), json!("shadowsocks"));
            outbound.insert("method".to_string(), json!(method));
            outbound.insert("password".to_string(), json!(password));
            if let Some(plugin) = plugin {
                outbound.insert("plugin".to_string(), json!(plugin));
            }
            if let Some(plugin_opts) = plugin_opts {
                outbound.insert("plugin_opts".to_string(), json!(plugin_opts));
            }
        }
    }

    Ok(Value::Object(outbound))
}

fn tls_value(tls: &TlsSettings) -> Option<Value> {
    if !tls.enabled {
        return None;
    }
    let mut value = json!({"enabled": true});
    if let Some(server_name) = &tls.server_name {
        value["server_name"] = json!(server_name);
    }
    if tls.insecure {
        value["insecure"] = json!(true);
    }
    if let Some(fingerprint) = &tls.fingerprint {
        value["utls"] = json!({"enabled": true, "fingerprint": fingerprint});
    }
    if let Some(reality) = &tls.reality {
        let mut reality_value = json!({
            "enabled": true,
            "public_key": reality.public_key,
            "short_id": reality.short_id,
        });
        if let Some(spider_x) = &reality.spider_x {
            reality_value["spider_x"] = json!(spider_x);
        }
        value["reality"] = reality_value;
    }
    Some(value)
}

fn transport_value(transport: &TransportConfig) -> Option<Value> {
    match transport {
        TransportConfig::Tcp => None,
        TransportConfig::WebSocket { path, host } => {
            let mut value = json!({"type": "ws", "path": path});
            if let Some(host) = host {
                value["headers"] = json!({"Host": host});
            }
            Some(value)
        }
        TransportConfig::Grpc { service_name } => {
            Some(json!({"type": "grpc", "service_name": service_name}))
        }
        TransportConfig::HttpUpgrade { path, host } => {
            let mut value = json!({"type": "httpupgrade", "path": path});
            if let Some(host) = host {
                value["headers"] = json!({"Host": host});
            }
            Some(value)
        }
    }
}

fn proxy_bypass_rule(server: &str) -> Value {
    if let Ok(ip) = server.parse::<IpAddr>() {
        let suffix = match ip {
            IpAddr::V4(_) => "/32",
            IpAddr::V6(_) => "/128",
        };
        return json!({
            "ip_cidr": [format!("{server}{suffix}")],
            "outbound": "direct"
        });
    }
    json!({"domain": [server], "outbound": "direct"})
}

fn reject_legacy_keys(value: &Value) -> Result<(), ConfigError> {
    match value {
        Value::Object(object) => {
            for key in object.keys() {
                if matches!(
                    key.as_str(),
                    "inet4_address" | "geosite" | "geoip" | "stack"
                ) {
                    return Err(ConfigError::InvalidSchema(format!(
                        "legacy or removed field is forbidden: {key}"
                    )));
                }
            }
            for child in object.values() {
                reject_legacy_keys(child)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                reject_legacy_keys(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}
