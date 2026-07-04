//! Content parts — the provider-faithful building blocks of a [`Message`].
//!
//! [`Message`]: crate::Message

mod citation;
mod content_part;
mod media_source;

pub use citation::Citation;
pub use content_part::ContentPart;
pub use media_source::MediaSource;
