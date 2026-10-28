//! Buffered, private Function URL entry point for the existing gateway.
//!
//! Event metadata is a defence in depth check, not signature verification. AWS
//! must enforce Function URL `AWS_IAM` and `InvokedViaFunctionUrl` permissions.
//! No admin, stateless inference or batch routes are exposed here.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    body::{to_bytes, Body as AxumBody},
    Router,
};
use http::{Method, Request, Response, StatusCode};
use lambda_http::{request::RequestContext, Body, RequestExt};
use tokio::sync::{OnceCell, Semaphore};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{build_router, AppState, Config};

const MAX_REQUEST: usize = 64 * 1024;
const MAX_RESPONSE: usize = 256 * 1024;

/// Finite same-account IAM callers admitted to this private HTTP surface.
#[derive(Clone)]
pub struct LambdaPolicy {
    account: String,
    region: String,
    roles: Vec<String>,
}

impl LambdaPolicy {
    /// Validate account, region and exact role ARNs (no wildcards or role paths).
    /// Role paths can be added explicitly later; ambiguous policies fail closed.
    pub fn new(account: String, region: String, roles: Vec<String>) -> Result<Self, &'static str> {
        if account.len() != 12
            || !account.bytes().all(|c| c.is_ascii_digit())
            || region.is_empty()
            || !region
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || roles.is_empty()
            || roles.len() > 16
        {
            return Err("invalid Lambda caller policy");
        }
        let prefix = format!("arn:aws:iam::{account}:role/");
        if roles.iter().any(|role| {
            role.strip_prefix(&prefix).is_none_or(|name| {
                name.is_empty() || name.len() > 64 || !name.bytes().all(role_char)
            })
        }) {
            return Err("invalid Lambda caller role");
        }
        Ok(Self {
            account,
            region,
            roles,
        })
    }

    /// Read the private operator policy without logging its contents.
    pub fn from_env() -> Result<Self, &'static str> {
        let account =
            std::env::var("LOOM_LAMBDA_ACCOUNT_ID").map_err(|_| "missing Lambda account")?;
        let region = std::env::var("AWS_REGION").map_err(|_| "missing Lambda region")?;
        let roles = serde_json::from_str(
            &std::env::var("LOOM_LAMBDA_CALLER_ROLE_ARNS").map_err(|_| "missing Lambda roles")?,
        )
        .map_err(|_| "invalid Lambda roles")?;
        Self::new(account, region, roles)
    }

    fn caller_allowed(&self, caller: &str) -> bool {
        self.roles.iter().any(|role| {
            if role == caller {
                return true;
            }
            let name = role.rsplit('/').next().unwrap_or_default();
            let prefix = format!("arn:aws:sts::{}:assumed-role/{name}/", self.account);
            caller.strip_prefix(&prefix).is_some_and(|session| {
                (2..=64).contains(&session.len()) && session.bytes().all(role_char)
            })
        })
    }

    fn validate(&self, request: &lambda_http::Request) -> bool {
        let Some(RequestContext::ApiGatewayV2(context)) = request.request_context_ref() else {
            return false;
        };
        let Some(runtime) = request.lambda_context_ref() else {
            return false;
        };
        let Some(iam) = context.authorizer.as_ref().and_then(|a| a.iam.as_ref()) else {
            return false;
        };
        let Some(domain) = context.domain_name.as_deref() else {
            return false;
        };
        let suffix = format!(".lambda-url.{}.on.aws", self.region);
        let valid_domain = domain.strip_suffix(&suffix).is_some_and(|id| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
        let arn = format!("arn:aws:lambda:{}:{}:function:", self.region, self.account);
        valid_domain
            && runtime
                .invoked_function_arn
                .strip_prefix(&arn)
                .is_some_and(|name| {
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
                })
            && context.account_id.as_deref() == Some(self.account.as_str())
            && iam.account_id.as_deref() == Some(self.account.as_str())
            && iam
                .user_arn
                .as_deref()
                .is_some_and(|arn| self.caller_allowed(arn))
            && context.http.method == request.method()
            && context.http.path.as_deref() == Some(request.raw_http_path())
            && request.uri().path() == request.raw_http_path()
            && request.uri().query().is_none()
            && !request.headers().contains_key("cookie")
            && request
                .headers()
                .get_all("host")
                .iter()
                .all(|h| h.as_bytes() == domain.as_bytes())
    }
}

