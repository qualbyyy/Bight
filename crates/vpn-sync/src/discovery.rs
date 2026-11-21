use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
    time::{Duration, Instant},
};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use thiserror::Error;

use crate::pairing::{PairingError, PairingPayload};

pub const MDNS_SERVICE_TYPE: &str = "_qonnect-sync._tcp.local.";

#[derive(Debug, Error)]
pub enum MdnsError {
    #[error("mDNS error: {0}")]
    Daemon(#[from] mdns_sd::Error),
    #[error("mDNS receiver error: {0}")]
    Receiver(String),
    #[error("invalid advertised Qonnect pairing: {0}")]
    Pairing(#[from] PairingError),
    #[error("mDNS service has no usable IPv4 address")]
    MissingAddress,
}

#[derive(Debug, Clone)]
pub struct MdnsPeer {
    pub instance_name: String,
    pub pairing: PairingPayload,
}

pub struct MdnsRegistration {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsRegistration {
    pub fn register(pairing: &PairingPayload, instance_name: &str) -> Result<Self, MdnsError> {
        pairing.validate_syntax()?;
        let daemon = ServiceDaemon::new()?;
        let addresses = pairing
            .ips
            .iter()
            .filter(|ip| matches!(ip, IpAddr::V4(value) if !value.is_unspecified()))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(MdnsError::MissingAddress);
        }
        let session_id = pairing.session_id.to_string();
        let expiry = pairing.expires_at_ms.to_string();
        let properties = [
            ("v", pairing.version.to_string()),
            ("pk", pairing.public_key.clone()),
            ("sid", session_id),
            ("pin", pairing.pin.clone()),
            ("exp", expiry),
        ];
        let host_name = format!("{instance_name}.local.");
        let service = ServiceInfo::new(
            MDNS_SERVICE_TYPE,
            instance_name,
            &host_name,
            addresses.join(","),
            pairing.port,
            &properties[..],
        )?;
        let fullname = service.get_fullname().to_string();
        daemon.register(service)?;
        Ok(Self { daemon, fullname })
    }

    pub fn shutdown(self) -> Result<(), MdnsError> {
        let receiver = self.daemon.unregister(&self.fullname)?;
        let _ = receiver.recv_timeout(Duration::from_secs(2));
        self.daemon.shutdown()?;
        Ok(())
    }
}

impl Drop for MdnsRegistration {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

pub async fn discover_mdns(timeout: Duration) -> Result<Vec<MdnsPeer>, MdnsError> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(MDNS_SERVICE_TYPE)?;
    let deadline = Instant::now() + timeout;
    let mut peers = BTreeMap::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let event = match tokio::time::timeout(remaining, receiver.recv_async()).await {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => return Err(MdnsError::Receiver(error.to_string())),
            Err(_) => break,
        };
        let ServiceEvent::ServiceResolved(service) = event else {
            continue;
        };
        let Some(pairing) = pairing_from_service(&service)? else {
            continue;
        };
        let instance_name = service
            .fullname
            .split("._qonnect-sync._tcp.local.")
            .next()
            .unwrap_or(&service.fullname)
            .to_string();
        peers.insert(
            pairing.session_id,
            MdnsPeer {
                instance_name,
                pairing,
            },
        );
    }

    let _ = daemon.stop_browse(MDNS_SERVICE_TYPE);
    daemon.shutdown()?;
    Ok(peers.into_values().collect())
}

fn pairing_from_service(
    service: &mdns_sd::ResolvedService,
) -> Result<Option<PairingPayload>, MdnsError> {
    let Some(public_key) = service.get_property_val_str("pk") else {
        return Ok(None);
    };
    let Some(session_id) = service.get_property_val_str("sid") else {
        return Ok(None);
    };
    let Some(pin) = service.get_property_val_str("pin") else {
        return Ok(None);
    };
    let Some(expiry) = service.get_property_val_str("exp") else {
        return Ok(None);
    };
    let ips = service
        .get_addresses_v4()
        .into_iter()
        .filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
        .map(IpAddr::V4)
        .collect::<Vec<_>>();
    if ips.is_empty() {
        return Ok(None);
    }
    let payload = PairingPayload::new(
        ips,
        service.port,
        public_key.to_string(),
        session_id
            .parse()
            .map_err(|_| PairingError::InvalidPayload("invalid mDNS session ID".to_string()))?,
        pin.to_string(),
        expiry
            .parse()
            .map_err(|_| PairingError::InvalidPayload("invalid mDNS expiry".to_string()))?,
    );
    payload.validate_at(crate::now_ms())?;
    Ok(Some(payload))
}

#[allow(dead_code)]
fn is_valid_mdns_ipv4(ip: Ipv4Addr) -> bool {
    !ip.is_unspecified() && !ip.is_loopback()
}
