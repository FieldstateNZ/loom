//! Lossless Responses request/response translation. Native message metadata and
//! reasoning use provider extensions alongside normalized text/function parts.

use loom_core::{
    CacheNegotiation, Citation, ContentPart, Conversation, ConversationOptions, MediaSource,
    Message, Role, Usage,
};
use loom_provider::{Capability, ProviderError, StopReason};
use serde_json::{json, Value};

use crate::PROVIDER_NAME;

pub(crate) fn invalid(message: &str) -> ProviderError {
    ProviderError::Serialization(message.to_owned())
}

fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str, ProviderError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing OpenAI string field"))
}

/// Translates a full Loom conversation into a stateless native Responses body.
/// Unsupported content/options fail before an HTTP request is made.
pub fn request(
    conversation: &Conversation,
    options: &ConversationOptions,
    stream: bool,
) -> Result<Value, ProviderError> {
    if !options.stop_sequences.is_empty() {
        return Err(invalid("Responses does not support Loom stop_sequences"));
    }
    if !options.server_tools.is_empty() || !options.mcp_servers.is_empty() {
        return Err(invalid(
            "OpenAI hosted tools and MCP connectors are not implemented by this adapter",
        ));
    }
    let hints = options.auto_cache
        || conversation.system_cache.is_some()
        || options.tools.iter().any(|tool| tool.cache.is_some())
        || conversation
            .messages
            .iter()
            .flat_map(|message| &message.content)
            .any(|part| match part {
                ContentPart::Text { cache, .. }
                | ContentPart::Image { cache, .. }
                | ContentPart::Document { cache, .. }
                | ContentPart::ToolUse { cache, .. }
                | ContentPart::ToolResult { cache, .. }
                | ContentPart::Thinking { cache, .. } => cache.is_some(),
                _ => false,
            });
    if hints && options.cache_negotiation == CacheNegotiation::HardFail {
        return Err(ProviderError::CapabilityUnsupported {
            capability: Capability::PromptCaching,
            provider: PROVIDER_NAME.into(),
            model: conversation.binding.model.clone(),
        });
    }
    if hints {
        tracing::warn!(provider=PROVIDER_NAME,"OpenAI ignores advisory Loom cache breakpoints; native automatic caching remains provider-controlled");
    }
    let mut input = Vec::new();
    for message in &conversation.messages {
        append_message(message, &mut input)?;
    }
    let mut body = json!({"model":conversation.binding.model,"input":input,"store":false,"stream":stream,
        "max_output_tokens":options.max_tokens.unwrap_or(4096)});
    if let Some(system) = &conversation.system {
        body["instructions"] = json!(system);
    }
    if let Some(temperature) = options.temperature {
        body["temperature"] = json!(temperature);
    }
    // Preserve the current Tandem model and its low-latency setting. An
    // operator-added model gets its own native default unless explicitly set.
    if conversation.binding.model == "gpt-6-luna" {
        body["reasoning"] = json!({"effort":"none"});
    }
    if !options.tools.is_empty() {
        body["tools"] = options.tools.iter().map(|tool| {
            // Generic Loom schemas can have optional fields. Explicit false
            // prevents Responses' implicit strict normalization changing them.
            let mut native = json!({"type":"function","name":tool.name,"parameters":tool.input_schema,"strict":false});
            if let Some(description) = &tool.description { native["description"] = json!(description); }
            native
        }).collect();
    }
    if let Some(native) = options.provider_options.get(PROVIDER_NAME) {
        let object = native
            .as_object()
            .ok_or_else(|| invalid("OpenAI native options must be an object"))?;
        for (key, value) in object {
            if [
                "model",
                "input",
                "instructions",
                "store",
                "stream",
                "previous_response_id",
                "conversation",
                "background",
                "tools",
                "max_output_tokens",
            ]
            .contains(&key.as_str())
            {
                return Err(invalid("OpenAI native options cannot override Loom-owned history, persistence or tools"));
            }
            body[key] = value.clone();
        }
    }
    Ok(body)
}

fn flush(
    input: &mut Vec<Value>,
    role: &str,
    metadata: &mut Option<Value>,
    content: &mut Vec<Value>,
) {
    if content.is_empty() && metadata.is_none() {
        return;
    }
    let item = if let Some(mut native) = metadata.take() {
        native["content"] = Value::Array(std::mem::take(content));
        native
    } else {
        json!({"role":role,"content":std::mem::take(content)})
    };
    input.push(item);
}

