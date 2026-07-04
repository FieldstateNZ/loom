//! The boxed streaming event type, [`TurnEventStream`].

use futures::stream::BoxStream;

use crate::error::ProviderError;
use crate::event::TurnEvent;

/// A stream of streaming turn events, boxed so the trait stays object-safe.
///
/// Each item is a `Result` so a mid-stream provider failure can be surfaced
/// without tearing down the whole stream.
pub type TurnEventStream = BoxStream<'static, Result<TurnEvent, ProviderError>>;
