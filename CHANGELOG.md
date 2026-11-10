# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the version is 0.x, a minor release may contain breaking changes.

## [Unreleased]

### Added

- Research note on the indexing method, content-addressed fact replay, with the experiment script that produced its numbers (`docs/research`).

### Changed

- The landing page (`site/`) now describes the finished product and has a research section in place of the roadmap. The README and `docs/ROADMAP.md` remain the record of what is built.

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
