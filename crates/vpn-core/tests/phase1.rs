use std::error::Error;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;
use uuid::Uuid;
use vpn_core::{
    AppProfile, BuildOptions, ClashApiClient, ClashApiError, NodeProtocol, ProfileMode,
    RoutingAction, RoutingMatch, Subscription, TargetPlatform, TlsSettings, TransportConfig,
    build_config, decode_subscription, parse_node_uri, parse_subscription_userinfo,
    validate_config,
};

#[test]
fn lww_models_serialize_all_merge_fields() -> Result<(), Box<dyn Error>> {
    let node = parse_node_uri("trojan://secret@example.com:443?sni=example.com#node")?;
    let subscription = Subscription {
        id: Uuid::new_v4(),
        name: "Main".to_string(),
        url: "https://subscription.example/list".to_string(),
        nodes: vec![node.clone()],
        traffic_used: Some(10),
        traffic_total: Some(100),
        expire_at: Some(1_700_000_000),
        last_fetched_at: Some(1_700_000_001),
        updated_at_ms: 20,
        is_deleted: false,
    };
    let profile = AppProfile {
        id: Uuid::new_v4(),
        name: "Default".to_string(),
        subscriptions: vec![subscription],
        custom_nodes: vec![node],
        routing_rules: vec![vpn_core::RoutingRule {
            id: Uuid::new_v4(),
            enabled: true,
            match_type: RoutingMatch::DomainSuffix,
            value: "example.com".to_string(),
            action: RoutingAction::Direct,
            comment: "local".to_string(),
            updated_at_ms: 21,
            is_deleted: false,
        }],
        active_node_id: None,
        mode: ProfileMode::Tun,
        updated_at_ms: 22,
        is_deleted: false,
    };

    let encoded = serde_json::to_value(profile)?;
    assert!(encoded["id"].is_string());
    assert_eq!(encoded["updated_at_ms"], 22);
    assert_eq!(encoded["is_deleted"], false);
    assert_eq!(encoded["subscriptions"][0]["updated_at_ms"], 20);
    assert_eq!(encoded["custom_nodes"][0]["protocol"], "trojan");
    Ok(())
}

#[test]
fn parses_vless_reality_with_websocket_transport() -> Result<(), Box<dyn Error>> {
    let node = parse_node_uri(
        "vless://8f3c2f5d-6f4e-4b1d-8f6c-1f2a3b4c5d6e@example.com:443?security=reality&sni=www.microsoft.com&fp=chrome&pbk=public-key&sid=short-id&spx=%2Fedge&type=ws&path=%2Fvless&host=cdn.example.com&flow=xtls-rprx-vision#Reality%20WS",
    )?;

    assert_eq!(node.name, "Reality WS");
    assert_eq!(node.server, "example.com");
    assert_eq!(node.server_port, 443);
    match node.protocol {
        NodeProtocol::Vless {
            uuid,
            flow,
            tls,
            transport,
        } => {
            assert_eq!(uuid, "8f3c2f5d-6f4e-4b1d-8f6c-1f2a3b4c5d6e");
            assert_eq!(flow.as_deref(), Some("xtls-rprx-vision"));
            assert_eq!(tls.server_name.as_deref(), Some("www.microsoft.com"));
            assert_eq!(tls.fingerprint.as_deref(), Some("chrome"));
            assert_eq!(tls.reality.as_ref().unwrap().public_key, "public-key");
            assert_eq!(tls.reality.as_ref().unwrap().short_id, "short-id");
            assert_eq!(
                tls.reality.as_ref().unwrap().spider_x.as_deref(),
                Some("/edge")
            );
            assert_eq!(
                transport,
                TransportConfig::WebSocket {
                    path: "/vless".to_string(),
                    host: Some("cdn.example.com".to_string()),
                }
            );
        }
        other => panic!("expected VLESS, got {other:?}"),
    }
    Ok(())
}

#[test]
fn parses_hysteria2_with_obfuscation_and_limits() -> Result<(), Box<dyn Error>> {
    let node = parse_node_uri(
        "hy2://p%40ss@example.com:443?sni=hy.example.com&insecure=1&obfs=salamander&obfs-password=obfs-pass&upmbps=50&downmbps=100#Hy2",
    )?;

    assert_eq!(node.name, "Hy2");
    match node.protocol {
        NodeProtocol::Hysteria2 {
            password,
            tls,
            obfs,
            up_mbps,
            down_mbps,
        } => {
            assert_eq!(password, "p@ss");
            assert_eq!(tls.server_name.as_deref(), Some("hy.example.com"));
            assert!(tls.insecure);
            assert_eq!(obfs.unwrap().password, "obfs-pass");
            assert_eq!(up_mbps, Some(50));
            assert_eq!(down_mbps, Some(100));
        }
        other => panic!("expected Hysteria2, got {other:?}"),
    }
    Ok(())
}

