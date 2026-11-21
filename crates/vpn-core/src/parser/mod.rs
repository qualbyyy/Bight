use std::num::ParseIntError;

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD},
};
use percent_encoding::percent_decode_str;
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::models::{
    HysteriaObfs, NodeConfig, NodeProtocol, RealitySettings, TlsSettings, TransportConfig,
};

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid URI: {0}")]
    InvalidUri(#[from] url::ParseError),
    #[error("unsupported node scheme: {0}")]
    UnsupportedScheme(String),
    #[error("missing URI field: {0}")]
    MissingField(&'static str),
    #[error("invalid node: {0}")]
    InvalidNode(String),
    #[error("invalid port: {0}")]
    InvalidPort(#[from] ParseIntError),
    #[error("invalid base64 payload")]
    InvalidBase64,
    #[error("invalid UTF-8 in decoded payload")]
    InvalidUtf8,
    #[error("invalid JSON subscription: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("subscription has no valid nodes")]
    EmptySubscription,
    #[error("invalid subscription-userinfo field: {0}")]
    InvalidUserInfo(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionUsage {
    pub upload_bytes: Option<u64>,
    pub download_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub expire_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionDecode {
    pub nodes: Vec<NodeConfig>,
    pub skipped: Vec<String>,
    pub usage: Option<SubscriptionUsage>,
}

pub fn parse_node_uri(input: &str) -> Result<NodeConfig, ParseError> {
    let input = input.trim();
    let url = Url::parse(input)?;
    let scheme = url.scheme().to_ascii_lowercase();
    match scheme.as_str() {
        "vless" => parse_vless(&url),
        "hy2" | "hysteria2" => parse_hysteria2(&url),
        "trojan" => parse_trojan(&url),
        "ss" => parse_shadowsocks(input, &url),
        other => Err(ParseError::UnsupportedScheme(other.to_string())),
    }
}

pub fn parse_subscription_userinfo(value: &str) -> Result<SubscriptionUsage, ParseError> {
    let mut upload_bytes = None;
    let mut download_bytes = None;
    let mut total_bytes = None;
    let mut expire_at_ms = None;

    for field in value.split([';', ',']) {
        let Some((raw_key, raw_value)) = field.trim().split_once('=') else {
            continue;
        };
        let key = raw_key.trim().to_ascii_lowercase();
        let number = raw_value
            .trim()
            .parse::<u64>()
            .map_err(|_| ParseError::InvalidUserInfo(field.trim().to_string()))?;
        match key.as_str() {
            "upload" => upload_bytes = Some(number),
            "download" => download_bytes = Some(number),
            "total" => total_bytes = Some(number),
            "expire" => {
                let millis = number
                    .checked_mul(1_000)
                    .ok_or_else(|| ParseError::InvalidUserInfo(field.trim().to_string()))?;
                expire_at_ms = Some(
                    i64::try_from(millis)
                        .map_err(|_| ParseError::InvalidUserInfo(field.trim().to_string()))?,
                );
            }
            _ => {}
        }
    }

    Ok(SubscriptionUsage {
        upload_bytes,
        download_bytes,
        total_bytes,
        expire_at_ms,
    })
}

pub fn decode_subscription(input: &str) -> Result<SubscriptionDecode, ParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ParseError::EmptySubscription);
    }

    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return decode_json_subscription(trimmed);
    }

    if !looks_like_node_list(trimmed)
        && let Ok(decoded) = decode_base64_text(trimmed)
        && decoded != trimmed
    {
        if decoded.trim_start().starts_with('{') || decoded.trim_start().starts_with('[') {
            return decode_json_subscription(&decoded);
        }
        return decode_node_lines(&decoded);
    }

    decode_node_lines(trimmed)
}

fn parse_vless(url: &Url) -> Result<NodeConfig, ParseError> {
    let uuid = decode_component(url.username());
    if uuid.is_empty() {
        return Err(ParseError::MissingField("vless UUID"));
    }
    if uuid::Uuid::parse_str(&uuid).is_err() {
        return Err(ParseError::InvalidNode(
            "vless UUID is not valid".to_string(),
        ));
    }
    let (server, server_port) = server_and_port(url, 443)?;
    let params = QueryParams::new(url);
    let security = params.get("security").unwrap_or_else(|| "none".to_string());
    let tls = parse_tls(&params, &security)?;
    let transport = parse_transport(&params)?;
    let name = node_name(url, &server);

    Ok(NodeConfig::new(
        name,
        server,
        server_port,
        NodeProtocol::Vless {
            uuid,
            flow: params.get("flow"),
            tls,
            transport,
        },
    ))
}

fn parse_hysteria2(url: &Url) -> Result<NodeConfig, ParseError> {
    let (server, server_port) = server_and_port(url, 443)?;
    let params = QueryParams::new(url);
    let password = userinfo_secret(url).ok_or(ParseError::MissingField("hysteria2 password"))?;
    let mut tls = TlsSettings {
        enabled: true,
        ..TlsSettings::default()
    };
    tls.server_name = params.get("sni");
    tls.insecure = params.bool("insecure");

    let obfs = params.get("obfs").map(|kind| HysteriaObfs {
        kind,
        password: params.get("obfs-password").unwrap_or_default(),
    });
    let up_mbps = params.u32("upmbps").or_else(|| params.u32("up_mbps"));
    let down_mbps = params.u32("downmbps").or_else(|| params.u32("down_mbps"));
    let name = node_name(url, &server);

    Ok(NodeConfig::new(
        name,
        server,
        server_port,
        NodeProtocol::Hysteria2 {
            password,
            tls,
            obfs,
            up_mbps,
            down_mbps,
        },
    ))
}

fn parse_trojan(url: &Url) -> Result<NodeConfig, ParseError> {
    let (server, server_port) = server_and_port(url, 443)?;
    let params = QueryParams::new(url);
    let password = userinfo_secret(url).ok_or(ParseError::MissingField("trojan password"))?;
    let security = params.get("security").unwrap_or_else(|| "tls".to_string());
    let tls = parse_tls(&params, &security)?;
    let transport = parse_transport(&params)?;
    let name = node_name(url, &server);

    Ok(NodeConfig::new(
        name,
        server,
        server_port,
        NodeProtocol::Trojan {
            password,
            tls,
            transport,
        },
    ))
}

fn parse_shadowsocks(input: &str, url: &Url) -> Result<NodeConfig, ParseError> {
    let name = url
        .fragment()
        .map(decode_component)
        .filter(|value| !value.is_empty());
    let params = QueryParams::new(url);
    let plugin = params.get("plugin");
    let (plugin, plugin_opts) = split_plugin(plugin);

    if let (Some(host), Some(port)) = (url.host_str(), url.port()) {
        let encoded_userinfo = decode_component(url.username());
        if encoded_userinfo.is_empty() {
            return Err(ParseError::MissingField("shadowsocks SIP002 userinfo"));
        }
        let decoded_userinfo = decode_base64_text(&encoded_userinfo)?;
        let (method, password) = decoded_userinfo
            .split_once(':')
            .ok_or_else(|| ParseError::InvalidNode("shadowsocks credentials".to_string()))?;
        let name = name.unwrap_or_else(|| host.to_string());
        return Ok(NodeConfig::new(
            name,
            host.to_string(),
            port,
            NodeProtocol::Shadowsocks {
                method: method.to_string(),
                password: password.to_string(),
                plugin,
                plugin_opts,
            },
        ));
    }

    let raw_payload = input
        .strip_prefix("ss://")
        .ok_or_else(|| ParseError::InvalidNode("invalid shadowsocks URI".to_string()))?
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let decoded_payload = decode_base64_text(raw_payload)?;
    let (credentials, host_port) = decoded_payload
        .rsplit_once('@')
        .ok_or_else(|| ParseError::InvalidNode("legacy shadowsocks payload".to_string()))?;
    let (method, password) = credentials
        .split_once(':')
        .ok_or_else(|| ParseError::InvalidNode("shadowsocks credentials".to_string()))?;
    let host_url = Url::parse(&format!("ss://{host_port}"))?;
    let host = host_url
        .host_str()
        .ok_or(ParseError::MissingField("shadowsocks server"))?;
    let port = host_url
        .port()
        .ok_or(ParseError::MissingField("shadowsocks port"))?;
    let name = name.unwrap_or_else(|| host.to_string());

    Ok(NodeConfig::new(
        name,
        host.to_string(),
        port,
        NodeProtocol::Shadowsocks {
            method: method.to_string(),
            password: password.to_string(),
            plugin,
            plugin_opts,
        },
    ))
}

fn decode_json_subscription(input: &str) -> Result<SubscriptionDecode, ParseError> {
    let value: Value = serde_json::from_str(input)?;
    let mut entries = Vec::new();
    collect_json_entries(&value, &mut entries)?;
    decode_entries(entries)
}

fn collect_json_entries(value: &Value, entries: &mut Vec<String>) -> Result<(), ParseError> {
    match value {
        Value::String(value) => entries.push(value.clone()),
        Value::Array(values) => {
            for value in values {
                collect_json_entries(value, entries)?;
            }
        }
        Value::Object(object) => {
            if let Ok(node) = serde_json::from_value::<NodeConfig>(value.clone()) {
                entries.push(serde_json::to_string(&node)?);
                return Ok(());
            }
            let mut found = false;
            for key in ["nodes", "links", "uris", "outbounds"] {
                if let Some(value) = object.get(key) {
                    found = true;
                    collect_json_entries(value, entries)?;
                }
            }
            if !found {
                return Err(ParseError::InvalidNode(
                    "JSON subscription has no nodes, links, uris, or outbounds".to_string(),
                ));
            }
        }
        _ => {
            return Err(ParseError::InvalidNode(
                "JSON subscription entry must be a string or object".to_string(),
            ));
        }
    }
    Ok(())
}

fn decode_node_lines(input: &str) -> Result<SubscriptionDecode, ParseError> {
    let entries = input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    decode_entries(entries)
}

fn decode_entries(entries: Vec<String>) -> Result<SubscriptionDecode, ParseError> {
    let mut nodes = Vec::new();
    let mut skipped = Vec::new();
    for entry in entries {
        match parse_node_uri(&entry) {
            Ok(node) => nodes.push(node),
            Err(_) if entry.trim_start().starts_with('{') => {
                match serde_json::from_str::<NodeConfig>(&entry) {
                    Ok(node) => nodes.push(node),
                    Err(_) => skipped.push(entry),
                }
            }
            Err(_) => skipped.push(entry),
        }
    }
    if nodes.is_empty() {
        return Err(ParseError::EmptySubscription);
    }
    Ok(SubscriptionDecode {
        nodes,
        skipped,
        usage: None,
    })
}

fn parse_tls(params: &QueryParams<'_>, security: &str) -> Result<TlsSettings, ParseError> {
    match security.to_ascii_lowercase().as_str() {
        "none" | "" => Ok(TlsSettings::default()),
        "tls" => Ok(TlsSettings {
            enabled: true,
            server_name: params.get("sni"),
            insecure: params.bool("allowInsecure") || params.bool("insecure"),
            fingerprint: params.get("fp"),
            reality: None,
        }),
        "reality" => {
            let public_key = params
                .get("pbk")
                .ok_or(ParseError::MissingField("reality public key (pbk)"))?;
            let short_id = params
                .get("sid")
                .ok_or(ParseError::MissingField("reality short ID (sid)"))?;
            Ok(TlsSettings {
                enabled: true,
                server_name: params.get("sni"),
                insecure: params.bool("allowInsecure") || params.bool("insecure"),
                fingerprint: params.get("fp"),
                reality: Some(RealitySettings {
                    public_key,
                    short_id,
                    spider_x: params.get("spx"),
                }),
            })
        }
        other => Err(ParseError::InvalidNode(format!(
            "unsupported TLS security mode: {other}"
        ))),
    }
}

fn parse_transport(params: &QueryParams<'_>) -> Result<TransportConfig, ParseError> {
    match params.get("type").as_deref().unwrap_or("tcp") {
        "tcp" | "raw" => Ok(TransportConfig::Tcp),
        "ws" => Ok(TransportConfig::WebSocket {
            path: params.get("path").unwrap_or_else(|| "/".to_string()),
            host: params.get("host"),
        }),
        "grpc" => Ok(TransportConfig::Grpc {
            service_name: params
                .get("serviceName")
                .or_else(|| params.get("service_name"))
                .unwrap_or_default(),
        }),
        "httpupgrade" | "http-upgrade" => Ok(TransportConfig::HttpUpgrade {
            path: params.get("path").unwrap_or_else(|| "/".to_string()),
            host: params.get("host"),
        }),
        other => Err(ParseError::InvalidNode(format!(
            "unsupported transport type: {other}"
        ))),
    }
}

fn server_and_port(url: &Url, default_port: u16) -> Result<(String, u16), ParseError> {
    let server = url
        .host_str()
        .ok_or(ParseError::MissingField("server"))?
        .to_string();
    let port = url.port().unwrap_or(default_port);
    Ok((server, port))
}

fn node_name(url: &Url, server: &str) -> String {
    url.fragment()
        .map(decode_component)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| server.to_string())
}

