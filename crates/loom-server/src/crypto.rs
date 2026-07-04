//! Envelope encryption for provider credentials at rest.
//!
//! Provider API keys are encrypted with AES-256-GCM under the gateway's
//! [`LOOM_ENCRYPTION_KEY`](crate::config). Each ciphertext carries a fresh
//! random 96-bit nonce; the nonce is stored alongside the ciphertext (it is not
//! secret) and both are persisted via the store's `CredentialStore`. GCM's
//! authentication tag detects tampering on decrypt.

use aes_gcm::aead::{Aead, KeyInit, Payload};
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
    /// `aad` is bound into the AEAD as associated data: it is authenticated but
    /// not encrypted, so the resulting ciphertext only decrypts when the same
    /// `aad` is supplied to [`decrypt`](Self::decrypt). Callers use this to bind
    /// a ciphertext to the identity of the row it belongs to (for credentials,
    /// `"{tenant_id}:{provider}"`), so relocating the ciphertext into another
    /// row causes decryption to fail rather than silently succeed. Pass `b""`
    /// when no binding is required.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Encrypt`] if the underlying AEAD fails (which in
    /// practice only happens on absurdly large inputs).
    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<EncryptedSecret, CryptoError> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| CryptoError::Encrypt)?;
        Ok(EncryptedSecret {
            nonce: nonce_bytes.to_vec(),
            ciphertext,
        })
    }

    /// Decrypts a `(nonce, ciphertext)` pair produced by [`Crypto::encrypt`].
    ///
    /// `aad` must be byte-identical to the associated data supplied at
    /// encryption time; otherwise authentication fails and this returns
    /// [`CryptoError::Decrypt`]. This is what makes a confused-deputy row swap
    /// (moving one row's ciphertext into another row) fail closed.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Decrypt`] if the nonce length is wrong or the
    /// authentication tag does not verify (wrong key, wrong `aad`, or tampered
    /// ciphertext).
    pub fn decrypt(
        &self,
        nonce: &[u8],
        ciphertext: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        if nonce.len() != NONCE_LEN {
            return Err(CryptoError::Decrypt);
        }
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
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
        let aad = b"tenant:anthropic";
        let enc = crypto.encrypt(secret, aad).unwrap();
        assert_ne!(enc.ciphertext, secret);
        assert_eq!(enc.nonce.len(), NONCE_LEN);
        let dec = crypto.decrypt(&enc.nonce, &enc.ciphertext, aad).unwrap();
        assert_eq!(dec, secret);
    }

    #[test]
    fn distinct_nonces_per_encryption() {
        let crypto = Crypto::new([9u8; 32]);
        let a = crypto.encrypt(b"same", b"").unwrap();
        let b = crypto.encrypt(b"same", b"").unwrap();
        // Random nonces mean identical plaintext yields distinct ciphertext.
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let enc = Crypto::new([1u8; 32]).encrypt(b"secret", b"").unwrap();
        let err = Crypto::new([2u8; 32])
            .decrypt(&enc.nonce, &enc.ciphertext, b"")
            .unwrap_err();
        assert!(matches!(err, CryptoError::Decrypt));
    }

    #[test]
    fn wrong_aad_fails_to_decrypt() {
        // Encrypting under one row's identity and decrypting under another's
        // (the confused-deputy row-swap) must fail closed, not panic.
        let crypto = Crypto::new([3u8; 32]);
        let enc = crypto.encrypt(b"secret", b"tenant-a:anthropic").unwrap();
        let err = crypto
            .decrypt(&enc.nonce, &enc.ciphertext, b"tenant-b:anthropic")
            .unwrap_err();
        assert!(matches!(err, CryptoError::Decrypt));
        // The correct aad still decrypts.
        let dec = crypto
            .decrypt(&enc.nonce, &enc.ciphertext, b"tenant-a:anthropic")
            .unwrap();
        assert_eq!(dec, b"secret");
    }
}
