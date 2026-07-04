//! Envelope encryption for provider credentials at rest.
//!
//! Provider API keys are encrypted with AES-256-GCM under the gateway's
//! [`LOOM_ENCRYPTION_KEY`](crate::config). Each ciphertext carries a fresh
//! random 96-bit nonce; the nonce is stored alongside the ciphertext (it is not
//! secret) and both are persisted via the store's `CredentialStore`. GCM's
//! authentication tag detects tampering on decrypt.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;

/// The AES-GCM nonce length in bytes (96 bits, the standard for GCM).
const NONCE_LEN: usize = 12;

/// A symmetric encryptor for credentials at rest.
///
/// Cheap to clone (it holds only the 32-byte key). The key never appears in the
/// [`Debug`] representation.
#[derive(Clone)]
pub struct Crypto {
    key: [u8; 32],
}

impl Crypto {
    /// Builds an encryptor from a 32-byte AES-256 key.
    #[must_use]
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// Encrypts `plaintext`, returning a fresh nonce and the ciphertext (which
    /// includes the GCM authentication tag).
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Encrypt`] if the underlying AEAD fails (which in
    /// practice only happens on absurdly large inputs).
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedSecret, CryptoError> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| CryptoError::Encrypt)?;
        Ok(EncryptedSecret {
            nonce: nonce_bytes.to_vec(),
            ciphertext,
        })
    }

    /// Decrypts a `(nonce, ciphertext)` pair produced by [`Crypto::encrypt`].
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Decrypt`] if the nonce length is wrong or the
    /// authentication tag does not verify (wrong key, or tampered ciphertext).
    pub fn decrypt(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if nonce.len() != NONCE_LEN {
            return Err(CryptoError::Decrypt);
        }
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| CryptoError::Decrypt)
    }
}

impl std::fmt::Debug for Crypto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Crypto")
            .field("key", &"<redacted>")
            .finish()
    }
}

/// The output of [`Crypto::encrypt`]: a nonce and its ciphertext.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedSecret {
    /// The random 96-bit nonce (not secret; store it with the ciphertext).
    pub nonce: Vec<u8>,
    /// The ciphertext, including the trailing GCM authentication tag.
    pub ciphertext: Vec<u8>,
}

/// An error from the credential encryption layer.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Encryption failed.
    #[error("failed to encrypt credential")]
    Encrypt,

    /// Decryption or authentication failed (wrong key or tampered ciphertext).
    #[error("failed to decrypt credential")]
    Decrypt,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_plaintext() {
        let crypto = Crypto::new([7u8; 32]);
        let secret = b"sk-ant-super-secret";
        let enc = crypto.encrypt(secret).unwrap();
        assert_ne!(enc.ciphertext, secret);
        assert_eq!(enc.nonce.len(), NONCE_LEN);
        let dec = crypto.decrypt(&enc.nonce, &enc.ciphertext).unwrap();
        assert_eq!(dec, secret);
    }

    #[test]
    fn distinct_nonces_per_encryption() {
        let crypto = Crypto::new([9u8; 32]);
        let a = crypto.encrypt(b"same").unwrap();
        let b = crypto.encrypt(b"same").unwrap();
        // Random nonces mean identical plaintext yields distinct ciphertext.
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let enc = Crypto::new([1u8; 32]).encrypt(b"secret").unwrap();
        let err = Crypto::new([2u8; 32])
            .decrypt(&enc.nonce, &enc.ciphertext)
            .unwrap_err();
        assert!(matches!(err, CryptoError::Decrypt));
    }
}
