//! Fixture-based tests for Anthropic SSE streaming: event-sequence mapping,
//! verbatim `raw` preservation, streamed/non-streamed message equivalence, and
//! partial-turn recovery on mid-stream error and early disconnect.
//!
//! The transcript fixtures under `tests/fixtures/*.sse` are raw `event:/data:`
//! Server-Sent Events text. Sequence tests drive [`SseAccumulator`] directly;
//! the end-to-end tests drive the full [`Provider::stream`] path against a
//! `wiremock` server that serves the transcript as an SSE response body.
//!
//! Split by feature: [`text`], [`tool_use`], [`thinking`], [`citations`],
//! [`web_search`], and [`mcp`] each cover one content-block kind's streaming
//! and reassembly; [`partial_turns`] covers recovery from a mid-stream error
//! or an early disconnect; [`end_to_end`] drives the full HTTP + SSE path.

#[path = "streaming/citations.rs"]
mod citations;
#[path = "streaming/end_to_end.rs"]
mod end_to_end;
#[path = "streaming/mcp.rs"]
mod mcp;
#[path = "streaming/partial_turns.rs"]
mod partial_turns;
#[path = "streaming/support.rs"]
mod support;
#[path = "streaming/text.rs"]
mod text;
#[path = "streaming/thinking.rs"]
mod thinking;
#[path = "streaming/tool_use.rs"]
mod tool_use;
#[path = "streaming/web_search.rs"]
mod web_search;
