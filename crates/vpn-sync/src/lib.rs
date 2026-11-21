mod crypto;
mod discovery;
mod merge;
mod pairing;
mod transport;

pub use crypto::{CryptoError, EncryptedEnvelope, EphemeralKeyPair, SessionCipher};
pub use discovery::{MDNS_SERVICE_TYPE, MdnsError, MdnsPeer, MdnsRegistration, discover_mdns};
pub use merge::{MergeError, MergeReport, SyncPayload, merge_payloads, merge_profiles};
pub use pairing::{
    DEFAULT_SESSION_TTL, PAIRING_VERSION, PairingError, PairingPayload, SyncSession, now_ms,
};
pub use transport::{
    HealthBody, RunningSyncHost, SyncClient, SyncClientConfig, SyncHost, SyncHostConfig,
    SyncTransportError,
};
