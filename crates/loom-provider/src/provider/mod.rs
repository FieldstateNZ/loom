//! The [`Provider`] plugin trait and its streaming envelope type.

mod provider_trait;
mod turn_event_stream;

pub use provider_trait::Provider;
pub use turn_event_stream::TurnEventStream;
