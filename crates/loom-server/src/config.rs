//! Startup configuration loaded from the process environment.
//!
//! Every secret is validated eagerly at boot so the server fails fast with a
//! clear diagnostic rather than 500-ing on the first request. Secrets never
//! appear in the [`Debug`] representation.

use std::net::SocketAddr;
use std::time::Duration;

use crate::keys::derive_pepper;

/// The default address the server binds when `LOOM_BIND_ADDR` is unset.
pub const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

/// The default batch poll interval (seconds) when `LOOM_BATCH_POLL_INTERVAL_SECS`
/// is unset.
pub const DEFAULT_BATCH_POLL_INTERVAL_SECS: u64 = 5;

/// Environment variable names read at startup.
mod env_keys {
    /// `host:port` to bind the HTTP listener to.
    pub const BIND_ADDR: &str = "LOOM_BIND_ADDR";
    /// PostgreSQL connection URL.
    pub const DATABASE_URL: &str = "DATABASE_URL";
    /// Runtime pool bound (migration-only commands always use one connection).
    pub const DATABASE_MAX_CONNECTIONS: &str = "LOOM_DATABASE_MAX_CONNECTIONS";
    /// Opt in to request-driven startup and close-on-release database connections.
    pub const REQUEST_DRIVEN: &str = "LOOM_REQUEST_DRIVEN";
    /// Bearer token guarding the `/admin` API.
    pub const ROOT_ADMIN_TOKEN: &str = "LOOM_ROOT_ADMIN_TOKEN";
    /// 32-byte AES-256-GCM key, hex-encoded (64 hex chars).
    pub const ENCRYPTION_KEY: &str = "LOOM_ENCRYPTION_KEY";
    /// Optional pepper for the virtual-key HMAC; derived from the encryption
    /// key when unset.
    pub const KEY_PEPPER: &str = "LOOM_KEY_PEPPER";
    /// Whether to run database migrations on startup (default: on).
    pub const RUN_MIGRATIONS: &str = "LOOM_RUN_MIGRATIONS";
    /// Seconds between batch poll-worker passes (`0` disables the worker).
    pub const BATCH_POLL_INTERVAL_SECS: &str = "LOOM_BATCH_POLL_INTERVAL_SECS";
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
    /// Runtime database connection budget, including held conversation locks.
    pub database_max_connections: u32,
    /// Disable startup writes/workers and discard connections on release.
    pub request_driven: bool,
    /// Whether to apply database migrations on startup.
    pub run_migrations: bool,
    /// How often the batch poll worker advances active jobs. Zero disables the
    /// worker (e.g. when a dedicated worker process owns it).
    pub batch_poll_interval: Duration,
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
    /// - `LOOM_REQUEST_DRIVEN` (`true`/`1`: no startup writes/workers, close DB connections on release)
    /// - `LOOM_BIND_ADDR` (default `0.0.0.0:8080`)
    /// - `LOOM_DATABASE_MAX_CONNECTIONS` (default `10`, range `2..=64`)
    /// - `LOOM_KEY_PEPPER` (default: derived deterministically from the
    ///   encryption key via `HMAC-SHA256(encryption_key, "loom.virtual-key.pepper.v1")`)
    /// - `LOOM_RUN_MIGRATIONS` (`false`/`0`/`no`/`off` disable it; default on)
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if a required variable is missing or any value
    /// is malformed.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_reader(std::env::var)
    }

    // A per-startup lookup overlays validated secrets without mutating global
    // process state, which is unsafe after the Tokio runtime has started.
    pub(crate) fn from_reader(
        read: impl Fn(&'static str) -> Result<String, std::env::VarError>,
    ) -> Result<Self, ConfigError> {
        let required = |name| match read(name) {
            Ok(value) => Ok(value),
            Err(std::env::VarError::NotPresent) => Err(ConfigError::Missing(name)),
            Err(std::env::VarError::NotUnicode(_)) => Err(ConfigError::Malformed {
                name,
                detail: "value is not valid UTF-8".to_owned(),
            }),
        };
        let optional = |name| read(name).ok().filter(|v| !v.is_empty());
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
        let request_driven = parse_request_driven(optional(env_keys::REQUEST_DRIVEN))?;
        let database_max_connections = parse_pool_limit(
            optional(env_keys::DATABASE_MAX_CONNECTIONS)
                .or_else(|| request_driven.then(|| "2".to_owned())),
        )?;

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
            None => !request_driven,
        };

        let batch_poll_interval = match optional(env_keys::BATCH_POLL_INTERVAL_SECS) {
            Some(v) => {
                let secs = v.parse::<u64>().map_err(|e| ConfigError::Malformed {
                    name: env_keys::BATCH_POLL_INTERVAL_SECS,
                    detail: format!("expected a non-negative integer ({e})"),
                })?;
                Duration::from_secs(secs)
            }
            None if request_driven => Duration::ZERO,
            None => Duration::from_secs(DEFAULT_BATCH_POLL_INTERVAL_SECS),
        };

        validate_request_driven(request_driven, run_migrations, batch_poll_interval)?;

        Ok(Self {
            bind_addr,
            database_url,
            database_max_connections,
            request_driven,
            run_migrations,
            batch_poll_interval,
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
            .field("database_max_connections", &self.database_max_connections)
            .field("request_driven", &self.request_driven)
            .field("run_migrations", &self.run_migrations)
            .field("batch_poll_interval", &self.batch_poll_interval)
            .field("root_admin_token", &"<redacted>")
            .field("encryption_key", &"<redacted>")
            .field("key_pepper", &"<redacted>")
            .finish()
    }
}

fn parse_request_driven(value: Option<String>) -> Result<bool, ConfigError> {
    match value.as_deref() {
        None | Some("false" | "0") => Ok(false),
        Some("true" | "1") => Ok(true),
        _ => Err(ConfigError::Malformed {
            name: env_keys::REQUEST_DRIVEN,
            detail: "expected true/false or 1/0".into(),
        }),
    }
}

fn validate_request_driven(
    enabled: bool,
    migrations: bool,
    poll: Duration,
) -> Result<(), ConfigError> {
    if enabled && (migrations || !poll.is_zero()) {
        return Err(ConfigError::Malformed {
            name: env_keys::REQUEST_DRIVEN,
            detail: "requires startup migrations and batch polling disabled".into(),
        });
    }
    Ok(())
}

fn parse_pool_limit(value: Option<String>) -> Result<u32, ConfigError> {
    match value {
        None => Ok(10),
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|n| (2..=64).contains(n))
            .ok_or(ConfigError::Malformed {
                name: env_keys::DATABASE_MAX_CONNECTIONS,
                detail: "expected an integer from 2 to 64".into(),
            }),
    }
}