#[test]
fn parses_trojan_with_grpc_transport() -> Result<(), Box<dyn Error>> {
    let node = parse_node_uri(
        "trojan://p%40ss@example.com:443?security=tls&sni=trojan.example.com&fp=firefox&type=grpc&serviceName=trojan-grpc#Trojan",
    )?;

    match node.protocol {
        NodeProtocol::Trojan {
            password,
            tls,
            transport,
        } => {
            assert_eq!(password, "p@ss");
            assert_eq!(tls.server_name.as_deref(), Some("trojan.example.com"));
            assert_eq!(tls.fingerprint.as_deref(), Some("firefox"));
            assert_eq!(
                transport,
                TransportConfig::Grpc {
                    service_name: "trojan-grpc".to_string(),
                }
            );
        }
        other => panic!("expected Trojan, got {other:?}"),
    }
    Ok(())
}

#[test]
fn parses_shadowsocks_sip002_and_legacy_base64_forms() -> Result<(), Box<dyn Error>> {
    let sip002 = parse_node_uri(
        "ss://YWVzLTI1Ni1nY206cGFzcw@example.com:8388/?plugin=v2ray-plugin%3Bhost%3Dcdn.example.com#SIP002",
    )?;
    match sip002.protocol {
        NodeProtocol::Shadowsocks {
            method,
            password,
            plugin,
            plugin_opts,
        } => {
            assert_eq!(method, "aes-256-gcm");
            assert_eq!(password, "pass");
            assert_eq!(plugin.as_deref(), Some("v2ray-plugin"));
            assert_eq!(plugin_opts.as_deref(), Some("host=cdn.example.com"));
        }
        other => panic!("expected Shadowsocks, got {other:?}"),
    }

    let legacy_payload = STANDARD.encode("chacha20-ietf-poly1305:secret@legacy.example.com:1234");
    let legacy = parse_node_uri(&format!("ss://{legacy_payload}#Legacy"))?;
    assert_eq!(legacy.name, "Legacy");
    assert_eq!(legacy.server, "legacy.example.com");
    assert_eq!(legacy.server_port, 1234);
    Ok(())
}

#[test]
fn decodes_plain_base64_and_json_subscriptions_without_failing_on_bad_lines()
-> Result<(), Box<dyn Error>> {
    let vless = "vless://8f3c2f5d-6f4e-4b1d-8f6c-1f2a3b4c5d6e@example.com:443?security=tls&sni=example.com#VLESS";
    let trojan = "trojan://secret@example.net:443?sni=example.net#Trojan";

    let plain = decode_subscription(&format!("{vless}\nnot-a-node\n{trojan}"))?;
    assert_eq!(plain.nodes.len(), 2);
    assert_eq!(plain.skipped.len(), 1);

    let encoded = STANDARD.encode(format!("{vless}\n{trojan}"));
    let from_base64 = decode_subscription(&encoded)?;
    assert_eq!(from_base64.nodes.len(), 2);

    let from_json = decode_subscription(&serde_json::to_string(&vec![vless, trojan])?)?;
    assert_eq!(from_json.nodes.len(), 2);
    Ok(())
}

#[test]
fn parses_subscription_userinfo_header_in_bytes_and_milliseconds() -> Result<(), Box<dyn Error>> {
    let usage =
        parse_subscription_userinfo("upload=123; download=456; total=789; expire=1700000000")?;
    assert_eq!(usage.upload_bytes, Some(123));
    assert_eq!(usage.download_bytes, Some(456));
    assert_eq!(usage.total_bytes, Some(789));
    assert_eq!(usage.expire_at_ms, Some(1_700_000_000_000));
    Ok(())
}

