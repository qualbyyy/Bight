use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use uuid::Uuid;
use vpn_core::{
    AppProfile, NodeConfig, NodeProtocol, ProfileMode, RoutingAction, RoutingMatch, RoutingRule,
    Subscription, TlsSettings, TransportConfig,
};
use vpn_sync::{
    EncryptedEnvelope, EphemeralKeyPair, PairingPayload, SyncClient, SyncClientConfig, SyncHost,
    SyncHostConfig, SyncPayload, SyncSession, merge_profiles, now_ms,
};

fn node(id: Uuid, name: &str, updated_at_ms: i64, is_deleted: bool) -> NodeConfig {
    NodeConfig {
        id,
        name: name.to_string(),
        server: "example.com".to_string(),
        server_port: 443,
        protocol: NodeProtocol::Trojan {
            password: "secret".to_string(),
            tls: TlsSettings {
                enabled: true,
                server_name: Some("example.com".to_string()),
                ..TlsSettings::default()
            },
            transport: TransportConfig::Tcp,
        },
        updated_at_ms,
        is_deleted,
    }
}

fn profile(id: Uuid, node: NodeConfig, updated_at_ms: i64) -> AppProfile {
    let active_node_id = node.id;
    AppProfile {
        id,
        name: "Default".to_string(),
        subscriptions: vec![Subscription {
            id: Uuid::from_u128(0x100),
            name: "Subscription".to_string(),
            url: "https://example.com/sub".to_string(),
            nodes: vec![node],
            traffic_used: None,
            traffic_total: None,
            expire_at: None,
            last_fetched_at: None,
            updated_at_ms,
            is_deleted: false,
        }],
        custom_nodes: Vec::new(),
        routing_rules: vec![RoutingRule {
            id: Uuid::from_u128(0x200),
            enabled: true,
            match_type: RoutingMatch::DomainSuffix,
            value: "example.com".to_string(),
            action: RoutingAction::Direct,
            comment: "local".to_string(),
            updated_at_ms,
            is_deleted: false,
        }],
        active_node_id: Some(active_node_id),
        mode: ProfileMode::Tun,
        updated_at_ms,
        is_deleted: false,
    }
}

#[test]
fn x25519_chacha20poly1305_round_trip_and_wrong_key_fail() {
    let alice = EphemeralKeyPair::generate();
    let bob = EphemeralKeyPair::generate();
    let session_id = Uuid::from_u128(0xabc);
    let alice_cipher = alice.derive_cipher(bob.public_key(), session_id).unwrap();
    let bob_cipher = bob.derive_cipher(alice.public_key(), session_id).unwrap();
    let envelope = alice_cipher.encrypt(session_id, b"private delta").unwrap();

    assert_eq!(
        bob_cipher.decrypt(session_id, &envelope).unwrap(),
        b"private delta"
    );
    assert!(
        bob_cipher
            .decrypt(Uuid::from_u128(0xdef), &envelope)
            .is_err()
    );

    let mallory = EphemeralKeyPair::generate();
    let mallory_cipher = mallory
        .derive_cipher(alice.public_key(), session_id)
        .unwrap();
    assert!(mallory_cipher.decrypt(session_id, &envelope).is_err());

    let malformed = EncryptedEnvelope {
        version: 1,
        session_id,
        nonce: "bad".to_string(),
        ciphertext: envelope.ciphertext.clone(),
    };
    assert!(bob_cipher.decrypt(session_id, &malformed).is_err());
}

#[test]
fn pairing_payload_round_trips_as_uri_and_rejects_bad_pin_or_expiry() {
    let session = SyncSession::new(Duration::from_secs(60));
    let payload = session.pairing_payload(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)], 4567);
    let uri = payload.to_uri().unwrap();
    let decoded = PairingPayload::from_uri(&uri).unwrap();
    assert_eq!(decoded, payload);
    assert!(decoded.validate_at(now_ms()).is_ok());
    assert!(decoded.verify_pin("000000", now_ms()).is_err());
    assert!(
        decoded
            .verify_pin(session.pin(), decoded.expires_at_ms + 1)
            .is_err()
    );
}

