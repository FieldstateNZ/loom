//! Bounded Loom-owned model/tool loop. Every intermediate message is durable,
//! each inference is metered, and final success follows history persistence.
//! A dropped response drops the actual execution future; it is not detached.
use super::super::{
    runner::attribution::{record_turn_usage, turn_cost, UsageAttribution},
    turn_response::TurnResponse,
};
use super::{collect, mark, Prepared};
use crate::{auth::TenantContext, budget::BudgetWarning, error::ApiError, state::AppState};
use axum::{
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures::stream;
use loom_core::{ContentPart, Conversation, ConversationOptions, Message, Role, Usage};
use loom_provider::{Provider, StopReason, TurnEvent, TurnEventKind};
use loom_store::ConversationStore;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use sha2::Digest;
use std::{
    collections::{BTreeSet, VecDeque},
    convert::Infallible,
    sync::Arc,
    time::{Duration, Instant},
};

struct Run {
    provider: Arc<dyn Provider>,
    state: AppState,
    conversation: Conversation,
    options: ConversationOptions,
    ctx: TenantContext,
    prepared: Prepared,
    attribution: UsageAttribution,
    rounds: usize,
    calls: usize,
    seen: BTreeSet<String>,
    usage: Usage,
    cost: Option<Decimal>,
    started: Instant,
    events: VecDeque<TurnEvent>,
    done: bool,
    pending: VecDeque<(String, String, Value)>,
    proposed: bool,
    remembered: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute(
    provider: Arc<dyn Provider>,
    state: &AppState,
    conversation: Conversation,
    options: ConversationOptions,
    streaming: bool,
    ctx: TenantContext,
    warning: Option<BudgetWarning>,
    prepared: Prepared,
) -> Result<Response, ApiError> {
    let attribution = UsageAttribution {
        tenant_id: ctx.tenant_id,
        virtual_key_id: Some(ctx.key_id),
        conversation_id: Some(conversation.id),
        provider: conversation.binding.provider.clone(),
        model: conversation.binding.model.clone(),
    };
    let seen = conversation
        .messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|p| {
            if let ContentPart::ToolUse { id, .. } = p {
                Some(id.clone())
            } else {
                None
            }
        })
        .collect();
    let mut run = Run {
        provider,
        state: state.clone(),
        conversation,
        options,
        ctx,
        prepared,
        attribution,
        rounds: 0,
        calls: 0,
        seen,
        usage: Usage::default(),
        cost: Some(Decimal::ZERO),
        started: Instant::now(),
        events: VecDeque::new(),
        done: false,
        pending: VecDeque::new(),
        proposed: false,
        remembered: false,
    };
    let mut response = if streaming {
        run.events
            .push_back(TurnEvent::new(TurnEventKind::TurnStarted, Value::Null));
        let body = stream::unfold(run, |mut run| async move {
            loop {
                if let Some(event) = run.events.pop_front() {
                    return Some((
                        Ok::<_, Infallible>(
                            Event::default()
                                .json_data(event)
                                .unwrap_or_else(|_| Event::default().data("{}")),
                        ),
                        run,
                    ));
                }
                if run.done {
                    return None;
                }
                let remaining = Duration::from_secs(60).saturating_sub(run.started.elapsed());
                match tokio::time::timeout(remaining, run.step()).await {
                    Ok(Ok(Some((message, stop)))) => {
                        run.final_events(message, stop);
                        run.done = true;
                    }
                    Ok(Ok(None)) => {}
                    result => {
                        run.done = true;
                        let error = match result {
                            Ok(Err(e)) => e,
                            _ => ApiError::unavailable(
                                "private memory run timed out; start a new conversation",
                            ),
                        };
                        return Some((
                            Ok(Event::default()
                                .event("error")
                                .json_data(error.envelope())
                                .unwrap_or_else(|_| Event::default().event("error").data("{}"))),
                            run,
                        ));
                    }
                }
            }
        });
        Sse::new(body)
            .keep_alive(
                KeepAlive::new()
                    .interval(Duration::from_secs(5))
                    .text("loom"),
            )
            .into_response()
    } else {
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if let Some((mut message, _)) = run.step().await? {
                    message.usage = Some(run.usage.clone());
                    return Ok::<_, ApiError>(message);
                }
            }
        })
        .await
        .map_err(|_| {
            ApiError::unavailable("private memory run timed out; start a new conversation")
        })??;
        Json(TurnResponse {
            message: result,
            cost: turn_cost(run.cost),
        })
        .into_response()
    };
    response
        .headers_mut()
        .insert("x-accel-buffering", http::HeaderValue::from_static("no"));
    if let Some(warning) = warning {
        if let Ok(value) = http::HeaderValue::from_str(warning.header_value()) {
            response
                .headers_mut()
                .insert("x-loom-budget-warning", value);
        }
    }
    Ok(response)
}

