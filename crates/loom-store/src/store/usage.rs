//! Persistence for usage events and their rollups.

use async_trait::async_trait;
use uuid::Uuid;

use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::model::{NewUsageEvent, RollupGroup, UsageEvent, UsageRollup, UsageRollupRow};

/// Persistence for usage events and their rollups.
#[async_trait]
pub trait UsageStore {
    /// Records a usage event and returns its generated id.
    async fn record_event(&self, event: NewUsageEvent) -> Result<Uuid>;

    /// Lists a tenant's usage events, most recent first, capped by `limit`.
    async fn list_events(&self, tenant_id: Uuid, limit: i64) -> Result<Vec<UsageEvent>>;

    /// Rolls a tenant's usage up into aggregate token totals.
    async fn rollup(&self, tenant_id: Uuid) -> Result<UsageRollup>;

    /// Rolls a tenant's usage up into grouped token and cost totals over an
    /// optional `[from, to]` time window (inclusive; `None` bounds are open).
    ///
    /// `group_by` selects the grouping dimension; passing
    /// [`RollupGroup::Tenant`] here is a caller error and yields an empty
    /// result — gateway-wide reporting uses [`Self::rollup_by_tenant`].
    async fn rollup_grouped(
        &self,
        tenant_id: Uuid,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        group_by: RollupGroup,
    ) -> Result<Vec<UsageRollupRow>>;

    /// Rolls **all** tenants' usage up, grouped by tenant, over an optional
    /// time window. Gateway-wide; not tenant-scoped — for the root-token admin
    /// query only.
    async fn rollup_by_tenant(
        &self,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> Result<Vec<UsageRollupRow>>;
}
