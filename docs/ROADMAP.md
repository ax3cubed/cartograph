# Roadmap

This is the plan for taking Cartograph from a prototype on demo data to a tool that maps real workspaces. It is split into seven stages, listed in the order they are planned. Each stage has a "done when" line that says what has to be true before it counts as finished.

## Current status

Cartograph is an early prototype.

- **Released:** v0.1.0 (2026-11-10). The interactive map on a bundled demo workspace called `northwind`, hand-authored in `lib/workspace.ts`. See the [changelog](../CHANGELOG.md) for the feature list.
- **Released:** v0.2.0 (2026-11-10). Bring-your-own-model answers for "Ask the map". This is a small first step towards stage 6.
- **Not built yet:** the indexer. Real repositories cannot be loaded. No stage below has been completed.

| Stage | Status |
| --- | --- |
| 1. Core indexer | Not started |
| 2. Breadth | Not started |
| 3. Real-data UI | Not started |
| 4. Cross-service links | Not started |
| 5. Symbols and calls | Not started |
| 6. AI | Not started. A first step shipped in v0.2.0 |
| 7. Hardening | Not started |

## Stage 1: Core indexer

A Rust CLI that:

- discovers the repositories in a workspace;
- walks first-parent git history once;
- parses each distinct file version once, with the extracted facts cached by git blob id;
- replays history to open and close link intervals, so every link has the commit that added it and the commit that removed it;
- writes one SQLite index file.

TypeScript and JavaScript imports come first.

**Done when:** one run of the CLI over a real TypeScript or JavaScript workspace produces a single SQLite index file in which every import link carries the commit that opened it and, where it was later removed, the commit that closed it.

An early throwaway experiment tried this design on a real TypeScript repository with 35,000 commits. It processed the first-parent history (9,498 commits, 57,060 distinct file versions) in 4.4 seconds, with 719× fewer parses than re-indexing every commit. The experiment used a simplified regex extractor, so it shows that the approach scales. It says nothing about the final speed. The method, the numbers and the script are in [docs/research](research/README.md).

## Stage 2: Breadth

- Go, Python and PHP.
- Rename lineage, so a file that moves keeps its identity.
- One merged timeline across several repositories.
- Incremental updates, and correct handling of rewritten history.

**Done when:** a workspace that mixes these languages across several repositories indexes onto one timeline, a moved file keeps one identity across the move, new commits are picked up without a full re-index, and a rewritten history still yields a correct index.

## Stage 3: Real-data UI

- The app reads the index in place of the demo data.
- Level of detail: services, then modules, then files.
- Stored layout.
- WebGL rendering for large maps.

**Done when:** the app opens an index produced by the indexer, a large workspace opens at service level and expands to modules and files on demand, the layout is the same the next time the map is opened, and large maps render through WebGL.

## Stage 4: Cross-service links

- HTTP routes, events and tables matched across services.
- Contracts (OpenAPI, AsyncAPI, protobuf) ingested first.
- Evidence and a confidence level on every link.
- An override file for corrections.

**Done when:** HTTP, event and table links between services appear in the map, each with its evidence and a confidence level, contracts are used wherever they exist, and a wrong or missing link can be corrected in the override file.

## Stage 5: Symbols and calls

- Nodes below file level: symbols, and the calls between them.
- Optional SCIP import and export.

**Done when:** the index and the map include symbols and call links, and SCIP data can be imported and exported for those who want it.

## Stage 6: AI

- A provider registry.
- A tool-calling Ask agent with checked citations.
- Model-written summaries, off by default, with a local model suggested.
- An MCP server.

**Done when:** providers are configured through the registry, the Ask agent answers by calling graph tools and its citations are checked before an answer is shown, summaries can be switched on and stay off by default, and other tools can query the map through the MCP server.

## Stage 7: Hardening

- Fuzzing.
- Crash and fault injection.
- Performance gates in CI.
- Large-repository targets.

**Done when:** the indexer is fuzzed, crash and fault injection tests pass, CI fails a change that misses a performance gate, and the large-repository targets are met.

## Proposing changes to the roadmap

The roadmap is open to change. To propose one, [open an issue](https://github.com/ax3cubed/cartograph/issues/new/choose) with the feature request form and say:

- which stage it affects, or that it is a new stage;
- what you would add, remove or reorder;
- why, and what it would cost or delay.

Discussion happens on the issue. Once a change is agreed, this file is updated in a pull request that links back to it.

To work on a stage, see [CONTRIBUTING.md](../CONTRIBUTING.md).
