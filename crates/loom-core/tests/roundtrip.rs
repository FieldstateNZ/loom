//! Serde round-trip tests: serialize -> deserialize -> equal for a
//! representative value of every [`ContentPart`] variant, plus `Usage`,
//! `Message`, and `Conversation`.

use chrono::{TimeZone, Utc};
use loom_core::{
    CacheHint, CacheNegotiation, CacheTtl, Citation, ContentPart, Conversation,
    ConversationOptions, McpServerRef, MediaSource, Message, ProviderBinding, Role, ServerTool,
    ToolDefinition, Usage,
};
use serde_json::json;
use uuid::Uuid;

/// Asserts that a value survives a JSON serialize -> deserialize cycle
/// unchanged.
fn assert_json_roundtrip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let encoded = serde_json::to_string(value).expect("serialize");
    let decoded: T = serde_json::from_str(&encoded).expect("deserialize");
    assert_eq!(value, &decoded, "round-trip mismatch for {encoded}");
}

#[test]
fn content_part_variants_roundtrip() {
    let parts = vec![
        ContentPart::Text {
            text: "plain".into(),
            citations: None,
            cache: None,
        },
        // A cache hint on a text block round-trips.
        ContentPart::Text {
            text: "cacheable prefix".into(),
            citations: None,
            cache: Some(CacheHint::ephemeral()),
        },
        ContentPart::Text {
            text: "cited".into(),
            citations: Some(vec![Citation(json!({
                "type": "char_location",
                "cited_text": "x",
                "start_char_index": 0,
                "end_char_index": 1
            }))]),
            cache: None,
        },
        ContentPart::Image {
            source: MediaSource::Base64 {
                media_type: "image/png".into(),
                data: "aGVsbG8=".into(),
            },
            cache: None,
        },
        ContentPart::Image {
            source: MediaSource::Url {
                url: "https://example.com/a.png".into(),
            },
            cache: Some(CacheHint::with_ttl(CacheTtl::OneHour)),
        },
        ContentPart::Document {
            source: MediaSource::Base64 {
                media_type: "application/pdf".into(),
                data: "JVBERi0=".into(),
            },
            cache: Some(CacheHint::with_ttl(CacheTtl::FiveMinutes)),
        },
        ContentPart::ToolUse {
            id: "toolu_1".into(),
            name: "get_weather".into(),
            input: json!({ "location": "Wellington" }),
            cache: None,
        },
        ContentPart::ToolResult {
            tool_use_id: "toolu_1".into(),
            content: json!("18C and windy"),
            is_error: None,
            cache: None,
        },
        ContentPart::ToolResult {
            tool_use_id: "toolu_1".into(),
            content: json!({ "error": "not found" }),
            is_error: Some(true),
            cache: Some(CacheHint::ephemeral()),
        },
        ContentPart::ServerToolUse {
            id: "srvtoolu_1".into(),
            name: "web_search".into(),
            input: json!({ "query": "loom gateway" }),
        },
        ContentPart::ServerToolResult {
            tool_use_id: "srvtoolu_1".into(),
            content: json!([{ "type": "web_search_result", "url": "https://x" }]),
        },
        ContentPart::Thinking {
            thinking: "let me reason".into(),
            signature: Some("sig-blob".into()),
            cache: None,
        },
        ContentPart::Thinking {
            thinking: String::new(),
            signature: None,
            cache: None,
        },
        ContentPart::RedactedThinking {
            data: "opaque==".into(),
        },
        ContentPart::ProviderExtension {
            provider: "anthropic".into(),
            kind: "mcp_tool_use".into(),
            payload: json!({ "id": "mcptoolu_1", "server_name": "github" }),
        },
    ];

    for part in &parts {
        assert_json_roundtrip(part);
    }
}

#[test]
fn text_part_type_tag_is_self_describing() {
    let value = serde_json::to_value(ContentPart::text("hi")).unwrap();
    assert_eq!(value["type"], json!("text"));
    assert_eq!(value["text"], json!("hi"));
    // Absent citations must not serialize (preserves provider omission).
    assert!(value.get("citations").is_none());
}

#[test]
fn usage_roundtrips() {
    let mut usage = Usage::new();
    usage.input_tokens = Some(100);
    usage.output_tokens = Some(42);
    usage.cache_read_tokens = Some(10);
    usage.cache_write_tokens = Some(5);
    usage
        .server_tool_use
        .insert("web_search_requests".to_owned(), 3);
    usage.raw = Some(json!({ "input_tokens": 100, "service_tier": "standard" }));
    assert_json_roundtrip(&usage);
    assert_json_roundtrip(&Usage::new());
}

#[test]
fn message_roundtrips() {
    let message = Message {
        role: Role::Assistant,
        content: vec![
            ContentPart::Thinking {
                thinking: "hmm".into(),
                signature: Some("sig".into()),
                cache: None,
            },
            ContentPart::text("done"),
        ],
        usage: Some({
            let mut u = Usage::new();
            u.output_tokens = Some(7);
            u
        }),
        // The verbatim native payload must survive its own serde round-trip.
        raw: Some(serde_json::json!({ "id": "msg_1", "type": "message" })),
    };
    assert_json_roundtrip(&message);

    for role in [Role::User, Role::Assistant, Role::Provider] {
        assert_json_roundtrip(&Message::new(role, vec![ContentPart::text("x")]));
    }
}

