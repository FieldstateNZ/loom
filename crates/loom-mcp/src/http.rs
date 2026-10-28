//! Private HTTP transport: ordinary endpoints or IAM-signed Function URLs.
//! The MCP SDK retains its protocol parsing and no-retry behavior.

use crate::Error;
use aws_credential_types::Credentials;
use futures::stream::BoxStream;
use reqwest_mcp::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client, Request, Response, Url,
};
use rmcp::{
    model::ClientJsonRpcMessage,
    transport::streamable_http_client::{
        SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
    },
};
use serde::Serialize;
use sse_stream::Sse;
use std::{borrow::Cow, collections::HashMap, sync::Arc};

type CredentialSource = Arc<dyn Fn() -> Result<Credentials, Error> + Send + Sync>;
#[derive(Clone)]
struct SigningTarget {
    region: String,
    credentials: CredentialSource,
}

/// Origin-pinned client shared by context, MCP and recoverable renewal calls.
/// No credentials are included in Debug, errors or logs.
#[derive(Clone)]
pub struct PrivateHttpClient {
    client: Client,
    origin: Url,
    signing: Option<SigningTarget>,
}

impl PrivateHttpClient {
    /// Select signing only for a canonical regional AWS Function URL. Lambda
    /// uses its injected temporary execution-role environment credentials.
    /// No metadata/SSO/provider request is started by construction or signing.
    pub fn new(endpoint: &Url) -> Result<Self, Error> {
        let hostname = endpoint.host_str().ok_or(Error::Configuration)?;
        let parts: Vec<_> = hostname.split('.').collect();
        let signing = if hostname.contains(".lambda-url.") {
            if parts.len() != 5
                || parts[1] != "lambda-url"
                || parts[3] != "on"
                || parts[4] != "aws"
                || parts[0].is_empty()
                || !parts[0]
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                || parts[2].is_empty()
                || !parts[2]
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || endpoint.scheme() != "https"
                || endpoint.port().is_some()
                || std::env::var("AWS_REGION").ok().as_deref() != Some(parts[2])
            {
                return Err(Error::Configuration);
            }
            Some(SigningTarget {
                region: parts[2].to_owned(),
                credentials: Arc::new(environment_credentials),
            })
        } else {
            None
        };
        let loopback = matches!(hostname, "127.0.0.1" | "localhost" | "[::1]" | "::1");
        if (endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback))
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/mcp"
        {
            return Err(Error::Configuration);
        }
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let client = Client::builder()
            .redirect(reqwest_mcp::redirect::Policy::none())
            .retry(reqwest_mcp::retry::never())
            .connect_timeout(std::time::Duration::from_secs(3))
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|_| Error::Configuration)?;
        Ok(Self {
            client,
            origin: endpoint.clone(),
            signing,
        })
    }

    fn check(&self, method: &str, url: &Url) -> Result<(), Error> {
        if url.origin() != self.origin.origin()
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(
                (method, url.path()),
                ("GET", "/mcp" | "/mcp/context")
                    | ("POST", "/mcp" | "/api/v1/oauth/token")
                    | ("DELETE", "/mcp")
            )
        {
            return Err(Error::Configuration);
        }
        Ok(())
    }

    fn signed_headers(
        &self,
        method: &str,
        url: &Url,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<HeaderMap, Error> {
        self.check(method, url)?;
        let Some(target) = &self.signing else {
            return Ok(headers.clone());
        };
        if body.len() > 65536
            || headers.contains_key("x-tandem-authorization")
            || headers.keys().any(|n| n.as_str().starts_with("x-amz-"))
        {
            return Err(Error::Configuration);
        }
        let mut headers = headers.clone();
        if let Some(mut bearer) = headers.remove("authorization") {
            if headers.contains_key("authorization")
                || !bearer.to_str().is_ok_and(|v| {
                    v.starts_with("Bearer ")
                        && v.len() > 7
                        && !v[7..].bytes().any(|b| b.is_ascii_whitespace())
                })
            {
                return Err(Error::Configuration);
            }
            bearer.set_sensitive(true);
            headers.insert("x-tandem-authorization", bearer);
        } else if url.path() != "/api/v1/oauth/token" {
            return Err(Error::Configuration);
        }
        crate::aws::signed_headers(
            method,
            url,
            &headers,
            body,
            &target.region,
            "lambda",
            (target.credentials)()?,
        )
    }

    /// Start a bounded private context read using the underlying client's policy.
    pub fn get(&self, url: Url) -> PrivateRequest {
        PrivateRequest {
            client: self.clone(),
            request: self.client.get(url),
        }
    }
    /// Start a private token renewal, preserving its original committed body.
    pub fn post(&self, url: Url) -> PrivateRequest {
        PrivateRequest {
            client: self.clone(),
            request: self.client.post(url),
        }
    }

    async fn execute(&self, mut request: Request) -> Result<Response, Error> {
        let body = match request.body() {
            Some(body) => body.as_bytes().ok_or(Error::Configuration)?,
            None => &[],
        };
        let headers = self.signed_headers(
            request.method().as_str(),
            request.url(),
            request.headers(),
            body,
        )?;
        *request.headers_mut() = headers;
        self.client
            .execute(request)
            .await
            .map_err(|_| Error::Transport)
    }

    fn mcp_headers(
        &self,
        method: &str,
        uri: &str,
        mut custom: HashMap<HeaderName, HeaderValue>,
        auth: Option<String>,
        resume: (Option<&str>, Option<&str>),
        body: &[u8],
    ) -> Result<HashMap<HeaderName, HeaderValue>, StreamableHttpError<reqwest_mcp::Error>> {
        let (session, last) = resume;
        for name in ["accept", "mcp-session-id", "last-event-id"] {
            if custom.contains_key(name) {
                return Err(StreamableHttpError::ReservedHeaderConflict(name.into()));
            }
        }
        let url = Url::parse(uri).map_err(|_| transport_error())?;
        self.check(method, &url).map_err(|_| transport_error())?;
        // Leave ordinary transport semantics to the official client unchanged.
        if self.signing.is_none() {
            return Ok(custom);
        }
        if let Some(auth) = auth {
            if custom.contains_key("authorization") {
                return Err(transport_error());
            }
            let mut value =
                HeaderValue::from_str(&format!("Bearer {auth}")).map_err(|_| transport_error())?;
            value.set_sensitive(true);
            custom.insert(HeaderName::from_static("authorization"), value);
        }
        let mut canonical: HeaderMap = custom.into_iter().collect();
        if method != "DELETE" {
            canonical.insert(
                "accept",
                HeaderValue::from_static("text/event-stream, application/json"),
            );
        }
        if method == "POST" && !canonical.contains_key("content-type") {
            canonical.insert("content-type", HeaderValue::from_static("application/json"));
        }
        if let Some(id) = session {
            canonical.insert(
                "mcp-session-id",
                HeaderValue::from_str(id).map_err(|_| transport_error())?,
            );
        }
        if let Some(id) = last {
            canonical.insert(
                "last-event-id",
                HeaderValue::from_str(id).map_err(|_| transport_error())?,
            );
        }
        let mut signed = self
            .signed_headers(method, &url, &canonical, body)
            .map_err(|_| transport_error())?;
        // The SDK adds these exact reserved fields itself after custom headers.
        for name in ["accept", "mcp-session-id", "last-event-id"] {
            signed.remove(name);
        }
        Ok(signed
            .into_iter()
            .filter_map(|(n, v)| n.map(|n| (n, v)))
            .collect())
    }
}

