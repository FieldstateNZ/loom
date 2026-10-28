//! Opt-in encrypted settings before the private Lambda gateway is constructed.
//! Existing server startup and environment-based Lambda operation are unchanged.
use crate::Config;
use futures::StreamExt;
use loom_mcp::aws::{execution_role_credentials, signed_headers};
use reqwest_mcp::{
    header::{HeaderMap, HeaderValue},
    Client, Url,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    env::VarError,
    fs,
    io::{Cursor, Write},
    path::PathBuf,
    time::Duration,
};
use uuid::Uuid;

const SECRET_KEYS: [&str; 4] = [
    "DATABASE_URL",
    "LOOM_ROOT_ADMIN_TOKEN",
    "LOOM_ENCRYPTION_KEY",
    "LOOM_KEY_PEPPER",
];
const LIMIT: usize = 16 * 1024;
const DEADLINE: Duration = Duration::from_secs(6);
type Env<'a> = dyn Fn(&'static str) -> Result<String, VarError> + 'a;

/// A deliberately redacted error, including all upstream/configuration failures.
#[derive(Debug, thiserror::Error)]
#[error("private Lambda bootstrap unavailable")]
pub struct BootstrapError;

struct Reference {
    arn: String,
    name: String,
    region: String,
    version: u64,
}
fn optional(read: &Env<'_>, name: &'static str) -> Result<Option<String>, BootstrapError> {
    match read(name) {
        Ok(v) => Ok(Some(v)),
        Err(VarError::NotPresent) => Ok(None),
        Err(_) => Err(BootstrapError),
    }
}
fn reference(read: &Env<'_>) -> Result<Option<Reference>, BootstrapError> {
    let Some(arn) = optional(read, "LOOM_BOOTSTRAP_PARAMETER_ARN")? else {
        if optional(read, "LOOM_BOOTSTRAP_PARAMETER_VERSION")?.is_some() {
            return Err(BootstrapError);
        }
        return Ok(None);
    };
    let region = read("AWS_REGION").map_err(|_| BootstrapError)?;
    let account = read("LOOM_LAMBDA_ACCOUNT_ID").map_err(|_| BootstrapError)?;
    let raw_version = read("LOOM_BOOTSTRAP_PARAMETER_VERSION").map_err(|_| BootstrapError)?;
    let version = raw_version.parse::<u64>().map_err(|_| BootstrapError)?;
    let prefix = format!("arn:aws:ssm:{region}:{account}:parameter");
    let name = arn.strip_prefix(&prefix).ok_or(BootstrapError)?.to_owned();
    if region.is_empty()
        || !region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || account.len() != 12
        || !account.bytes().all(|b| b.is_ascii_digit())
        || !name.starts_with("/tandem/")
        || name
            .split('/')
            .skip(1)
            .any(|x| x.is_empty() || x == "." || x == "..")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./-".contains(&b))
        || version == 0
        || version > i64::MAX as u64
        || raw_version != version.to_string()
        || !matches!(
            read("LOOM_REQUEST_DRIVEN").ok().as_deref(),
            Some("true" | "1")
        )
        || SECRET_KEYS
            .iter()
            .any(|&k| !matches!(read(k), Err(VarError::NotPresent)))
    {
        return Err(BootstrapError);
    }
    Ok(Some(Reference {
        arn,
        name,
        region,
        version,
    }))
}

#[derive(Deserialize)]
struct Response {
    #[serde(rename = "Parameter")]
    parameter: Parameter,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Parameter {
    #[serde(rename = "ARN")]
    arn: String,
    name: String,
    #[serde(rename = "Type")]
    kind: String,
    version: u64,
    selector: Option<String>,
    value: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    kind: String,
    database_url: String,
    database_ca_pem: String,
    database_ca_sha256: String,
    root_admin_token: String,
    encryption_key: String,
    key_pepper: Pepper,
}
#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Pepper {
    Derived,
    Explicit { value: String },
}

struct PrivateCa {
    directory: PathBuf,
}
impl Drop for PrivateCa {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.directory.join("database-ca.pem"));
        let _ = fs::remove_dir(&self.directory);
    }
}
impl PrivateCa {
    #[cfg(unix)]
    fn create(pem: &str) -> Result<Self, BootstrapError> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let directory = std::env::temp_dir().join(format!("loom-lambda-ca-{}", Uuid::new_v4()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| BootstrapError)?;
        let owned = Self { directory };
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(owned.directory.join("database-ca.pem"))
            .map_err(|_| BootstrapError)?;
        file.write_all(pem.as_bytes()).map_err(|_| BootstrapError)?;
        Ok(owned)
    }
    #[cfg(not(unix))]
    fn create(_pem: &str) -> Result<Self, BootstrapError> {
        Err(BootstrapError)
    }
}

/// Validated configuration and its private CA lifetime, retained by the runtime.
pub struct LoadedConfig {
    config: Config,
    _ca: Option<PrivateCa>,
}
impl LoadedConfig {
    /// Borrow settings without exposing credentials through diagnostics.
    pub fn config(&self) -> &Config {
        &self.config
    }
}

