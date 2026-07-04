//! Capability model and capability negotiation.
//!
//! Every provider declares, per model, which [`Capability`] values it supports.
//! Before a request is dispatched, Loom computes the set of capabilities the
//! request actually exercises and checks them against the bound model. If the
//! model does not support a required capability the request fails **fast** with
//! [`ProviderError::CapabilityUnsupported`] — Loom never silently degrades a
//! request by dropping a feature the caller asked for.
//!
//! [`ProviderError::CapabilityUnsupported`]: crate::ProviderError::CapabilityUnsupported

#[allow(clippy::module_inception)]
mod capability;
mod model_descriptor;
mod negotiation;
mod provider_descriptor;

pub use capability::Capability;
pub use model_descriptor::ModelDescriptor;
pub use negotiation::{ensure_supported, required_capabilities};
pub use provider_descriptor::ProviderDescriptor;
