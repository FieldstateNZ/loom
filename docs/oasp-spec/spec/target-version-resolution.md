# Target-version resolution

Every [`migrate`](./interactions.md#migrate-session-upgrade) call needs
a target `AgentVersionRef` — the version to mint a new `Session`
against. This document specifies, unambiguously, how that target is
resolved for every session context.

## The table (normative)

| Session context | Resolves to | Source field on `AgentDefinition` |
|---|---|---|
| **Builder** — an interactive session used to edit/preview an `AgentDefinition` | Latest | `draftVersion` |
| **Test-session** — an ephemeral session spun up to validate a draft before publish | Latest draft | `draftVersion` |
| **Real conversation** — a persisted `Conversation`, where the `AgentDefinition`'s `publishedVersion` is non-null | The current published version | `publishedVersion` |
| **Real conversation**, but the `AgentDefinition`'s `publishedVersion` is `null` (never published) | Leave in place — no target resolved | — |

A conformant resolver **MUST** produce exactly one of two outcomes for
any (session context, `AgentDefinition`) pair:

1. A specific `AgentVersionRef { agentDefinitionId, version }` (see
   [`agentVersionRefSchema`](../../packages/schemas/src/common/agent-version-ref.ts))
   to migrate toward, or
2. **No target** ("leave in place") — `migrate` **MUST** then be a
   successful no-op, never an error and never a partial attempt (see
   [`interactions.md` § Preconditions](./interactions.md#preconditions)).

## Resolving to content, not only a pointer

Outcome 1 above resolves to an `AgentVersionRef` — a bare, comparable
`{ agentDefinitionId, version }` pointer. That pointer alone answers
*which* version to migrate toward; it does not answer *what that
version's content is*, because `AgentDefinition` stores only its
CURRENT, still-mutable `instructions`/`provider`/`model`/`tools`/`guardrails`,
not a history of every version's content
(issue [#10](https://github.com/FieldstateNZ/oasp-standard/issues/10)).
A conformant server **MUST** therefore back every `AgentVersionRef`
this table can ever produce with an immutable content snapshot — see
[`AgentDefinitionVersion`](../../packages/schemas/src/resources/agent-definition-version.ts) —
recorded under that exact `{ agentDefinitionId, version }` key at the
moment the version number is minted, whether or not it is ever
published. `migrate`'s Stage 1 `vaultIds` re-resolution and `drain`'s
pre-dispatch tool-call authorization (issue #9) both resolve against
THAT snapshot, not against whatever the live `AgentDefinition` happens
to hold at the moment they run — otherwise a target version this table
resolves to today could silently pick up a DIFFERENT version's content
by the time a later draft edit changes the live Definition, defeating
the whole point of resolving to a specific, comparable version in the
first place.

Resolution **MUST NOT** produce a target version that does not exist.
In particular, resolving a real conversation whose `AgentDefinition`
has a `null` `publishedVersion` **MUST** fall through to "leave in
place." A resolver **MUST NOT** substitute `draftVersion` for a real
conversation merely because `publishedVersion` happens to be unset —
that would pin live, real usage to unpublished, still-changing
content, which is exactly what the draft/published split exists to
prevent (see
[`agentDefinitionSchema`](../../packages/schemas/src/resources/agent-definition.ts):
"a never-published Definition is left in place rather than resolved to
a version that doesn't exist").

**Builder** and **test-session** both resolve to `draftVersion`
because `draftVersion` is, by definition, the only value that is
always "latest": every edit to the Definition advances it (see
[`agentDefinitionSchema`](../../packages/schemas/src/resources/agent-definition.ts)),
so it necessarily leads `publishedVersion` whenever the two differ,
and equals it whenever they don't.

> **Note (a resolution call, flagged for sign-off):** the S0 resource
> schemas — [`Conversation`](../../packages/schemas/src/resources/conversation.ts),
> [`Session`](../../packages/schemas/src/resources/session.ts) — carry
> no explicit `kind`/`purpose` discriminant distinguishing builder /
> test-session / real conversation. "Session context" in the table
> above is therefore a caller-supplied or server-tracked
> classification at the point a `Session` is created or a `migrate`
> sweep runs, not a persisted resource field this document mandates.
> How a conformant server tracks that classification is
> implementation-defined; this table only pins down what each
> classification resolves *to*, matching the concept draft's
> provenance note (`determineTargetVersion` in LucidBrain) and the
> [Interactions (v0) table](../oasp-v0-concept.md#interactions-v0).
> This goes a little beyond what the concept draft spells out and is
> called out again in this slice's handback to the dev lead.

## Relationship to `publish`

Because [`publish`](./interactions.md#publish) only ever advances
`publishedVersion` and never touches an existing `Conversation`,
target-version resolution is what a subsequent, separate
[`migrate`](./interactions.md#migrate-session-upgrade) call uses to
catch a **real conversation** up to that new `publishedVersion`.
Resolution and `publish` together are what make "snap forward, don't
disturb" and "move me forward, on request" two independently
triggerable operations rather than one implicit cascade.

## Relationship to `createConversation`

[`createConversation`](./interactions.md#createconversation) pins a
brand-new Conversation's first Session using the same source field the
table's **real conversation** row names: `publishedVersion`. This is
not a fifth row needing its own resolution logic — a Conversation being
minted right now is, for this purpose, exactly the **real conversation**
case; there is no separate rule for "a real conversation that doesn't
exist yet."

Where `createConversation` diverges from this table is the
never-published outcome. The table's **real conversation, but
`publishedVersion` is `null`** row resolves to "leave in place — no
target resolved," which [`migrate`](./interactions.md#migrate-session-upgrade)'s
[Preconditions](./interactions.md#preconditions) turn into a successful
no-op: an *existing* Conversation is left exactly as it was. That
outcome presupposes a Conversation to leave in place. `createConversation`
has none yet — there is no prior state to preserve — so "leave in
place" has no meaning at creation time. A conformant server **MUST**
instead **reject** `createConversation` outright when the target
`AgentDefinition`'s `publishedVersion` is `null`, rather than either
(a) inventing a new "leave in place" behaviour that has nothing to
apply to, or (b) falling back to pinning the new Conversation to
`draftVersion`. That fallback is explicitly the wrong shape: it would
pin live, real usage to unpublished, still-changing content, the exact
outcome [the table's normative `MUST NOT`](#the-table-normative) above
already forbids for `migrate` — there is no principled reason
`createConversation`'s *initial* pin should be held to a looser
standard than every *subsequent* `migrate` call is.

This rejection has a durable consequence for `migrate` itself: since
`publishedVersion` is monotonically non-decreasing and v0 has no
"unpublish" ([`interactions.md` § `publish`](./interactions.md#publish)),
a server that rejects `createConversation` correctly can never produce
a real Conversation that later reaches `migrate`'s "leave in place"
precondition for the never-published reason — that row of the table
remains reachable only as a defensive/theoretical case for a server
auditing its own invariants, not as a path any conformant
`createConversation` implementation can route a live Conversation
through.
