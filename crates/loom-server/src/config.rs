//! Startup configuration loaded from the process environment.
//!
//! Every secret is validated eagerly at boot so the server fails fast with a
//! clear diagnostic rather than 500-ing on the first request. Secrets never
//! appear in the [`Debug`] representation.

use std::net::SocketAddr;

use crate::keys::derive_pepper;

/// The default address the server binds when `LOOM_BIND_ADDR` is unset.
pub const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

/// Environment variable names read at startup.
mod env_keys {
    /// `host:port` to bind the HTTP listener to.
    pub const BIND_ADDR: &str = "LOOM_BIND_ADDR";
    /// PostgreSQL connection URL.
    pub const DATABASE_URL: &str = "DATABASE_URL";
    /// Bearer token guarding the `/admin` API.
    pub const ROOT_ADMIN_TOKEN: &str = "LOOM_ROOT_ADMIN_TOKEN";
    /// 32-byte AES-256-GCM key, hex-encoded (64 hex chars).
    pub const ENCRYPTION_KEY: &str = "LOOM_ENCRYPTION_KEY";
    /// Optional pepper for the virtual-key HMAC; derived from the encryption
    /// key when unset.
    pub const KEY_PEPPER: &str = "LOOM_KEY_PEPPER";
    /// Whether to run database migrations on startup (default: on).
    pub const RUN_MIGRATIONS: &str = "LOOM_RUN_MIGRATIONS";
}

/// A validated startup configuration.
///
/// Construct it with [`Config::from_env`]. The secret fields are private and
/// only surfaced to the components that need them; the [`Debug`] impl redacts
/// them so a configuration can be logged safely.
#[derive(Clone)]
pub struct Config {
    /// The socket address to bind the HTTP listener to.
    pub bind_addr: SocketAddr,
    /// The PostgreSQL connection URL.
    pub database_url: String,
    /// Whether to apply database migrations on startup.
    pub run_migrations: bool,
    /// The `/admin` bearer token.
    pub(crate) root_admin_token: String,
    /// The AES-256-GCM key for credential encryption.
    pub(crate) encryption_key: [u8; 32],
    /// The pepper for the virtual-key lookup HMAC.
    pub(crate) key_pepper: Vec<u8>,
}

impl Config {
    /// Loads and validates configuration from the process environment.
    ///
    /// # Required variables
    ///
    /// - `DATABASE_URL`
    /// - `LOOM_ROOT_ADMIN_TOKEN` (must be non-empty)
    /// - `LOOM_ENCRYPTION_KEY` (64 hex characters → 32 bytes)
    ///
    /// # Optional variables
    ///
    /// - `LOOM_BIND_ADDR` (default `0.0.0.0:8080`)
    /// - `LOOM_KEY_PEPPER` (default: derived deterministically from the
    ///   encryption key via `HMAC-SHA256(encryption_key, "loom.virtual-key.pepper.v1")`)
    /// - `LOOM_RUN_MIGRATIONS` (`false`/`0`/`no`/`off` disable it; default on)
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if a required variable is missing or any value
    /// is malformed.
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind_addr_raw =
            optional(env_keys::BIND_ADDR).unwrap_or_else(|| DEFAULT_BIND_ADDR.to_owned());
        let bind_addr =
            bind_addr_raw
                .parse::<SocketAddr>()
                .map_err(|e| ConfigError::Malformed {
                    name: env_keys::BIND_ADDR,
                    detail: format!("expected host:port ({e})"),
                })?;

        let database_url = required(env_keys::DATABASE_URL)?;

        let root_admin_token = required(env_keys::ROOT_ADMIN_TOKEN)?;
        if root_admin_token.is_empty() {
            return Err(ConfigError::Malformed {
                name: env_keys::ROOT_ADMIN_TOKEN,
                detail: "must not be empty".to_owned(),
            });
        }

        let encryption_key = parse_encryption_key(&required(env_keys::ENCRYPTION_KEY)?)?;

        let key_pepper = match optional(env_keys::KEY_PEPPER) {
            Some(p) if !p.is_empty() => p.into_bytes(),
            _ => derive_pepper(&encryption_key),
        };

        let run_migrations = match optional(env_keys::RUN_MIGRATIONS) {
            Some(v) => !matches!(
                v.to_ascii_lowercase().as_str(),
                "false" | "0" | "no" | "off"
            ),
            None => true,
        };

        Ok(Self {
            bind_addr,
            database_url,
            run_migrations,
            root_admin_token,
            encryption_key,
            key_pepper,
        })
    }

    /// The `/admin` bearer token.
    #[must_use]
    pub(crate) fn root_admin_token(&self) -> &str {
        &self.root_admin_token
    }

    /// The AES-256-GCM credential-encryption key.
    #[must_use]
    pub(crate) fn encryption_key(&self) -> [u8; 32] {
        self.encryption_key
    }

    /// The virtual-key lookup pepper.
    #[must_use]
    pub(crate) fn key_pepper(&self) -> &[u8] {
        &self.key_pepper
    }
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field("database_url", &"<redacted>")
            .field("run_migrations", &self.run_migrations)
            .field("root_admin_token", &"<redacted>")
            .field("encryption_key", &"<redacted>")
            .field("key_pepper", &"<redacted>")
            .finish()
    }
}

/// Parses a hex-encoded 32-byte AES key.
fn parse_encryption_key(raw: &str) -> Result<[u8; 32], ConfigError> {
    let bytes = hex::decode(raw.trim()).map_err(|e| ConfigError::Malformed {
        name: env_keys::ENCRYPTION_KEY,
        detail: format!("must be hex-encoded ({e})"),
    })?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| ConfigError::Malformed {
            name: env_keys::ENCRYPTION_KEY,
            detail: format!("must decode to exactly 32 bytes, got {}", v.len()),
        })
}

/// Reads a required environment variable, mapping absence to a typed error.
fn required(name: &'static str) -> Result<String, ConfigError> {
    match std::env::var(name) {
        Ok(v) => Ok(v),
        Err(std::env::VarError::NotPresent) => Err(ConfigError::Missing(name)),
        Err(std::env::VarError::NotUnicode(_)) => Err(ConfigError::Malformed {
            name,
            detail: "value is not valid UTF-8".to_owned(),
        }),
    }
}

/// Reads an optional environment variable, treating non-Unicode as absent.
fn optional(name: &'static str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// An error encountered while loading [`Config`] from the environment.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A required environment variable was not set.
    #[error("required environment variable {0} is not set")]
    Missing(&'static str),

    /// An environment variable held a malformed value.
    #[error("environment variable {name} is malformed: {detail}")]
    Malformed {
        /// The offending variable name.
        name: &'static str,
        /// A human-readable description of the problem.
        detail: String,
    },
}