fn role_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b"+=,.@_-".contains(&c)
}

fn allowed_route(method: &Method, path: &str) -> bool {
    if method == Method::GET
        && matches!(
            path,
            "/healthz" | "/readyz" | "/v1/whoami" | "/v1/mcp-servers"
        )
    {
        return true;
    }
    if method == Method::POST && path == "/v1/conversations" {
        return true;
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() < 4
        || !parts[0].is_empty()
        || parts[1] != "v1"
        || parts[2] != "conversations"
        || !canonical_uuid(parts[3])
    {
        return false;
    }
    match parts.as_slice() {
        [_, _, _, _] => method == Method::GET || method == Method::DELETE,
        [_, _, _, _, "turns"] => method == Method::POST,
        [_, _, _, _, "actions"] => method == Method::GET,
        [_, _, _, _, "actions", id, command] => {
            method == Method::POST
                && canonical_uuid(id)
                && matches!(
                    *command,
                    "decision" | "cancel" | "dispatch" | "retry" | "reconcile"
                )
        }
        _ => false,
    }
}

fn canonical_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

/// One Lambda execution environment with one admitted invocation at a time.
/// The supervisor owns cancellation cleanup even if its caller drops the future.
#[derive(Clone)]
pub struct LambdaGateway {
    state: AppState,
    router: Router,
    policy: LambdaPolicy,
    schema: Arc<OnceCell<()>>,
    admission: Arc<Semaphore>,
    poisoned: Arc<AtomicBool>,
    retire: Arc<dyn Fn() + Send + Sync>,
    work_budget: Duration,
    cleanup_budget: Duration,
}

impl LambdaGateway {
    /// Construct with the request-driven pool, no startup jobs or migrations.
    /// Schema verification occurs inside the first admitted invocation budget.
    pub fn new(config: &Config, policy: LambdaPolicy) -> Result<Self, &'static str> {
        if !config.request_driven
            || config.run_migrations
            || !config.batch_poll_interval.is_zero()
            || config.database_max_connections < 2
        {
            return Err("Lambda requires request-driven configuration");
        }
        let pool = crate::database::runtime_pool_options(config.database_max_connections, true)
            .connect_lazy(&config.database_url)
            .map_err(|_| "invalid Lambda database configuration")?;
        let state = AppState::from_config(config, loom_store::PgStore::new(pool));
        Ok(Self::from_state(state, policy))
    }

    fn from_state(state: AppState, policy: LambdaPolicy) -> Self {
        Self {
            router: build_router(state.clone()),
            state,
            policy,
            schema: Arc::new(OnceCell::new()),
            admission: Arc::new(Semaphore::new(1)),
            poisoned: Arc::new(AtomicBool::new(false)),
            retire: Arc::new(|| std::process::exit(1)),
            work_budget: Duration::from_secs(18),
            cleanup_budget: Duration::from_secs(2),
        }
    }

    /// Handle a buffered Function URL request. No model or write is retried.
    /// In production unresolved cleanup exits the process without acknowledgement.
    pub async fn handle(&self, request: lambda_http::Request) -> Response<Body> {
        if self.poisoned.load(Ordering::SeqCst) {
            return failure(StatusCode::SERVICE_UNAVAILABLE);
        }
        if !self.policy.validate(&request) {
            return failure(StatusCode::FORBIDDEN);
        }
        let path = request.raw_http_path().to_owned();
        if !allowed_route(request.method(), &path) {
            return failure(StatusCode::NOT_FOUND);
        }
        if request.body().as_ref().len() > MAX_REQUEST {
            return failure(StatusCode::PAYLOAD_TOO_LARGE);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let deadline = request
            .lambda_context_ref()
            .map_or(0, |c| u128::from(c.deadline));
        let remaining = Duration::from_millis(
            deadline.saturating_sub(now + 100).min(u128::from(u64::MAX)) as u64,
        );
        let Some(work_budget) = remaining
            .checked_sub(self.cleanup_budget)
            .filter(|d| !d.is_zero())
        else {
            return failure(StatusCode::SERVICE_UNAVAILABLE);
        };
        let work_budget = work_budget.min(self.work_budget);
        let Ok(permit) = self.admission.clone().try_acquire_owned() else {
            return failure(StatusCode::CONFLICT);
        };
        let this = self.clone();
        // Dropping JoinHandle detaches the supervisor; request cancellation must
        // not cancel the cleanup barrier or release admission prematurely.
        let supervisor = tokio::spawn(async move {
            let mut guard = RetirementGuard {
                poisoned: this.poisoned.clone(),
                retire: this.retire.clone(),
                armed: true,
            };
            let cleanup_deadline = tokio::time::Instant::now() + work_budget + this.cleanup_budget;
            let response =
                match tokio::time::timeout(work_budget, this.dispatch(request, &path)).await {
                    Ok(response) => response,
                    Err(_) => failure(StatusCode::GATEWAY_TIMEOUT),
                };
            let cleanup_deadline =
                cleanup_deadline.min(tokio::time::Instant::now() + this.cleanup_budget);
            let settled = tokio::time::timeout_at(cleanup_deadline, async {
                while !this.state.request_work_idle() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .is_ok();
            if !settled {
                // The production callback exits. Test callbacks can return; the
                // poisoned environment still refuses all subsequent requests.
                drop(guard);
                return failure(StatusCode::SERVICE_UNAVAILABLE);
            }
            guard.armed = false;
            drop(permit);
            response
        });
        supervisor
            .await
            .unwrap_or_else(|_| failure(StatusCode::SERVICE_UNAVAILABLE))
    }

    async fn dispatch(&self, request: lambda_http::Request, path: &str) -> Response<Body> {
        if path != "/healthz"
            && self
                .schema
                .get_or_try_init(|| async {
                    loom_store::verify_migrations(self.state.store().pool()).await
                })
                .await
                .is_err()
        {
            return failure(StatusCode::SERVICE_UNAVAILABLE);
        }
        let mut builder = Request::builder()
            .method(request.method().clone())
            .uri(path);
        // Rebuild headers, discarding outer SigV4 credentials, host, cookies and
        // forwarding claims. Application auth remains checked by tenant_auth.
        for name in ["content-type", "accept", "x-request-id"] {
            if let Some(value) = request.headers().get(name) {
                builder = builder.header(name, value);
            }
        }
        if let Some(value) = request.headers().get("x-tandem-authorization") {
            builder = builder.header("authorization", value);
        }
        let Ok(internal) = builder.body(AxumBody::from(request.body().as_ref().to_vec())) else {
            return failure(StatusCode::BAD_REQUEST);
        };
        let response = self
            .router
            .clone()
            .oneshot(internal)
            .await
            .unwrap_or_else(|never| match never {});
        let (parts, body) = response.into_parts();
        let Ok(bytes) = to_bytes(body, MAX_RESPONSE).await else {
            return failure(StatusCode::BAD_GATEWAY);
        };
        let mut response = Response::new(Body::Binary(bytes.to_vec()));
        *response.status_mut() = parts.status;
        for name in ["content-type", "x-request-id", "retry-after"] {
            if let Some(value) = parts.headers.get(name) {
                response.headers_mut().insert(name, value.clone());
            }
        }
        response
    }
}

struct RetirementGuard {
    poisoned: Arc<AtomicBool>,
    retire: Arc<dyn Fn() + Send + Sync>,
    armed: bool,
}
impl Drop for RetirementGuard {
    fn drop(&mut self) {
        if self.armed {
            self.poisoned.store(true, Ordering::SeqCst);
            (self.retire)();
        }
    }
}

fn failure(status: StatusCode) -> Response<Body> {
    let mut response = Response::new(Body::Text(
        "{\"error\":\"private_gateway_unavailable\"}".into(),
    ));
    *response.status_mut() = status;
    response.headers_mut().insert(
        "content-type",
        http::HeaderValue::from_static("application/json"),
    );
    response
}

#[cfg(test)]
mod tests;