fn userinfo_secret(url: &Url) -> Option<String> {
    let value = url.password().or_else(|| {
        let username = url.username();
        (!username.is_empty()).then_some(username)
    })?;
    let decoded = decode_component(value);
    (!decoded.is_empty()).then_some(decoded)
}

fn decode_component(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().into_owned()
}

fn decode_base64_text(value: &str) -> Result<String, ParseError> {
    let cleaned = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let bytes = STANDARD
        .decode(&cleaned)
        .or_else(|_| URL_SAFE.decode(&cleaned))
        .or_else(|_| URL_SAFE_NO_PAD.decode(&cleaned))
        .or_else(|_| {
            let mut padded = cleaned.clone();
            while padded.len() % 4 != 0 {
                padded.push('=');
            }
            STANDARD.decode(padded)
        })
        .map_err(|_| ParseError::InvalidBase64)?;
    String::from_utf8(bytes).map_err(|_| ParseError::InvalidUtf8)
}

fn looks_like_node_list(value: &str) -> bool {
    value.lines().any(|line| {
        let line = line.trim_start();
        ["vless://", "hy2://", "hysteria2://", "trojan://", "ss://"]
            .iter()
            .any(|prefix| line.starts_with(prefix))
    })
}

fn split_plugin(plugin: Option<String>) -> (Option<String>, Option<String>) {
    let Some(plugin) = plugin else {
        return (None, None);
    };
    let Some((name, options)) = plugin.split_once(';') else {
        return (Some(plugin), None);
    };
    (Some(name.to_string()), Some(options.to_string()))
}

struct QueryParams<'a> {
    url: &'a Url,
}

impl<'a> QueryParams<'a> {
    fn new(url: &'a Url) -> Self {
        Self { url }
    }

    fn get(&self, key: &str) -> Option<String> {
        self.url
            .query_pairs()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.into_owned())
            .filter(|value| !value.is_empty())
    }

    fn bool(&self, key: &str) -> bool {
        self.get(key)
            .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false)
    }

    fn u32(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(|value| value.parse().ok())
    }
}