fn text(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len()) && !value.bytes().any(|b| matches!(b, 0 | b'\r' | b'\n'))
}
fn materialise(
    ref_: &Reference,
    bytes: &[u8],
    read: &Env<'_>,
) -> Result<LoadedConfig, BootstrapError> {
    if bytes.len() > LIMIT {
        return Err(BootstrapError);
    }
    let parameter = serde_json::from_slice::<Response>(bytes)
        .map_err(|_| BootstrapError)?
        .parameter;
    if parameter.arn != ref_.arn
        || parameter.name != ref_.name
        || parameter.kind != "SecureString"
        || parameter.version != ref_.version
        || parameter
            .selector
            .as_ref()
            .is_some_and(|s| s != &format!(":{}", ref_.version))
        || parameter.value.len() > 4096
    {
        return Err(BootstrapError);
    }
    let d: Document = serde_json::from_str(&parameter.value).map_err(|_| BootstrapError)?;
    if d.schema_version != 1
        || d.kind != "loom-private-lambda"
        || !text(&d.database_url, 16, 1024)
        || !text(&d.root_admin_token, 32, 256)
        || d.encryption_key.len() != 64
        || !d.encryption_key.bytes().all(|b| b.is_ascii_hexdigit())
        || d.database_ca_sha256 != hex::encode(Sha256::digest(d.database_ca_pem.as_bytes()))
    {
        return Err(BootstrapError);
    }
    let mut database = Url::parse(&d.database_url).map_err(|_| BootstrapError)?;
    if !matches!(database.scheme(), "postgres" | "postgresql")
        || database.host_str().is_none()
        || database.username().is_empty()
        || database.password().is_none_or(str::is_empty)
        || database.query().is_some()
        || database.fragment().is_some()
        || !database.path().strip_prefix('/').is_some_and(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
    {
        return Err(BootstrapError);
    }
    let pem = &d.database_ca_pem;
    let encoded = pem
        .strip_prefix("-----BEGIN CERTIFICATE-----\n")
        .and_then(|p| {
            p.trim_end_matches('\n')
                .strip_suffix("\n-----END CERTIFICATE-----")
        })
        .ok_or(BootstrapError)?;
    if encoded.is_empty()
        || !encoded
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+/=\n".contains(&b))
    {
        return Err(BootstrapError);
    }
    let certs = rustls_pemfile::certs(&mut Cursor::new(pem.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| BootstrapError)?;
    if certs.len() != 1 {
        return Err(BootstrapError);
    }
    rustls::RootCertStore::empty()
        .add(certs.into_iter().next().ok_or(BootstrapError)?)
        .map_err(|_| BootstrapError)?;
    let ca = PrivateCa::create(pem)?;
    let ca_file = ca.directory.join("database-ca.pem");
    database
        .query_pairs_mut()
        .append_pair("sslmode", "verify-full")
        .append_pair("sslrootcert", ca_file.to_str().ok_or(BootstrapError)?);
    let mut values = HashMap::from([
        ("DATABASE_URL", database.to_string()),
        ("LOOM_ROOT_ADMIN_TOKEN", d.root_admin_token),
        ("LOOM_ENCRYPTION_KEY", d.encryption_key),
    ]);
    if let Pepper::Explicit { value } = d.key_pepper {
        if !text(&value, 1, 256) {
            return Err(BootstrapError);
        }
        values.insert("LOOM_KEY_PEPPER", value);
    }
    let config =
        Config::from_reader(|name| values.get(name).cloned().map_or_else(|| read(name), Ok))
            .map_err(|_| BootstrapError)?;
    Ok(LoadedConfig {
        config,
        _ca: Some(ca),
    })
}

fn request(ref_: &Reference, read: &Env<'_>) -> Result<(Url, HeaderMap, Vec<u8>), BootstrapError> {
    let credentials =
        execution_role_credentials(|name| read(name).ok()).map_err(|_| BootstrapError)?;
    let endpoint =
        Url::parse(&format!("https://ssm.{}.api.aws/", ref_.region)).map_err(|_| BootstrapError)?;
    let body = serde_json::to_vec(
        &serde_json::json!({"Name":format!("{}:{}",ref_.arn,ref_.version),"WithDecryption":true}),
    )
    .map_err(|_| BootstrapError)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_static("application/x-amz-json-1.1"),
    );
    headers.insert(
        "x-amz-target",
        HeaderValue::from_static("AmazonSSM.GetParameter"),
    );
    let headers = signed_headers(
        "POST",
        &endpoint,
        &headers,
        &body,
        &ref_.region,
        "ssm",
        credentials,
    )
    .map_err(|_| BootstrapError)?;
    Ok((endpoint, headers, body))
}
async fn fetch(
    ref_: &Reference,
    read: &Env<'_>,
    client: Client,
) -> Result<Vec<u8>, BootstrapError> {
    let (endpoint, headers, body) = request(ref_, read)?;
    let response = client
        .post(endpoint)
        .headers(headers)
        .body(body)
        .send()
        .await
        .map_err(|_| BootstrapError)?;
    if response.status() != 200 || response.content_length().is_some_and(|n| n > LIMIT as u64) {
        return Err(BootstrapError);
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| BootstrapError)?;
        if bytes.len() + chunk.len() > LIMIT {
            return Err(BootstrapError);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Read one explicitly pinned encrypted parameter, never poll or rotate it.
/// A missing reference preserves the original environment configuration path.
pub async fn load_config() -> Result<LoadedConfig, BootstrapError> {
    let read = |name| std::env::var(name);
    let Some(ref_) = reference(&read)? else {
        return Ok(LoadedConfig {
            config: Config::from_env().map_err(|_| BootstrapError)?,
            _ca: None,
        });
    };
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest_mcp::redirect::Policy::none())
        .retry(reqwest_mcp::retry::never())
        .connect_timeout(Duration::from_secs(3))
        .timeout(DEADLINE)
        .build()
        .map_err(|_| BootstrapError)?;
    let bytes = tokio::time::timeout(DEADLINE, fetch(&ref_, &read, client))
        .await
        .map_err(|_| BootstrapError)??;
    materialise(&ref_, &bytes, &read)
}

#[cfg(test)]
mod tests;
