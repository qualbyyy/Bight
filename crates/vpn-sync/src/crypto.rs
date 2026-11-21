use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

const ENVELOPE_VERSION: u8 = 1;
const NONCE_SIZE: usize = 12;
const KEY_SIZE: usize = 32;
const HKDF_SALT: &[u8] = b"qonnect-sync-v1-x25519-chacha20poly1305";

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("random number generation failed: {0}")]
    Random(String),
    #[error("invalid base64 {field}")]
    InvalidBase64 { field: &'static str },
    #[error("invalid {field} length: expected {expected}, got {actual}")]
    InvalidLength {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("key derivation failed")]
    KeyDerivation,
    #[error("authenticated encryption failed")]
    Encryption,
    #[error("authenticated decryption failed")]
    Decryption,
    #[error("session ID does not match the encrypted envelope")]
    SessionMismatch,
    #[error("unsupported encrypted envelope version: {0}")]
    UnsupportedVersion(u8),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedEnvelope {
    pub version: u8,
    pub session_id: Uuid,
    pub nonce: String,
    pub ciphertext: String,
}

pub struct EphemeralKeyPair {
    secret: StaticSecret,
    public: PublicKey,
}

impl fmt::Debug for EphemeralKeyPair {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EphemeralKeyPair")
            .field("public", &self.public.to_bytes())
            .finish()
    }
}

impl EphemeralKeyPair {
    pub fn generate() -> Self {
        let secret = StaticSecret::random();
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn public_key(&self) -> [u8; KEY_SIZE] {
        self.public.to_bytes()
    }

    pub fn public_key_base64(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.public_key())
    }

    pub fn derive_cipher(
        &self,
        peer_public_key: [u8; KEY_SIZE],
        session_id: Uuid,
    ) -> Result<SessionCipher, CryptoError> {
        let peer_public = PublicKey::from(peer_public_key);
        let shared_secret = self.secret.diffie_hellman(&peer_public);
        let mut key = [0_u8; KEY_SIZE];
        Hkdf::<Sha256>::new(Some(HKDF_SALT), shared_secret.as_bytes())
            .expand(session_id.as_bytes(), &mut key)
            .map_err(|_| CryptoError::KeyDerivation)?;
        Ok(SessionCipher { key })
    }
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SessionCipher {
    key: [u8; KEY_SIZE],
}

impl SessionCipher {
    pub fn encrypt(
        &self,
        session_id: Uuid,
        plaintext: &[u8],
    ) -> Result<EncryptedEnvelope, CryptoError> {
        let mut nonce_bytes = [0_u8; NONCE_SIZE];
        getrandom::fill(&mut nonce_bytes)
            .map_err(|error| CryptoError::Random(error.to_string()))?;
        let cipher =
            ChaCha20Poly1305::new_from_slice(&self.key).map_err(|_| CryptoError::Encryption)?;
        let nonce = Nonce::try_from(nonce_bytes.as_slice()).map_err(|_| CryptoError::Encryption)?;
        let aad = associated_data(session_id);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError::Encryption)?;
        Ok(EncryptedEnvelope {
            version: ENVELOPE_VERSION,
            session_id,
            nonce: URL_SAFE_NO_PAD.encode(nonce_bytes),
            ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
        })
    }

    pub fn decrypt(
        &self,
        expected_session_id: Uuid,
        envelope: &EncryptedEnvelope,
    ) -> Result<Vec<u8>, CryptoError> {
        if envelope.version != ENVELOPE_VERSION {
            return Err(CryptoError::UnsupportedVersion(envelope.version));
        }
        if envelope.session_id != expected_session_id {
            return Err(CryptoError::SessionMismatch);
        }
        let nonce = decode_fixed::<NONCE_SIZE>(&envelope.nonce, "nonce")?;
        let ciphertext = URL_SAFE_NO_PAD.decode(&envelope.ciphertext).map_err(|_| {
            CryptoError::InvalidBase64 {
                field: "ciphertext",
            }
        })?;
        let cipher =
            ChaCha20Poly1305::new_from_slice(&self.key).map_err(|_| CryptoError::Decryption)?;
        let nonce = Nonce::try_from(nonce.as_slice()).map_err(|_| CryptoError::Decryption)?;
        let aad = associated_data(expected_session_id);
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext.as_slice(),
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError::Decryption)
    }
}

pub fn decode_public_key(value: &str) -> Result<[u8; KEY_SIZE], CryptoError> {
    decode_fixed(value, "public key")
}

fn decode_fixed<const N: usize>(value: &str, field: &'static str) -> Result<[u8; N], CryptoError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(value))
        .map_err(|_| CryptoError::InvalidBase64 { field })?;
    bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| CryptoError::InvalidLength {
            field,
            expected: N,
            actual: bytes.len(),
        })
}

fn associated_data(session_id: Uuid) -> Vec<u8> {
    let mut data = Vec::with_capacity(16 + 16);
    data.extend_from_slice(b"qonnect-sync-v1");
    data.extend_from_slice(session_id.as_bytes());
    data
}
