# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the version is 0.x, a minor release may contain breaking changes.

## [Unreleased]

### Added

- Bring-your-own-model answers for "Ask the map". The graph engine finds the evidence and a model of your choice words the answer, streamed into the same answer card with clickable file and commit references.
- Model picker in the composer: model per row, reasoning effort per column, and an ULTRA lever that sends the whole map as context.
- Model settings dialog: provider preset or custom base URL, API key, model list loaded from the provider, up to six models in the picker.
- `POST /api/ai`: one provider-neutral route for any OpenAI-compatible endpoint. It streams chat completions as plain text, lists a provider's models, and retries without `reasoning_effort` when a provider rejects it.
- Server settings `CARTOGRAPH_AI_BASE_URL`, `CARTOGRAPH_AI_API_KEY` and `CARTOGRAPH_ALLOW_CLIENT_PROVIDER`.
- Checks for the SSE parser, prompt assembly and evidence linking in `pnpm check`.

### Changed

- `Answer` has an optional `by` field naming the model that worded it.

## [0.1.0] - 2026-11-10

First release. An interactive map running on a bundled demo workspace called `northwind`, hand-authored in `lib/workspace.ts`. There is no indexer yet, so real repositories cannot be loaded.

### Added

- Workspace model as an ontology (workspace → service → module → file) with a plain description on every node.
- Typed relationships (imports, calls, HTTP, events, database). Each one records the commit that added it and the commit that removed it.
- Several services in one map, with cross-service links drawn and counted.
- Network, Adaptive and Ontology layouts, with a morph between them.
- Focus view showing "Depends on" and "Used by" for the selected node.
- Time scrubber that steps commit by commit. Added links flash lime and removed links ghost coral.
- Side panel with links, cross-service links, churn, fan-in and fan-out, a trend chart, relationship changes and strongest links.
- Workspace tree, search, pins, a link-type filter and a minimum "used by" filter.
- "Ask the map": a deterministic graph engine that answers questions about the impact of a change, the path between two things, what changed since a date, and what something does. Answers cite the files and commits used as clickable evidence.
- `pnpm check`: assertions over the demo data and the Ask engine.

[Unreleased]: https://github.com/ax3cubed/cartograph/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ax3cubed/cartograph/releases/tag/v0.1.0
