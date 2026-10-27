//! Host-only task delivery and outcome lookup. Inference can only propose text.
use crate::{Error, MemoryClient, Mode};
use loom_core::ToolDefinition;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

/// Text proposed by inference. No actor, operation, category or workspace IDs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    /// Exact title displayed for approval.
    pub title: String,
    /// Optional detail displayed for approval.
    #[serde(default)]
    pub detail: Option<String>,
}
impl TaskInput {
    /// Parses the small, reviewed proposal shape without accepting host fields.
    pub fn parse(value: &Value) -> Result<Self, Error> {
        let input: Self = serde_json::from_value(value.clone()).map_err(|_| Error::Tool)?;
        if input.title.trim().is_empty()
            || input.title.chars().count() > 200
            || input
                .detail
                .as_ref()
                .is_some_and(|s| s.chars().count() > 2000)
        {
            return Err(Error::Tool);
        }
        Ok(input)
    }
}

/// Actual active category metadata verified in the pinned workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskCategory {
    /// LucidBrain category UUID.
    pub id: Uuid,
    /// Current human-readable category name.
    pub name: String,
}

/// Minimal immutable receipt returned by LucidBrain's atomic capture contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureReceipt {
    /// Application-owned operation UUID, retained on every retry.
    pub operation_id: Uuid,
    /// Must be `committed`; a tool error is never success.
    pub status: String,
    /// UUID of the originally created task (which may later have been deleted).
    pub item_id: Uuid,
    /// Original workspace.
    pub workspace_id: Uuid,
    /// Original commit timestamp.
    pub committed_at: String,
}

pub(super) fn proposal_tool() -> ToolDefinition {
    ToolDefinition { name: "propose_task".into(),
        description: Some("Prepare a task for explicit approval in Tandem. This does not create, schedule or delegate it. Never claim creation until the application reports a committed receipt. The user must use the application's confirmation control; conversation text is not approval.".into()),
        input_schema: json!({"type":"object","properties":{"title":{"type":"string","minLength":1,"maxLength":200},"detail":{"type":["string","null"],"maxLength":2000}},"required":["title"],"additionalProperties":false}), cache: None }
}

fn decoded(value: Value) -> Result<Value, Error> {
    let parts = value["content"].as_array().ok_or(Error::Transport)?;
    if parts.len() != 1 || parts[0]["type"] != "text" {
        return Err(Error::Transport);
    }
    serde_json::from_str(parts[0]["text"].as_str().ok_or(Error::Transport)?)
        .map_err(|_| Error::Transport)
}

impl MemoryClient {
    /// Reads actual category metadata. The model cannot choose the workspace.
    pub async fn task_category(&self, slug: &str, id: Uuid) -> Result<TaskCategory, Error> {
        if !matches!(self.mode, Mode::Tasks | Mode::TasksAndMemory)
            || slug.is_empty()
            || slug.len() > 100
        {
            return Err(Error::Configuration);
        }
        let (value, error) = self
            .call_private("list_categories", &json!({"workspaceSlug":slug}))
            .await?;
        if error {
            return Err(Error::Binding);
        }
        let value = decoded(value)?;
        let rows = value.as_array().ok_or(Error::Transport)?;
        let matches: Vec<_> = rows.iter().filter(|r| r["id"] == id.to_string()).collect();
        if matches.len() != 1 {
            return Err(Error::Binding);
        }
        let row = matches[0];
        let name = row["displayName"].as_str().ok_or(Error::Binding)?;
        if row["workspaceId"] != self.binding.workspace_id.to_string()
            || row["archived"] != false
            || name.trim().is_empty()
            || name.len() > 200
        {
            return Err(Error::Binding);
        }
        Ok(TaskCategory {
            id,
            name: name.into(),
        })
    }

    fn receipt(&self, value: Value, operation: Uuid) -> Result<CaptureReceipt, Error> {
        let receipt: CaptureReceipt =
            serde_json::from_value(value).map_err(|_| Error::Transport)?;
        if receipt.operation_id != operation
            || receipt.workspace_id != self.binding.workspace_id
            || receipt.status != "committed"
            || receipt.committed_at.len() > 64
            || receipt
                .committed_at
                .parse::<chrono::DateTime<chrono::Utc>>()
                .is_err()
        {
            return Err(Error::Binding);
        }
        Ok(receipt)
    }

    /// Executes one approved immutable capture with a host-owned operation ID.
    /// No automatic retries; callers must persist dispatch before invoking this.
    pub async fn capture_task(
        &self,
        operation: Uuid,
        category: Uuid,
        input: &TaskInput,
    ) -> Result<CaptureReceipt, Error> {
        if !matches!(self.mode, Mode::Tasks | Mode::TasksAndMemory) {
            return Err(Error::Tool);
        }
        TaskInput::parse(&serde_json::to_value(input).map_err(|_| Error::Tool)?)?;
        let mut args = json!({"operationId":operation,"categoryId":category,"title":input.title});
        if let Some(detail) = &input.detail {
            args["detail"] = json!(detail);
        }
        let (value, error) = self.call_private("capture_item", &args).await?;
        if error {
            return Err(Error::Transport);
        }
        self.receipt(decoded(value)?, operation)
    }

    /// Looks up an outcome without a write. Absence is only point-in-time.
    pub async fn capture_receipt(&self, operation: Uuid) -> Result<Option<CaptureReceipt>, Error> {
        if self.mode == Mode::Memory {
            return Err(Error::Tool);
        }
        let (value, error) = self
            .call_private("get_capture_operation", &json!({"operationId":operation}))
            .await?;
        if error {
            return Err(Error::Transport);
        }
        let value = decoded(value)?;
        if value["operationId"] == operation.to_string()
            && value["status"] == "not_observed"
            && value.as_object().is_some_and(|o| o.len() == 2)
        {
            return Ok(None);
        }
        self.receipt(value, operation).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proposal_cannot_choose_delivery_or_scope() {
        assert!(TaskInput::parse(&json!({"title":"Call the builder"})).is_ok());
        for args in [
            json!({"title":" "}),
            json!({"title":"x","operationId":Uuid::new_v4()}),
            json!({"title":"x","categoryId":Uuid::new_v4()}),
            json!({"title":"x","workspaceSlug":"other"}),
            json!({"title":"x","detail":"x".repeat(2001)}),
        ] {
            assert!(TaskInput::parse(&args).is_err());
        }
    }
}
