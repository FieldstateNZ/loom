//! Shared official AWS SigV4 signing; callers enforce their own origin policy.
use crate::Error;
/// Execution-role credentials; never include them in logs or diagnostics.
pub use aws_credential_types::Credentials;
use aws_sigv4::{
    http_request::{sign, PayloadChecksumKind, SignableBody, SignableRequest, SigningSettings},
    sign::v4,
};
use reqwest_mcp::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Url,
};
use std::time::SystemTime;

/// Load only injected execution-role values. No profile, metadata or SSO lookup.
pub fn execution_role_credentials(
    read: impl Fn(&'static str) -> Option<String>,
) -> Result<Credentials, Error> {
    let key = read("AWS_ACCESS_KEY_ID")
        .filter(|v| !v.is_empty())
        .ok_or(Error::Configuration)?;
    let secret = read("AWS_SECRET_ACCESS_KEY")
        .filter(|v| !v.is_empty())
        .ok_or(Error::Configuration)?;
    let token = read("AWS_SESSION_TOKEN")
        .filter(|v| !v.is_empty())
        .ok_or(Error::Configuration)?;
    Ok(Credentials::new(
        key,
        secret,
        Some(token),
        None,
        "lambda-environment",
    ))
}

/// Sign exact owned bytes with the existing redaction boundary. This helper
/// makes no HTTP request; each caller must validate endpoint/service/policy.
pub fn signed_headers(
    method: &str,
    url: &Url,
    headers: &HeaderMap,
    body: &[u8],
    region: &str,
    service: &str,
    credentials: Credentials,
) -> Result<HeaderMap, Error> {
    let mut headers = headers.clone();
    let authority = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().ok_or(Error::Configuration)?),
        None => url.host_str().ok_or(Error::Configuration)?.to_owned(),
    };
    headers.insert(
        "host",
        HeaderValue::from_str(&authority).map_err(|_| Error::Configuration)?,
    );
    let identity = credentials.into();
    let mut settings = SigningSettings::default();
    settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
    let params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name(service)
        .time(SystemTime::now())
        .settings(settings)
        .build()
        .map_err(|_| Error::Configuration)?
        .into();
    let entries: Result<Vec<_>, _> = headers
        .iter()
        .map(|(n, v)| v.to_str().map(|v| (n.as_str(), v)))
        .collect();
    let request = SignableRequest::new(
        method,
        url.as_str(),
        entries.map_err(|_| Error::Configuration)?.into_iter(),
        SignableBody::Bytes(body),
    )
    .map_err(|_| Error::Configuration)?;
    // The SDK's TRACE request Debug includes custom header strings. Scope
    // it out synchronously so application Bearers/body can never enter SDK
    // telemetry, even when an operator enables tracing for troubleshooting.
    let (instructions, _) =
        tracing::dispatcher::with_default(&tracing::Dispatch::none(), || sign(request, &params))
            .map_err(|_| Error::Configuration)?
            .into_parts();
    let (added, query) = instructions.into_parts();
    if !query.is_empty() {
        return Err(Error::Configuration);
    }
    for header in added {
        let mut value = HeaderValue::from_str(header.value()).map_err(|_| Error::Configuration)?;
        value.set_sensitive(
            header.sensitive() || matches!(header.name(), "authorization" | "x-amz-security-token"),
        );
        headers.insert(
            HeaderName::from_bytes(header.name().as_bytes()).map_err(|_| Error::Configuration)?,
            value,
        );
    }
    Ok(headers)
}
