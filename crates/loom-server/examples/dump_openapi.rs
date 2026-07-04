//! Dumps the gateway's OpenAPI document to stdout.
//!
//! `ApiDoc::openapi()` is a pure function over the `#[utoipa::path]` /
//! `#[derive(ToSchema)]` annotations baked into the binary at compile time —
//! no database or running server is needed to produce it. This makes the
//! dump deterministic and reusable: it is how `clients/typescript/openapi.json`
//! is regenerated, and it is the seed for the CI drift-guard that will compare
//! a fresh dump against the committed snapshot (tracked separately).
//!
//! ```sh
//! cargo run -p loom-server --example dump_openapi > clients/typescript/openapi.json
//! ```

use loom_server::ApiDoc;
use utoipa::OpenApi;

fn main() {
    println!(
        "{}",
        ApiDoc::openapi()
            .to_pretty_json()
            .expect("ApiDoc always serializes to JSON")
    );
}