fn append_message(message: &Message, input: &mut Vec<Value>) -> Result<(), ProviderError> {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant | Role::Provider => "assistant",
        _ => return Err(invalid("unsupported message role")),
    };
    let mut metadata = None;
    let mut call_metadata = None;
    let mut content = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text {
                text, citations, ..
            } => {
                let native = if metadata.is_some() {
                    json!({"type":"output_text","text":text,"annotations":citations.as_ref().map(|v| v.iter().map(|c| c.0.clone()).collect::<Vec<_>>()).unwrap_or_default()})
                } else {
                    json!({"type":"input_text","text":text})
                };
                if metadata.is_none() && citations.as_ref().is_some_and(|c| !c.is_empty()) {
                    return Err(invalid(
                        "cited assistant text requires its OpenAI message metadata",
                    ));
                }
                content.push(native);
            }
            ContentPart::Image { source, .. } if role == "user" => {
                let url = match source {
                    MediaSource::Url { url } => url.clone(),
                    MediaSource::Base64 { media_type, data } => {
                        format!("data:{media_type};base64,{data}")
                    }
                    _ => return Err(invalid("unsupported image source")),
                };
                content.push(json!({"type":"input_image","image_url":url}));
            }
            ContentPart::ToolUse {
                id,
                name,
                input: arguments,
                ..
            } => {
                flush(input, role, &mut metadata, &mut content);
                let mut call = call_metadata
                    .take()
                    .unwrap_or_else(|| json!({"type":"function_call"}));
                call["call_id"] = json!(id);
                call["name"] = json!(name);
                let original = call.get("arguments").and_then(Value::as_str);
                if original
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .as_ref()
                    != Some(arguments)
                {
                    call["arguments"] = json!(arguments.to_string());
                }
                input.push(call);
            }
            ContentPart::ToolResult {
                tool_use_id,
                content: output,
                is_error,
                ..
            } => {
                flush(input, role, &mut metadata, &mut content);
                let result = match is_error {
                    Some(is_error) => json!({"is_error":is_error,"content":output}).to_string(),
                    None => output
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| output.to_string()),
                };
                input.push(
                    json!({"type":"function_call_output","call_id":tool_use_id,"output":result}),
                );
            }
            ContentPart::ProviderExtension {
                provider,
                kind,
                payload,
            } if provider == PROVIDER_NAME => match kind.as_str() {
                "message_metadata" => {
                    flush(input, role, &mut metadata, &mut content);
                    if role != "assistant"
                        || payload.get("role").and_then(Value::as_str) != Some("assistant")
                        || payload.get("type").and_then(Value::as_str) != Some("message")
                    {
                        return Err(invalid("invalid OpenAI assistant message metadata"));
                    }
                    metadata = Some(payload.clone());
                }
                "function_call_metadata" => {
                    flush(input, role, &mut metadata, &mut content);
                    if call_metadata.is_some()
                        || payload.get("type").and_then(Value::as_str) != Some("function_call")
                    {
                        return Err(invalid("invalid OpenAI function metadata"));
                    }
                    call_metadata = Some(payload.clone());
                }
                "incomplete_function_call" => {
                    // Retain the native cutoff for inspection, but do not send
                    // a dangling call back as an executable history item.
                    return Err(invalid(
                        "incomplete OpenAI function calls cannot be replayed; start a new conversation",
                    ));
                }
                "refusal" | "message_content" => {
                    if metadata.is_none() {
                        return Err(invalid("OpenAI message content needs its native metadata"));
                    }
                    content.push(payload.clone());
                }
                _ => {
                    flush(input, role, &mut metadata, &mut content);
                    input.push(payload.clone());
                }
            },
            _ => {
                return Err(invalid(
                    "content cannot be represented by the OpenAI Responses adapter",
                ))
            }
        }
    }
    if call_metadata.is_some() {
        return Err(invalid("OpenAI function metadata has no tool call"));
    }
    flush(input, role, &mut metadata, &mut content);
    Ok(())
}

fn extension(kind: &str, payload: &Value) -> ContentPart {
    ContentPart::ProviderExtension {
        provider: PROVIDER_NAME.into(),
        kind: kind.into(),
        payload: payload.clone(),
    }
}

/// Metadata for a native item, kept ahead of its normalized content so replay
/// preserves message IDs, phases, statuses and additional provider fields.
pub(crate) fn metadata(item: &Value) -> ContentPart {
    let mut native = item.clone();
    if let Some(object) = native.as_object_mut() {
        object.remove("content");
    }
    extension("message_metadata", &native)
}