#[test]
fn builds_modern_linux_tun_config_with_dns_protection_and_bypass_rules()
-> Result<(), Box<dyn Error>> {
    let node = parse_node_uri(
        "vless://8f3c2f5d-6f4e-4b1d-8f6c-1f2a3b4c5d6e@203.0.113.10:443?security=reality&sni=www.microsoft.com&pbk=public-key&sid=short-id&fp=chrome#Proxy",
    )?;
    let config = build_config(
        &node,
        BuildOptions::new(ProfileMode::Tun, TargetPlatform::Linux),
    )?;
    validate_config(&config)?;

    assert_eq!(config["dns"]["servers"][0]["server"], "1.1.1.1");
    assert!(config["dns"]["servers"][0].get("address").is_none());
    assert_eq!(config["dns"]["final"], "remote-dns");
    assert!(
        config["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "tun")
    );
    assert_eq!(
        config["inbounds"][0]["route_exclude_address"][0],
        "10.0.0.0/8"
    );
    assert!(config["route"]["auto_detect_interface"].is_boolean());
    assert!(
        config["route"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| {
                rule["ip_cidr"]
                    .as_array()
                    .map(|items| items.iter().any(|item| item == "203.0.113.10/32"))
                    .unwrap_or(false)
            })
    );
    assert!(
        config["experimental"]["clash_api"]["external_controller"]
            .as_str()
            .unwrap()
            .starts_with("127.0.0.1:")
    );
    Ok(())
}

#[test]
fn builds_system_proxy_without_tun_and_android_without_interface_autodetection()
-> Result<(), Box<dyn Error>> {
    let node = parse_node_uri("trojan://secret@proxy.example.com:443?sni=proxy.example.com#Proxy")?;
    let desktop = build_config(
        &node,
        BuildOptions::new(ProfileMode::SystemProxy, TargetPlatform::Windows),
    )?;
    validate_config(&desktop)?;
    assert!(
        desktop["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["type"] != "tun")
    );

    let android = build_config(
        &node,
        BuildOptions::new(ProfileMode::Tun, TargetPlatform::Android)
            .with_cache_file_path("/data/user/0/com.qonnect/files/cache.db".to_string()),
    )?;
    validate_config(&android)?;
    assert_eq!(
        android["experimental"]["cache_file"]["path"],
        "/data/user/0/com.qonnect/files/cache.db"
    );
    assert!(android["route"].get("auto_detect_interface").is_none());
    assert!(
        android["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["type"] == "tun" && item["route_exclude_address"].is_array() })
    );
    Ok(())
}

#[test]
fn rejects_legacy_dns_server_shape_and_forbidden_outbound_types() {
    let legacy_dns = json!({
        "dns": {"servers": [{"tag": "bad", "type": "https", "address": "1.1.1.1"}]},
        "inbounds": [],
        "outbounds": [{"type": "direct", "tag": "direct"}],
        "route": {"rules": [], "final": "direct"}
    });
    assert!(validate_config(&legacy_dns).is_err());

    let forbidden_outbound = json!({
        "dns": {"servers": [{"tag": "ok", "type": "local"}]},
        "inbounds": [],
        "outbounds": [{"type": "dns", "tag": "bad"}],
        "route": {"rules": [], "final": "bad"}
    });
    assert!(validate_config(&forbidden_outbound).is_err());
}

#[tokio::test]
async fn clash_api_client_reads_delay_and_active_connections() -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let server = tokio::spawn(async move {
        for response_body in [
            r#"{"delay":123}"#,
            r#"{"downloadTotal":456,"uploadTotal":789,"connections":[{"id":"c1","metadata":{"host":"example.com"}}]}"#,
        ] {
            let (mut stream, _) = listener.accept().await?;
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).await?;
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        Ok::<(), std::io::Error>(())
    });

    let client = ClashApiClient::new(port, Some("secret".to_string()))?;
    assert_eq!(
        client
            .check_delay("proxy", "https://example.com", 5000)
            .await?,
        123
    );
    let connections = client.active_connections().await?;
    assert_eq!(connections.download_total, 456);
    assert_eq!(connections.upload_total, 789);
    assert_eq!(connections.connections.len(), 1);
    server.await??;
    Ok(())
}

#[test]
fn tls_settings_default_to_disabled_without_accidental_legacy_fields() {
    let tls = TlsSettings::default();
    assert!(!tls.enabled);
    assert!(tls.reality.is_none());
}

#[test]
fn generated_json_is_serializable_and_contains_no_null_schema_artifacts()
-> Result<(), Box<dyn Error>> {
    let node = parse_node_uri("ss://YWVzLTI1Ni1nY206cGFzcw@example.com:8388#SS")?;
    let config = build_config(
        &node,
        BuildOptions::new(ProfileMode::Tun, TargetPlatform::Linux),
    )?;
    let serialized = serde_json::to_string(&config)?;
    let reparsed: Value = serde_json::from_str(&serialized)?;
    assert_eq!(reparsed["outbounds"][0]["type"], "shadowsocks");
    assert!(
        reparsed["dns"]["servers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|server| server.get("address").is_none())
    );
    Ok(())
}

#[test]
fn generated_reality_tls_omits_optional_null_fields() -> Result<(), Box<dyn Error>> {
    let node = parse_node_uri(
        "vless://8f3c2f5d-6f4e-4b1d-8f6c-1f2a3b4c5d6e@203.0.113.10:443?security=reality&sni=www.microsoft.com&pbk=public-key&sid=short-id&fp=chrome#Proxy",
    )?;
    let config = build_config(
        &node,
        BuildOptions::new(ProfileMode::Tun, TargetPlatform::Linux),
    )?;
    assert!(
        config["outbounds"][0]["tls"]["reality"]
            .get("spider_x")
            .is_none()
    );
    Ok(())
}

#[test]
fn clash_api_rejects_unsupported_base_url_scheme() -> Result<(), Box<dyn Error>> {
    let result = ClashApiClient::with_base_url(Url::parse("ftp://127.0.0.1:9090/")?, None);
    assert!(matches!(result, Err(ClashApiError::UnsupportedScheme(_))));
    Ok(())
}

#[test]
fn android_builder_rejects_relative_cache_file_path() -> Result<(), Box<dyn Error>> {
    let node = parse_node_uri("trojan://secret@proxy.example.com:443?sni=proxy.example.com#Proxy")?;
    let result = build_config(
        &node,
        BuildOptions::new(ProfileMode::Tun, TargetPlatform::Android),
    );
    assert!(result.is_err());
    Ok(())
}