#[test]
fn merge_uses_timestamp_tombstones_and_deterministic_equal_timestamp_ties() {
    let profile_id = Uuid::from_u128(0x1);
    let record_id = Uuid::from_u128(0x2);
    let mut local = profile(profile_id, node(record_id, "local", 10, false), 10);
    let remote_newer = profile(profile_id, node(record_id, "remote", 20, false), 20);
    let report = merge_profiles(&mut local, &remote_newer).unwrap();
    assert!(report.updated > 0);
    assert_eq!(local.subscriptions[0].nodes[0].name, "remote");

    let remote_deleted = profile(profile_id, node(record_id, "remote", 30, true), 30);
    merge_profiles(&mut local, &remote_deleted).unwrap();
    assert!(local.subscriptions[0].nodes[0].is_deleted);

    let left = profile(profile_id, node(record_id, "aaa", 40, false), 40);
    let right = profile(profile_id, node(record_id, "zzz", 40, false), 40);
    let mut left_then_right = left.clone();
    let mut right_then_left = right.clone();
    merge_profiles(&mut left_then_right, &right).unwrap();
    merge_profiles(&mut right_then_left, &left).unwrap();
    assert_eq!(left_then_right, right_then_left);
    assert_eq!(left_then_right.subscriptions[0].nodes[0].name, "zzz");

    let payload = SyncPayload::from_profile(&left_then_right);
    assert_eq!(payload.profile, left_then_right);
}

#[tokio::test]
async fn local_http_exchange_merges_profiles_and_rejects_wrong_pin() {
    let profile_id = Uuid::from_u128(0x300);
    let host_profile = profile(
        profile_id,
        node(Uuid::from_u128(0x301), "host", 10, false),
        10,
    );
    let client_profile = profile(
        profile_id,
        node(Uuid::from_u128(0x302), "client", 20, false),
        20,
    );
    let host = SyncHost::start(
        host_profile,
        SyncHostConfig::new(SocketAddr::from(([127, 0, 0, 1], 0)))
            .with_ips(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]),
    )
    .await
    .unwrap();

    let mut wrong_pairing = host.pairing().clone();
    wrong_pairing.pin = "000000".to_string();
    let client = SyncClient::new(SyncClientConfig::default()).unwrap();
    let mut rejected_profile = client_profile.clone();
    assert!(
        client
            .exchange(&wrong_pairing, &mut rejected_profile)
            .await
            .is_err()
    );

    let mut client_profile = client_profile;
    let report = client
        .exchange(host.pairing(), &mut client_profile)
        .await
        .unwrap();
    assert!(report.added > 0 || report.updated > 0);
    assert!(
        client_profile
            .subscriptions
            .iter()
            .flat_map(|subscription| subscription.nodes.iter())
            .any(|item| item.name == "host")
    );

    let host_profile = host.profile().await;
    assert!(
        host_profile
            .subscriptions
            .iter()
            .flat_map(|subscription| subscription.nodes.iter())
            .any(|item| item.name == "client")
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn local_websocket_exchange_merges_profiles() {
    let profile_id = Uuid::from_u128(0x400);
    let host_profile = profile(
        profile_id,
        node(Uuid::from_u128(0x401), "host-ws", 10, false),
        10,
    );
    let mut client_profile = profile(
        profile_id,
        node(Uuid::from_u128(0x402), "client-ws", 20, false),
        20,
    );
    let host = SyncHost::start(
        host_profile,
        SyncHostConfig::new(SocketAddr::from(([127, 0, 0, 1], 0)))
            .with_ips(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]),
    )
    .await
    .unwrap();
    let client = SyncClient::new(SyncClientConfig::default()).unwrap();
    let report = client
        .exchange_websocket(host.pairing(), &mut client_profile)
        .await
        .unwrap();
    assert!(report.added > 0 || report.updated > 0);
    assert!(
        client_profile
            .subscriptions
            .iter()
            .flat_map(|subscription| subscription.nodes.iter())
            .any(|item| item.name == "host-ws")
    );
    host.shutdown().await.unwrap();
}

#[test]
fn expired_pairing_is_not_accepted() {
    let payload = PairingPayload {
        version: 1,
        ips: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
        port: 1234,
        public_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
        session_id: Uuid::new_v4(),
        pin: "123456".to_string(),
        expires_at_ms: now_ms() - 1,
    };
    assert!(payload.validate_at(now_ms()).is_err());
}