impl Run {
    async fn append(&mut self, message: Message) -> Result<(), ApiError> {
        self.state
            .store()
            .append_message(self.ctx.tenant_id, self.conversation.id, &message)
            .await
            .map_err(ApiError::from_store)?
            .ok_or_else(|| ApiError::conflict("private memory conversation was removed"))?;
        self.conversation.messages.push(message);
        Ok(())
    }

    async fn step(&mut self) -> Result<Option<(Message, StopReason)>, ApiError> {
        if let Some((id, name, input)) = self.pending.pop_front() {
            self.calls += 1;
            let (value, is_error) = if name == "propose_task" {
                let input = loom_mcp::TaskInput::parse(&input).map_err(|_| {
                    ApiError::unprocessable("gateway_action_input", "invalid task proposal")
                })?;
                let action = super::super::actions::propose(
                    &self.state,
                    self.ctx.tenant_id,
                    self.conversation.id,
                    &id,
                    input,
                    &self.prepared,
                )
                .await?;
                self.proposed = true;
                let data = serde_json::to_value(action).map_err(|_| ApiError::internal())?;
                self.events.push_back(TurnEvent::new(
                    TurnEventKind::Other {
                        native_type: Some("loom_action_proposed".into()),
                    },
                    data.clone(),
                ));
                (
                    json!({"status":"requires_application_approval","action_id":data["id"],"created":false}),
                    false,
                )
            } else if name == "memory_remember" {
                self.remembered = true;
                let operation = hex::encode(sha2::Sha256::digest(
                    serde_json::to_vec(&(
                        "tandem_memory_v1",
                        self.ctx.tenant_id,
                        self.conversation.id,
                        &id,
                    ))
                    .map_err(|_| ApiError::internal())?,
                ));
                self.prepared.client.remember(&operation, &input).await.map_err(|_| {
                    ApiError::unavailable("memory save outcome is unconfirmed; do not claim success or retry as a fresh save")
                })?
            } else {
                self.prepared
                    .client
                    .call(&name, &input)
                    .await
                    .map_err(|_| {
                        ApiError::unavailable(
                        "private memory read was interrupted or denied; start a new conversation",
                    )
                    })?
            };
            self.append(Message::new(
                Role::User,
                vec![ContentPart::ToolResult {
                    tool_use_id: id,
                    content: value,
                    is_error: Some(is_error),
                    cache: None,
                }],
            ))
            .await?;
            self.activity(&name, if is_error { "failed" } else { "completed" });
            if let Some((_, next, _)) = self.pending.front() {
                let next = next.clone();
                self.activity(&next, "started");
            }
            return Ok(None);
        }
        if self.rounds >= 4 {
            return Err(ApiError::unprocessable(
                "gateway_mcp_limit",
                "private memory model-round limit reached; start a new conversation",
            ));
        }
        if self.rounds > 0 {
            super::super::turns::enforce_limits(&self.state, &self.ctx).await?;
        }
        self.rounds += 1;
        let started = Instant::now();
        let stream = self
            .provider
            .stream(&self.conversation, &self.options)
            .await
            .map_err(|_| {
                ApiError::unavailable("private memory model request failed; outcome may be unknown")
            })?;
        let pass = collect::collect(stream).await;
        self.state.metrics().record_provider_call(
            &self.attribution.provider,
            &self.attribution.model,
            true,
            started.elapsed(),
        );
        if let Some(usage) = pass.usage {
            let cost = record_turn_usage(&self.state, &self.attribution, usage.clone()).await;
            self.cost = self.cost.zip(cost).map(|(a, b)| a + b);
            add_usage(&mut self.usage, &usage);
        } else {
            self.cost = None;
        }
        let (message, stop) = pass.result?;
        let calls: Vec<_> = message
            .content
            .iter()
            .filter_map(|part| {
                if let ContentPart::ToolUse {
                    id, name, input, ..
                } = part
                {
                    Some((id.clone(), name.clone(), input.clone()))
                } else {
                    None
                }
            })
            .collect();
        if calls.is_empty() {
            if !matches!(
                stop,
                StopReason::EndTurn | StopReason::StopSequence | StopReason::Refusal
            ) {
                return Err(ApiError::unavailable(
                    "private memory model response did not finish successfully",
                ));
            }
            self.append(message.clone()).await?;
            mark(
                &self.state,
                self.ctx.tenant_id,
                self.conversation.id,
                "idle",
            )
            .await?;
            return Ok(Some((message, stop)));
        }
        if stop != StopReason::ToolUse || self.calls + calls.len() > 4 {
            return Err(ApiError::unprocessable(
                "gateway_mcp_limit",
                "private memory tool-call limit or protocol violation",
            ));
        }
        if calls
            .iter()
            .filter(|(_, name, _)| name == "propose_task")
            .count()
            + usize::from(self.proposed)
            > 1
        {
            return Err(ApiError::unprocessable(
                "gateway_action_limit",
                "only one task may be proposed in a turn",
            ));
        }
        if calls
            .iter()
            .filter(|(_, name, _)| name == "memory_remember")
            .count()
            + usize::from(self.remembered)
            > 1
        {
            return Err(ApiError::unprocessable(
                "gateway_memory_limit",
                "only one memory save may be attempted in a turn",
            ));
        }
        // Validate the whole set before executing any call. Repeated IDs cannot
        // be replayed as fresh actions; only source-reviewed read tools exist.
        for (id, name, input) in &calls {
            if id.is_empty()
                || id.len() > 200
                || !self.seen.insert(id.clone())
                || !self.prepared.client.tools().iter().any(|t| &t.name == name)
                || input.to_string().len() > 8192
            {
                return Err(ApiError::unprocessable(
                    "gateway_mcp_tool",
                    "model requested an unoffered or duplicate tool call",
                ));
            }
            self.prepared.client.validate(name, input).map_err(|_| {
                ApiError::unprocessable(
                    "gateway_mcp_tool",
                    "model supplied invalid memory arguments",
                )
            })?;
        }
        self.append(message).await?;
        self.pending = calls.into();
        if let Some((_, name, _)) = self.pending.front() {
            let name = name.clone();
            self.activity(&name, "started");
        }
        Ok(None)
    }