/// A private request builder whose send path applies signing before HTTP.
pub struct PrivateRequest {
    client: PrivateHttpClient,
    request: reqwest_mcp::RequestBuilder,
}
impl PrivateRequest {
    /// Add the original application Bearer (moved only at the signing boundary).
    pub fn bearer_auth(mut self, value: impl AsRef<str>) -> Self {
        self.request = self.request.bearer_auth(value.as_ref());
        self
    }
    /// Add server-owned identity/protocol headers.
    pub fn header(mut self, name: &'static str, value: impl AsRef<str>) -> Self {
        self.request = self.request.header(name, value.as_ref());
        self
    }
    /// Preserve all typed binding headers.
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.request = self.request.headers(headers);
        self
    }
    /// Serialize once, then sign exactly the stored request bytes.
    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        self.request = self.request.json(value);
        self
    }
    /// Send once under the original HTTP client's deadline/redirect/retry policy.
    pub async fn send(self) -> Result<Response, Error> {
        self.client
            .execute(self.request.build().map_err(|_| Error::Configuration)?)
            .await
    }
}

fn environment_credentials() -> Result<Credentials, Error> {
    crate::aws::execution_role_credentials(|name| std::env::var(name).ok())
}

fn transport_error() -> StreamableHttpError<reqwest_mcp::Error> {
    StreamableHttpError::UnexpectedServerResponse(Cow::Borrowed(
        "private HTTP signing is unavailable",
    ))
}

impl StreamableHttpClient for PrivateHttpClient {
    type Error = reqwest_mcp::Error;
    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session: Option<Arc<str>>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let body = if self.signing.is_some() {
            serde_json::to_vec(&message).map_err(|_| transport_error())?
        } else {
            Vec::new()
        };
        let headers = self.mcp_headers(
            "POST",
            &uri,
            headers,
            auth.clone(),
            (session.as_deref(), None),
            &body,
        )?;
        self.client
            .post_message(
                uri,
                message,
                session,
                if self.signing.is_some() { None } else { auth },
                headers,
            )
            .await
    }
    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session: Option<Arc<str>>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
        limit: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let body = if self.signing.is_some() {
            serde_json::to_vec(&message).map_err(|_| transport_error())?
        } else {
            Vec::new()
        };
        let headers = self.mcp_headers(
            "POST",
            &uri,
            headers,
            auth.clone(),
            (session.as_deref(), None),
            &body,
        )?;
        self.client
            .post_message_with_max_sse_event_size(
                uri,
                message,
                session,
                if self.signing.is_some() { None } else { auth },
                headers,
                limit,
            )
            .await
    }
    async fn delete_session(
        &self,
        uri: Arc<str>,
        session: Arc<str>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let headers = self.mcp_headers(
            "DELETE",
            &uri,
            headers,
            auth.clone(),
            (Some(&session), None),
            &[],
        )?;
        self.client
            .delete_session(
                uri,
                session,
                if self.signing.is_some() { None } else { auth },
                headers,
            )
            .await
    }
    async fn get_stream(
        &self,
        uri: Arc<str>,
        session: Option<Arc<str>>,
        last: Option<String>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let headers = self.mcp_headers(
            "GET",
            &uri,
            headers,
            auth.clone(),
            (session.as_deref(), last.as_deref()),
            &[],
        )?;
        self.client
            .get_stream(
                uri,
                session,
                last,
                if self.signing.is_some() { None } else { auth },
                headers,
            )
            .await
    }
    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session: Option<Arc<str>>,
        last: Option<String>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
        limit: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let headers = self.mcp_headers(
            "GET",
            &uri,
            headers,
            auth.clone(),
            (session.as_deref(), last.as_deref()),
            &[],
        )?;
        self.client
            .get_stream_with_max_sse_event_size(
                uri,
                session,
                last,
                if self.signing.is_some() { None } else { auth },
                headers,
                limit,
            )
            .await
    }
}

#[cfg(test)]
mod tests;
