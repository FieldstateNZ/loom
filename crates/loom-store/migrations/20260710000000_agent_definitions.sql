-- OASP Wave 2: AgentDefinition + AgentDefinitionVersion.
--
-- A provider-neutral, named, versioned agent. Two version pointers:
--   draft_version     — advances on every edit
--   published_version — advanced ONLY by `publish` (monotonic; no unpublish)
--
-- Every version is an immutable content snapshot in agent_definition_versions,
-- recorded at mint time whether or not it is ever published, so "which version
-- produced which turns" resolves to content, not just an integer. This is the
-- hook version pinning attaches to (a session pins an agent version).

CREATE TABLE agent_definitions (
    id                UUID PRIMARY KEY,
    tenant_id         UUID NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    draft_version     INTEGER NOT NULL DEFAULT 1,
    published_version INTEGER,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_agent_definitions_tenant ON agent_definitions (tenant_id);

-- Immutable per-version content snapshots.
CREATE TABLE agent_definition_versions (
    agent_definition_id UUID NOT NULL REFERENCES agent_definitions (id) ON DELETE CASCADE,
    version             INTEGER NOT NULL,
    content             JSONB NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_definition_id, version)
);
