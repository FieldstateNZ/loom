# Loom and OASP — the design thesis

*An LLM gateway that gives you a managed agent over any provider — write against
one contract, not one API per provider.*

This is the "why Loom exists" document. It states what Loom is, how it relates to
the OASP standard, and the one requirement everything else follows from. For the
concrete inventory of what is built versus what is still needed to hold this
contract, see [`../oasp-gap-analysis.md`](../oasp-gap-analysis.md).

## What Loom is

Loom is an LLM gateway. You build your application against **one consistent
contract** — a *managed agent*, with sessions, memory and tools — and Loom
guarantees that contract over any provider. Where a provider offers managed-agent
features natively, Loom adapts to them; where it doesn't, **Loom supplies them
itself**. Rather than collapsing every provider down to a lowest-common-
denominator completion API, Loom levels every provider up to the same
managed-agent contract.

That contract is **OASP** (the Open Agent Session Protocol): a vendor-neutral
standard, shaped on the Anthropic Managed Agents API, written as the first step
toward building Loom. Loom is becoming its reference implementation — but the
product is the point; the standard is the contract it stands on.

## The core principle: write against a contract, not a provider

Developers should write against a contract, not a provider. One application
shouldn't need an `anthropic` endpoint and an `openai` endpoint — there is one
contract, and it is the same managed agent whether or not the provider underneath
has any concept of one.

An early cut of Loom did the opposite: a route per provider, each mirroring that
provider's own API, so a single application ended up with a different endpoint
depending on the provider it talked to. That is exactly the pattern the standard
exists to prevent. The contract comes first; providers are adapted to meet it.

## The requirement: a managed agent over any provider

A gateway that normalises every provider down to a shared completion API gives
you the *intersection* of what every provider has in common — the lowest common
denominator. That is a perfectly good fit when the requirement is broad,
provider-agnostic reach, and it is not a criticism of that approach.

Loom's requirement is the opposite. Loom's job is to give you a **managed agent** —
the same one — whatever the provider underneath happens to support:

- If a provider has no concept of **memory**, Loom is the memory.
- If a provider offers only **completion** and has no notion of a **session**,
  Loom provides the session.
- Wherever a provider lacks a managed-agent capability the contract promises,
  Loom fills it in.

The contract is a guarantee held at the *top*, with Loom backfilling whatever a
provider lacks — not a floor set by whatever every provider happens to share.
Where a provider *does* supply a capability natively, Loom uses it rather than
reinventing it.

This is also why Loom's forward work is what it is: a durable Conversation and
Session model, a memory/resource layer, a normalised event stream, an
identity/audit plane. Those are not "conformance for its own sake" — they are the
pieces Loom must own so it can be the managed agent over a provider that only does
completion.

## The on-ramp: change the endpoint, migrate as you go

Because the contract is shaped on the Anthropic Managed Agents API, an
application already using that API can adopt Loom by changing one thing: **the
endpoint**. Loom keeps an Anthropic-compatible entry point, so you point your
existing Anthropic-managed-agents client at the Loom server and it works on day
one — same code, now running through Loom, with the gateway's provider adapters,
virtual keys, budgets and usage underneath it.

That Anthropic-compatible route is an *on-ramp*, not the surface you develop
against long-term. From there you migrate onto the contract as you need it. So
the developer experience is:

> **Change the endpoint. Then migrate the API surface as required.**

Getting onto Loom is an endpoint swap, not a rewrite.

## The relationship, stated plainly

- **Loom** is the product: an LLM gateway / proxy that guarantees a managed agent
  over any provider.
- **OASP** is the contract Loom implements — written first, because the standard
  is the foundation the product is built on.
- **Adapters** are how each provider is made to meet the contract — using native
  managed-agent features where they exist, and letting Loom supply what a provider
  lacks.
- **Loom is OASP's reference implementation** — true, but downstream of the
  product, not above it. The standard serves the product, not the other way
  around.

The standard itself — spec prose, Zod-first schemas, generated JSON Schema /
OpenAPI, and the v0 concept draft — lives in
[oasp-dev/oasp-standard](https://github.com/oasp-dev/oasp-standard) (Apache-2.0).