/// Converts one native message content block, preserving unknown annotations
/// and content blocks without flattening them into ordinary text.
pub fn content_part(part: &Value) -> Result<ContentPart, ProviderError> {
    Ok(match string(part, "type")? {
        "output_text" => ContentPart::Text {
            text: string(part, "text")?.into(),
            cache: None,
            citations: part
                .get("annotations")
                .and_then(Value::as_array)
                .filter(|v| !v.is_empty())
                .map(|v| v.iter().cloned().map(Citation).collect()),
        },
        "refusal" => {
            string(part, "refusal")?;
            extension("refusal", part)
        }
        _ => extension("message_content", part),
    })
}

/// Converts a complete Responses output item to ordered Loom content parts.
pub fn output_item(item: &Value) -> Result<Vec<ContentPart>, ProviderError> {
    Ok(match string(item, "type")? {
        "message" => {
            if string(item, "role")? != "assistant" {
                return Err(invalid("OpenAI response message is not assistant-authored"));
            }
            let content = item
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("invalid OpenAI message content"))?;
            let mut parts = vec![metadata(item)];
            for part in content {
                parts.push(content_part(part)?);
            }
            parts
        }
        "function_call" if item.get("status").and_then(Value::as_str) != Some("completed") => vec![
            extension("function_call_metadata", item),
            // A cutoff can leave syntactically invalid arguments. Preserve it
            // without parsing or emitting a completed ToolUse, so terminal
            // reason/usage are still delivered by the stream accumulator.
            extension("incomplete_function_call", item),
        ],
        "function_call" => vec![
            extension("function_call_metadata", item),
            ContentPart::ToolUse {
                id: string(item, "call_id")?.into(),
                name: string(item, "name")?.into(),
                input: serde_json::from_str(string(item, "arguments")?)
                    .map_err(|_| invalid("OpenAI tool arguments are not valid JSON"))?,
                cache: None,
            },
        ],
        kind => vec![extension(kind, item)],
    })
}

/// Normalizes usage: OpenAI input tokens include cache hits, whereas Loom's
/// `input_tokens` represents only full-price input. Keep native usage as well.
#[must_use]
pub fn usage(value: &Value) -> Usage {
    let mut result = Usage::new();
    let cached = value
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(Value::as_u64);
    result.input_tokens = value
        .get("input_tokens")
        .and_then(Value::as_u64)
        .map(|n| n.saturating_sub(cached.unwrap_or(0)));
    result.output_tokens = value.get("output_tokens").and_then(Value::as_u64);
    result.cache_read_tokens = cached;
    result.raw = Some(value.clone());
    result
}

/// Maps a complete native response, including reasoning and unknown items.
pub fn response(native: &Value) -> Result<Message, ProviderError> {
    let output = native
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("OpenAI response has no output array"))?;
    let mut content = Vec::new();
    for item in output {
        content.extend(output_item(item)?);
    }
    Ok(Message {
        role: Role::Assistant,
        content,
        usage: native.get("usage").filter(|v| !v.is_null()).map(usage),
        raw: Some(native.clone()),
    })
}

/// Determines why generation stopped without treating a pending tool call or
/// incomplete response as a completed spoken answer.
pub fn stop_reason(native: &Value) -> StopReason {
    if native.get("status").and_then(Value::as_str) == Some("incomplete") {
        return match native
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
        {
            Some("max_output_tokens") => StopReason::MaxTokens,
            reason => StopReason::Other(format!("incomplete:{}", reason.unwrap_or("unknown"))),
        };
    }
    let output = native.get("output").and_then(Value::as_array);
    if output.is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "function_call" && item["status"] == "completed")
    }) {
        return StopReason::ToolUse;
    }
    if output.is_some_and(|items| items.iter().any(|item| item["type"] == "function_call")) {
        return StopReason::Other("incomplete:function_call".into());
    }
    if output.is_some_and(|items| {
        items
            .iter()
            .filter_map(|item| item["content"].as_array())
            .flatten()
            .any(|part| part["type"] == "refusal")
    }) {
        return StopReason::Refusal;
    }
    StopReason::EndTurn
}

pub(crate) fn response_error(payload: Value) -> ProviderError {
    ProviderError::Api {
        status: None,
        message: "OpenAI response did not complete".into(),
        payload: Some(payload),
    }
}