    fn activity(&mut self, tool: &str, status: &str) {
        self.events.push_back(TurnEvent::new(
            TurnEventKind::Other {
                native_type: Some("loom_memory_activity".into()),
            },
            json!({"type":"loom_memory_activity","tool":tool,"status":status}),
        ));
    }
    fn final_events(&mut self, message: Message, stop: StopReason) {
        for (index, part) in message.content.into_iter().enumerate() {
            self.events.push_back(TurnEvent::new(
                TurnEventKind::ContentPartComplete { index, part },
                Value::Null,
            ));
        }
        self.events.push_back(TurnEvent::new(
            TurnEventKind::TurnEnded {
                stop_reason: stop,
                usage: Some(self.usage.clone()),
                cost: turn_cost(self.cost),
            },
            Value::Null,
        ));
    }
}

fn add_usage(total: &mut Usage, usage: &Usage) {
    fn add(a: &mut Option<u64>, b: Option<u64>) {
        if let Some(b) = b {
            *a = Some(a.unwrap_or(0).saturating_add(b));
        }
    }
    add(&mut total.input_tokens, usage.input_tokens);
    add(&mut total.output_tokens, usage.output_tokens);
    add(&mut total.cache_read_tokens, usage.cache_read_tokens);
    add(&mut total.cache_write_tokens, usage.cache_write_tokens);
    for (key, n) in &usage.server_tool_use {
        let current = total.server_tool_use.entry(key.clone()).or_default();
        *current = current.saturating_add(*n);
    }
    let raw = total.raw.get_or_insert_with(|| json!([]));
    if let Some(values) = raw.as_array_mut() {
        values.push(serde_json::to_value(usage).unwrap_or(Value::Null));
    }
}
