//! Input types for agent-definition writes.

use loom_core::AgentContent;

/// Input to create a new [`AgentDefinition`](loom_core::AgentDefinition): a name
/// and the content of its first (draft) version.
#[derive(Clone, Debug)]
pub struct NewAgentDefinition {
    /// The definition's human-readable name.
    pub name: String,
    /// The content of draft version 1.
    pub content: AgentContent,
}

impl NewAgentDefinition {
    /// Constructs a new-definition input from a name and its first version's
    /// content.
    #[must_use]
    pub fn new(name: impl Into<String>, content: AgentContent) -> Self {
        Self {
            name: name.into(),
            content,
        }
    }
}
