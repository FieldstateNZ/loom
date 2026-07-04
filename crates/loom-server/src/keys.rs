//! Virtual API key generation and hashing.
//!
//! # Why a peppered HMAC rather than a slow salted hash
//!
//! Virtual keys are minted by the gateway with 256 bits of entropy from a CSPRNG
//! (see [`generate_key`]). A password KDF such as argon2 exists to make *low*
//! entropy secrets expensive to brute-force — irrelevant here, because guessing
//! a 256-bit random key is already infeasible. Worse, a per-row salted KDF would
//! force an O(n) scan on every authentication (each stored hash uses a different
//! salt), destroying the O(1) `key_hash` lookup the store is built around.
//!
//! So we store a deterministic keyed hash: `key_hash = HMAC-SHA256(pepper, key)`
//! (hex). It is fast, constant per key (enabling a unique-indexed lookup), and
//! the server-side pepper means a database-only compromise still cannot recover
//! or forge keys without also exfiltrating the pepper (which lives only in the
//! process environment, derived from `LOOM_ENCRYPTION_KEY` when not set
//! explicitly).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// The number of random bytes behind a virtual key (256 bits of entropy).
const KEY_ENTROPY_BYTES: usize = 32;

/// The number of characters of the random body retained in the display prefix.
const PREFIX_BODY_CHARS: usize = 6;

/// The environment label embedded in a virtual key (`loom_<env>_...`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEnv {
    /// A production key (`loom_live_...`).
    Live,
    /// A non-production / sandbox key (`loom_test_...`).
    Test,
}

impl KeyEnv {
    /// The wire label used in the key string and prefix.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            KeyEnv::Live => "live",
            KeyEnv::Test => "test",
        }
    }

    /// Parses a label, accepting `"live"`/`"test"` case-insensitively.
    ///
    /// # Errors
    ///
    /// Returns the offending string when it is neither `live` nor `test`.
    pub fn parse(label: &str) -> Result<Self, String> {
        match label.trim().to_ascii_lowercase().as_str() {
            "live" => Ok(KeyEnv::Live),
            "test" => Ok(KeyEnv::Test),
            other => Err(other.to_owned()),
        }
    }
}

/// A freshly generated virtual key: the plaintext secret and its display prefix.
///
/// The [`secret`](Self::secret) is shown to the caller exactly once at creation
/// and is never persisted; only its [`KeyHasher`] hash and the non-secret
/// [`prefix`](Self::prefix) are stored.
#[derive(Clone, Debug)]
pub struct GeneratedKey {
    /// The full plaintext key, e.g. `loom_live_<43-char-base64url>`.
    pub secret: String,
    /// A short, non-secret prefix for display, e.g. `loom_live_AbC123`.
    pub prefix: String,
}

/// Generates a new virtual key with 256 bits of entropy for the given
/// environment.
#[must_use]
pub fn generate_key(env: KeyEnv) -> GeneratedKey {
    let mut bytes = [0u8; KEY_ENTROPY_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let body = URL_SAFE_NO_PAD.encode(bytes);
    let label = env.as_str();
    let secret = format!("loom_{label}_{body}");
    // 32 bytes base64url encodes to 43 chars, so slicing 6 is always in bounds.
    let prefix = format!("loom_{label}_{}", &body[..PREFIX_BODY_CHARS]);
    GeneratedKey { secret, prefix }
}

/// Computes deterministic, peppered lookup hashes for virtual keys.
///
/// Cheap to clone (holds only the pepper). The pepper never appears in the
/// [`Debug`] representation.
#[derive(Clone)]
pub struct KeyHasher {
    pepper: Vec<u8>,
}

impl KeyHasher {
    /// Builds a hasher from the server pepper.
    #[must_use]
    pub fn new(pepper: Vec<u8>) -> Self {
        Self { pepper }
    }

    /// Returns `hex(HMAC-SHA256(pepper, key))`, the value stored and looked up in
    /// `virtual_keys.key_hash`.
    #[must_use]
    pub fn hash(&self, key: &str) -> String {
        // HMAC accepts a key of any length, so this never fails.
        let mut mac =
            HmacSha256::new_from_slice(&self.pepper).expect("HMAC accepts keys of any length");
        mac.update(key.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}

impl std::fmt::Debug for KeyHasher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyHasher")
            .field("pepper", &"<redacted>")
            .finish()
    }
}

/// Derives a stable pepper from the encryption key when `LOOM_KEY_PEPPER` is
/// unset.
///
/// Uses `HMAC-SHA256(encryption_key, "loom.virtual-key.pepper.v1")` so the
/// pepper is domain-separated from the encryption key itself while remaining
/// deterministic across restarts. Rotating `LOOM_ENCRYPTION_KEY` therefore also
/// rotates the derived pepper (invalidating existing key hashes); set
/// `LOOM_KEY_PEPPER` explicitly to decouple the two lifecycles.
#[must_use]
pub fn derive_pepper(encryption_key: &[u8; 32]) -> Vec<u8> {
    let mut mac =
        HmacSha256::new_from_slice(encryption_key).expect("HMAC accepts keys of any length");
    mac.update(b"loom.virtual-key.pepper.v1");
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_has_expected_shape() {
        let key = generate_key(KeyEnv::Live);
        assert!(key.secret.starts_with("loom_live_"));
        assert!(key.prefix.starts_with("loom_live_"));
        assert_eq!(key.prefix.len(), "loom_live_".len() + PREFIX_BODY_CHARS);
        assert!(key.secret.starts_with(&key.prefix));
    }

    #[test]
    fn test_env_label() {
        assert!(generate_key(KeyEnv::Test).secret.starts_with("loom_test_"));
    }

    #[test]
    fn keys_are_unique() {
        assert_ne!(
            generate_key(KeyEnv::Live).secret,
            generate_key(KeyEnv::Live).secret
        );
    }

    #[test]
    fn hash_is_deterministic_and_not_plaintext() {
        let hasher = KeyHasher::new(b"pepper".to_vec());
        let key = "loom_live_abc";
        assert_eq!(hasher.hash(key), hasher.hash(key));
        assert_ne!(hasher.hash(key), key);
    }

    #[test]
    fn hash_depends_on_pepper() {
        let a = KeyHasher::new(b"pepper-a".to_vec());
        let b = KeyHasher::new(b"pepper-b".to_vec());
        assert_ne!(a.hash("loom_live_abc"), b.hash("loom_live_abc"));
    }

    #[test]
    fn derived_pepper_is_deterministic() {
        let key = [3u8; 32];
        assert_eq!(derive_pepper(&key), derive_pepper(&key));
        assert_ne!(derive_pepper(&key), derive_pepper(&[4u8; 32]));
    }

    #[test]
    fn parse_env_labels() {
        assert_eq!(KeyEnv::parse("LIVE").unwrap(), KeyEnv::Live);
        assert_eq!(KeyEnv::parse(" test ").unwrap(), KeyEnv::Test);
        assert!(KeyEnv::parse("prod").is_err());
    }
}
