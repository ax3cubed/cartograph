# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Next.js (App Router), TypeScript. Chosen by the user. First version runs on a mock multi-service workspace; real indexing (file scan + git log) plugs in later behind the same data shape (`lib/workspace.ts`).

## Purpose

A codebase map that reduces how deep a person has to read to understand a system. The workspace is modelled as an ontology (workspace → service → module → file), every node carries a plain description of what it does, and the relationships between nodes (imports, calls, HTTP, events, DB) are first-class.

Relationships are tied to git: each edge records the commit that added or removed it, so the map can be scrubbed through time and every node shows which commits changed its links.

Works across code workspaces: multiple repos/services (microservices, multi-service apps) sit in one map, and cross-service links are drawn and counted.

## Users

Both equally:

- **Developer new to a codebase** (onboarding): "what is this, what does it touch, why did it change?"
- **Lead / architect**: reviewing coupling across services and drift in dependencies over time.

## Terminology

- Node: a service, module, file or symbol.
- Link / relationship: a typed edge between nodes.
- Fan-in: active links pointing at a node (blast radius). Fan-out: links leaving it.
- Cross-service: links whose ends live in different services.
- Churn: links added or removed in the 90 days before the selected date.

## Open decisions

- Product name: "Cartograph" is a placeholder.
- How real indexing runs (CLI that emits JSON vs Next route handler reading local repos).
