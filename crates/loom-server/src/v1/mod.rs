//! The `/v1` conversation API: tenant-scoped conversations and turns.
//!
//! Every route here is guarded by [`tenant_auth`](crate::auth::tenant_auth), so
//! handlers receive a resolved [`TenantContext`](crate::auth::TenantContext) and
//! scope every store call to it. The turn endpoints resolve the bound
//! [`Provider`](loom_provider::Provider) through the [`AppState`], run
//! capability negotiation, and either return the assistant `Message`
//! (non-streaming) or an SSE stream of `TurnEvent` envelopes (streaming). The
//! stateful and stateless turn paths share one core runner
//! ([`runner::execute_turn`]) so their behaviour cannot drift.
//!
//! # Layout
//!
//! - [`requests`] — the request DTOs (`CreateConversationRequest`,
//!   `TurnRequest`, `StatelessTurnRequest`).
//! - [`whoami`] — `GET /v1/whoami`.
//! - [`conversations`] — conversation create/fetch/delete handlers.
//! - [`turns`] — the stateful and stateless turn handlers, plus
//!   [`turns::enforce_limits`] (shared with the batch API).
//! - [`runner`] — the turn runner and SSE reassembly.
//! - [`usage`] — `GET /v1/usage`.

mod conversations;
mod requests;
mod runner;
mod turns;
mod usage;
mod whoami;

pub use requests::{CreateConversationRequest, StatelessTurnRequest, TurnRequest};

pub(crate) use turns::enforce_limits;

use axum::routing::{get, post};
use axum::{Json, Router};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::state::AppState;

/// Builds the `/v1` sub-router (without its auth layer, which the top-level
/// router applies).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/whoami", get(whoami::whoami))
        .route(
            "/v1/conversations",
            post(conversations::create_conversation),
        )
        .route(
            "/v1/conversations/{id}",
            get(conversations::get_conversation).delete(conversations::delete_conversation),
        )
        .route("/v1/conversations/{id}/turns", post(turns::create_turn))
        .route("/v1/turns", post(turns::stateless_turn))
        .route("/v1/usage", get(usage::usage_rollup))
}

/// Registers the `virtual_key` and `admin_token` security schemes referenced by
/// every guarded operation.
///
/// The tenant-scoped operations declare `security(("virtual_key" = []))` and
/// the `/admin` operations declare `security(("admin_token" = []))`, so both
/// schemes must exist in `components.securitySchemes` for the published
/// document to be a valid, self-consistent OpenAPI spec (no dangling
/// references). Both are HTTP bearer schemes: the tenant presents its virtual
/// key (`Authorization: Bearer loom_...`) and the operator presents the root
/// admin token, respectively.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::default);
        components.add_security_scheme(
            "virtual_key",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("virtual key")
                    .build(),
            ),
        );
        components.add_security_scheme(
            "admin_token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("admin token")
                    .build(),
            ),
        );
    }
}

/// The OpenAPI document for the gateway's HTTP surface.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Loom gateway API",
        description = "Multi-tenant LLM gateway: tenant-scoped conversations and turns over pluggable providers."
    ),
    modifiers(&SecurityAddon),
    paths(
        whoami::whoami,
        conversations::create_conversation,
        conversations::get_conversation,
        conversations::delete_conversation,
        turns::create_turn,
        turns::stateless_turn,
        usage::usage_rollup,
        crate::batch::create_batch,
        crate::batch::get_batch,
        crate::batch::get_batch_results,
        crate::batch::cancel_batch,
        crate::admin::usage_by_tenant,
    ),
    components(schemas(
        CreateConversationRequest,
        TurnRequest,
        StatelessTurnRequest,
        crate::batch::CreateBatchRequest,
        crate::batch::BatchItemInput,
        crate::batch::BatchJobDto,
        crate::batch::BatchCountsDto,
        // The domain model (#18): the conversation/turn/message shapes that
        // flow through request *and* response bodies. Explicitly listed so
        // every variant closure (content parts, media sources, citations,
        // cache hints, server tools, …) is present in the document even where
        // a given type is only reachable through a nested field.
        loom_core::Conversation,
        loom_core::ProviderBinding,
        loom_core::Message,
        loom_core::Role,
        loom_core::ContentPart,
        loom_core::MediaSource,
        loom_core::Citation,
        loom_core::Usage,
        loom_core::ConversationOptions,
        loom_core::ToolDefinition,
        loom_core::ServerTool,
        loom_core::McpServerRef,
        loom_core::CacheHint,
        loom_core::CacheTtl,
        loom_core::CacheNegotiation,
        // The streaming envelope (#18).
        loom_provider::TurnEvent,
        loom_provider::TurnEventKind,
        loom_provider::ContentDelta,
        loom_provider::StopReason,
        // Response envelopes (#19).
        whoami::WhoAmI,
        usage::UsageRollupResponse,
        usage::UsageRollupRowDto,
        crate::admin::AdminUsageResponse,
        crate::admin::AdminUsageRow,
    )),
    tags(
        (name = "conversations", description = "Tenant-scoped conversation and turn endpoints"),
        (name = "usage", description = "Spend and token usage rollups"),
        (name = "batches", description = "Asynchronous batch jobs (bulk turns at the discounted batch tier)"),
        (name = "admin", description = "Root-token-guarded gateway administration"),
    )
)]
pub struct ApiDoc;

/// `GET /openapi.json` — the generated OpenAPI 3.x document.
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
