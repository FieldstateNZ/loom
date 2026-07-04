//! Fixture-based tests for Anthropic request and response translation.
//!
//! Split by feature: [`request`] (Conversation → native body), [`response`]
//! (native response → Message), [`cache`] (prompt-cache placement and
//! round-tripping), [`server_tools`] (provider-executed tools and their beta
//! tokens), and [`mcp`] (the MCP connector).

#[path = "translate/cache.rs"]
mod cache;
#[path = "translate/mcp.rs"]
mod mcp;
#[path = "translate/request.rs"]
mod request;
#[path = "translate/response.rs"]
mod response;
#[path = "translate/server_tools.rs"]
mod server_tools;
#[path = "translate/support.rs"]
mod support;