#[test]
fn conversation_options_roundtrips() {
    let mut options = ConversationOptions::new();
    options.temperature = Some(0.7);
    options.max_tokens = Some(1024);
    options.stop_sequences = vec!["STOP".into()];
    options.tools = vec![ToolDefinition {
        name: "get_weather".into(),
        description: Some("Look up the weather".into()),
        input_schema: json!({ "type": "object", "properties": {} }),
        cache: Some(CacheHint::with_ttl(CacheTtl::OneHour)),
    }];
    options.auto_cache = true;
    options.cache_negotiation = CacheNegotiation::HardFail;
    options.server_tools = vec![
        ServerTool::WebSearch {
            max_uses: Some(5),
            allowed_domains: Some(vec!["example.com".into()]),
            blocked_domains: None,
        },
        ServerTool::CodeExecution {},
        // The Raw passthrough carries a native tool definition verbatim.
        ServerTool::Raw(json!({ "type": "web_search_20250305", "name": "web_search" })),
    ];
    options.mcp_servers = vec![
        McpServerRef::named("github"),
        McpServerRef {
            name: "inline".to_owned(),
            url: Some("https://mcp.example.com/mcp".to_owned()),
            authorization: Some("tok".to_owned()),
            tool_configuration: Some(json!({ "enabled": true })),
        },
    ];
    options.provider_options.insert(
        "anthropic".to_owned(),
        json!({ "tool_choice": { "type": "auto" }, "top_p": 0.9 }),
    );
    assert_json_roundtrip(&options);
    assert_json_roundtrip(&ConversationOptions::new());
}

#[test]
fn mcp_server_ref_debug_redacts_the_authorization_token() {
    // The bearer token must never appear in a log line, panic message, or test
    // failure — Debug redacts it while still showing the other fields.
    let server = McpServerRef {
        name: "github".to_owned(),
        url: Some("https://mcp.example.com/mcp".to_owned()),
        authorization: Some("super-secret-token".to_owned()),
        tool_configuration: None,
    };
    let debug = format!("{server:?}");
    assert!(
        !debug.contains("super-secret-token"),
        "token must be redacted"
    );
    assert!(debug.contains("<redacted>"));
    assert!(debug.contains("github"));

    // A named reference carries no token and shows None.
    let named = format!("{:?}", McpServerRef::named("github"));
    assert!(named.contains("authorization: None"));
}

#[test]
fn server_tool_variants_roundtrip_and_are_kind_tagged() {
    let tools = vec![
        ServerTool::WebSearch {
            max_uses: None,
            allowed_domains: None,
            blocked_domains: None,
        },
        ServerTool::WebSearch {
            max_uses: Some(3),
            allowed_domains: None,
            blocked_domains: Some(vec!["spam.example".into()]),
        },
        ServerTool::CodeExecution {},
        ServerTool::Raw(json!({ "type": "code_execution_20250522", "name": "code_execution" })),
    ];
    for tool in &tools {
        assert_json_roundtrip(tool);
    }

    // The discriminator is a Loom-owned `kind`, distinct from any provider-native
    // `type` a Raw payload carries.
    assert_eq!(
        serde_json::to_value(ServerTool::CodeExecution {}).unwrap(),
        json!({ "kind": "code_execution" })
    );
    let raw = serde_json::to_value(ServerTool::Raw(
        json!({ "type": "web_search_20250305", "name": "web_search" }),
    ))
    .unwrap();
    assert_eq!(raw["kind"], json!("raw"));
    assert_eq!(raw["type"], json!("web_search_20250305"));
}

#[test]
fn cache_hints_roundtrip_and_default_omits_fields() {
    assert_json_roundtrip(&CacheHint::ephemeral());
    assert_json_roundtrip(&CacheHint::with_ttl(CacheTtl::FiveMinutes));
    assert_json_roundtrip(&CacheHint::with_ttl(CacheTtl::OneHour));
    assert_json_roundtrip(&CacheNegotiation::SoftIgnore);
    assert_json_roundtrip(&CacheNegotiation::HardFail);

    // A default (no-TTL) hint serializes as an empty object; TTLs use
    // provider-agnostic names.
    assert_eq!(
        serde_json::to_value(CacheHint::ephemeral()).unwrap(),
        json!({})
    );
    assert_eq!(
        serde_json::to_value(CacheHint::with_ttl(CacheTtl::OneHour)).unwrap(),
        json!({ "ttl": "one_hour" })
    );

    // Default options omit the caching knobs entirely (stable prefix bytes).
    let value = serde_json::to_value(ConversationOptions::new()).unwrap();
    assert!(value.get("auto_cache").is_none());
    assert!(value.get("cache_negotiation").is_none());
}

#[test]
fn conversation_roundtrips() {
    let conversation = Conversation {
        id: Uuid::from_u128(1),
        tenant_id: Uuid::from_u128(2),
        binding: ProviderBinding::new("anthropic", "claude-opus-4-8"),
        system: Some("You are helpful.".into()),
        system_cache: Some(CacheHint::ephemeral()),
        messages: vec![
            Message::user("hi"),
            Message::new(
                Role::Assistant,
                vec![ContentPart::ServerToolResult {
                    tool_use_id: "srvtoolu_1".into(),
                    content: json!([{ "type": "web_search_result" }]),
                }],
            ),
        ],
        metadata: json!({ "trace_id": "abc" }),
        created_at: Utc.with_ymd_and_hms(2026, 7, 3, 12, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2026, 7, 3, 12, 5, 0).unwrap(),
    };
    assert_json_roundtrip(&conversation);
}
