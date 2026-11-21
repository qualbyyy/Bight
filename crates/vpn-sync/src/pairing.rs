use std::{
    net::IpAddr,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::crypto::{CryptoError, EphemeralKeyPair, decode_public_key};

pub const PAIRING_VERSION: u8 = 1;
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(180);

#[derive(Debug, Error)]
pub enum PairingError {
    #[error("invalid pairing payload: {0}")]
    InvalidPayload(String),
    #[error("pairing payload has expired")]
    Expired,
    #[error("invalid pairing URI: {0}")]
    InvalidUri(#[from] url::ParseError),
    #[error("invalid pairing JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("cryptographic error: {0}")]
    Crypto(#[from] CryptoError),
    #[error("random number generation failed: {0}")]
    Random(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairingPayload {
    pub version: u8,
    pub ips: Vec<IpAddr>,
    pub port: u16,
    pub public_key: String,
    pub session_id: Uuid,
    pub pin: String,
    pub expires_at_ms: i64,
}

impl PairingPayload {
    pub fn new(
        ips: Vec<IpAddr>,
        port: u16,
        public_key: String,
        session_id: Uuid,
        pin: String,
        expires_at_ms: i64,
    ) -> Self {
        Self {
            version: PAIRING_VERSION,
            ips,
            port,
            public_key,
            session_id,
            pin,
            expires_at_ms,
        }
    }

    pub fn to_json(&self) -> Result<String, PairingError> {
        self.validate_syntax()?;
        Ok(serde_json::to_string(self)?)
    }

    pub fn from_json(value: &str) -> Result<Self, PairingError> {
        let payload: Self = serde_json::from_str(value)?;
        payload.validate_syntax()?;
        Ok(payload)
    }

    pub fn to_uri(&self) -> Result<String, PairingError> {
        self.validate_syntax()?;
        let mut uri = Url::parse("qonnect://sync/v1")?;
        let ips = self
            .ips
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        uri.query_pairs_mut()
            .append_pair("ips", &ips)
            .append_pair("port", &self.port.to_string())
            .append_pair("pk", &self.public_key)
            .append_pair("sid", &self.session_id.to_string())
            .append_pair("pin", &self.pin)
            .append_pair("exp", &self.expires_at_ms.to_string());
        Ok(uri.to_string())
    }

    pub fn from_uri(value: &str) -> Result<Self, PairingError> {
        let uri = Url::parse(value)?;
        if uri.scheme() != "qonnect" || uri.host_str() != Some("sync") || uri.path() != "/v1" {
            return Err(PairingError::InvalidPayload(
                "expected qonnect://sync/v1 pairing URI".to_string(),
            ));
        }
        let query = uri.query_pairs().collect::<Vec<_>>();
        let get = |key: &str| {
            query
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.to_string())
        };
        let ips = get("ips")
            .ok_or_else(|| PairingError::InvalidPayload("missing ips".to_string()))?
            .split(',')
            .filter(|value| !value.is_empty())
            .map(IpAddr::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| PairingError::InvalidPayload("invalid IP address".to_string()))?;
        let payload = Self {
            version: PAIRING_VERSION,
            ips,
            port: get("port")
                .ok_or_else(|| PairingError::InvalidPayload("missing port".to_string()))?
                .parse()
                .map_err(|_| PairingError::InvalidPayload("invalid port".to_string()))?,
            public_key: get("pk")
                .ok_or_else(|| PairingError::InvalidPayload("missing public key".to_string()))?,
            session_id: get("sid")
                .ok_or_else(|| PairingError::InvalidPayload("missing session ID".to_string()))?
                .parse()
                .map_err(|_| PairingError::InvalidPayload("invalid session ID".to_string()))?,
            pin: get("pin")
                .ok_or_else(|| PairingError::InvalidPayload("missing PIN".to_string()))?,
            expires_at_ms: get("exp")
                .ok_or_else(|| PairingError::InvalidPayload("missing expiry".to_string()))?
                .parse()
                .map_err(|_| PairingError::InvalidPayload("invalid expiry".to_string()))?,
        };
        payload.validate_syntax()?;
        Ok(payload)
    }

    pub fn validate_syntax(&self) -> Result<(), PairingError> {
        if self.version != PAIRING_VERSION {
            return Err(PairingError::InvalidPayload(format!(
                "unsupported pairing version: {}",
                self.version
            )));
        }
        if self.ips.is_empty() {
            return Err(PairingError::InvalidPayload(
                "at least one IP address is required".to_string(),
            ));
        }
        if self.port == 0 {
            return Err(PairingError::InvalidPayload(
                "port must not be zero".to_string(),
            ));
        }
        decode_public_key(&self.public_key)?;
        if self.pin.len() != 6 || !self.pin.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(PairingError::InvalidPayload(
                "PIN must contain exactly six digits".to_string(),
            ));
        }
        if self.expires_at_ms <= 0 {
            return Err(PairingError::InvalidPayload(
                "expiry must be a positive Unix timestamp".to_string(),
            ));
        }
        Ok(())
    }

    pub fn validate_at(&self, now_ms: i64) -> Result<(), PairingError> {
        self.validate_syntax()?;
        if now_ms >= self.expires_at_ms {
            return Err(PairingError::Expired);
        }
        Ok(())
    }

    pub fn verify_pin(&self, pin: &str, now_ms: i64) -> Result<(), PairingError> {
        self.validate_at(now_ms)?;
        if self.pin != pin {
            return Err(PairingError::InvalidPayload("invalid PIN".to_string()));
        }
        Ok(())
    }

    pub fn public_key_bytes(&self) -> Result<[u8; 32], PairingError> {
        Ok(decode_public_key(&self.public_key)?)
    }
}

#[derive(Debug)]
pub struct SyncSession {
    key_pair: EphemeralKeyPair,
    session_id: Uuid,
    pin: String,
    expires_at_ms: i64,
}

impl SyncSession {
    pub fn new(ttl: Duration) -> Self {
        Self::try_new(ttl).expect("secure random source unavailable")
    }

    pub fn try_new(ttl: Duration) -> Result<Self, PairingError> {
        let millis = i64::try_from(ttl.as_millis())
            .map_err(|_| PairingError::InvalidPayload("session TTL is too large".to_string()))?;
        let expires_at_ms = now_ms()
            .checked_add(millis)
            .ok_or_else(|| PairingError::InvalidPayload("session expiry overflow".to_string()))?;
        Ok(Self {
            key_pair: EphemeralKeyPair::generate(),
            session_id: Uuid::new_v4(),
            pin: generate_pin()?,
            expires_at_ms,
        })
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    pub fn pin(&self) -> &str {
        &self.pin
    }

    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }

    pub fn pairing_payload(&self, ips: Vec<IpAddr>, port: u16) -> PairingPayload {
        PairingPayload::new(
            ips,
            port,
            self.key_pair.public_key_base64(),
            self.session_id,
            self.pin.clone(),
            self.expires_at_ms,
        )
    }

    pub fn validate_pin(&self, pin: &str, now_ms: i64) -> Result<(), PairingError> {
        if now_ms >= self.expires_at_ms {
            return Err(PairingError::Expired);
        }
        if self.pin != pin {
            return Err(PairingError::InvalidPayload("invalid PIN".to_string()));
        }
        Ok(())
    }

    pub fn derive_cipher(
        &self,
        peer_public_key: [u8; 32],
        now_ms: i64,
    ) -> Result<crate::crypto::SessionCipher, PairingError> {
        if now_ms >= self.expires_at_ms {
            return Err(PairingError::Expired);
        }
        Ok(self
            .key_pair
            .derive_cipher(peer_public_key, self.session_id)?)
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

fn generate_pin() -> Result<String, PairingError> {
    let mut bytes = [0_u8; 4];
    getrandom::fill(&mut bytes).map_err(|error| PairingError::Random(error.to_string()))?;
    let value = u32::from_le_bytes(bytes) % 1_000_000;
    Ok(format!("{value:06}"))
}