#[cfg(test)]
mod pool_tests {
    #[test]
    fn request_driven_mode_rejects_background_writes_and_unknown_values() {
        use super::{parse_request_driven, validate_request_driven};
        use std::time::Duration;
        assert!(!parse_request_driven(None).unwrap());
        assert!(parse_request_driven(Some("true".into())).unwrap());
        assert!(parse_request_driven(Some("1".into())).unwrap());
        assert!(!parse_request_driven(Some("false".into())).unwrap());
        assert!(parse_request_driven(Some("yes".into())).is_err());
        assert!(validate_request_driven(true, false, Duration::ZERO).is_ok());
        assert!(validate_request_driven(true, true, Duration::ZERO).is_err());
        assert!(validate_request_driven(true, false, Duration::from_secs(1)).is_err());
        assert!(validate_request_driven(false, true, Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn pool_budget_is_bounded_and_keeps_database_work_capacity() {
        assert_eq!(super::parse_pool_limit(None).unwrap(), 10);
        for n in [2, 6, 64] {
            assert_eq!(super::parse_pool_limit(Some(n.to_string())).unwrap(), n);
        }
        for n in ["0", "1", "65", "-1", "1.5", "unexpected"] {
            assert!(super::parse_pool_limit(Some(n.into())).is_err());
        }
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
