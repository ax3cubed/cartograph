# Phase 1: the indexer vertical slice (IDX-03 to IDX-07)

Measured results for the first slice of phase 1 in [the implementation plan](../architecture.html): the store, the gix walk, the extractors and fact cache, the replay engine and `cartograph index`. Each check can be re-run from the ignored tests named below. Method as in [phase0.md](phase0.md).

**Status.** Run on 22 Nov 2026. Same machine as phase 0: Apple M4 (10 cores), 24 GB, macOS 26, rustc 1.100.0. **Not a quiet machine**: the load average was 10 to 11 during the timed runs. Counts and equalities are exact; times are upper bounds.

**Fixture.** The phase 0 bare clone of remotion, read at `2b1b0c9` (9,498 first-parent commits) unless stated. Every run writes under a temporary `CARTOGRAPH_HOME`, never `~/.cartograph`.

## Gate

| Gate line | Measured | Result |
| --- | --- | --- |
| `cartograph index <repo>` writes one SQLite index; every import link carries the commit that opened it and, where removed, the commit that closed it | remotion at `2b1b0c9`: 36,037 links, 36,628 intervals (24,540 open at head), one 83 MB file | **Pass** |
| The tree equals `ls-tree` | The walk yields 9,498 steps; the tree rebuilt from them equals `git ls-tree -r` (git's own output) at 24 sampled commits, the last one head | **Pass** |
| Replayed links equal a from-scratch index at 24 sampled commits | Equal at 24 of 24. The written intervals reproduce the link set at all 24, and the links queried from the index file as of each sampled sha equal the from-scratch index | **Pass** |
| Resolutions of the order of 60,039, not 37.8 million | **420,590** | **Partial**: 90× below 37.8 million, 7× above 60,039. See "Resolutions" below |
| Replay equals direct indexing, as a property over generated histories | `replay_equals_direct_indexing`: 96 cases a run, up to 30 steps of source, package.json and tsconfig edits and deletes; checked at every step, plus intervals and the ablation | **Pass**. A planted bug (a missed `node_modules` link change) fails it in under a second |
| Proposed: cold under 60 s, warm under 10 s, peak memory under 1 GB | `2b1b0c9`: cold 45.5 s, warm 38.7 s. HEAD (11,453 commits): cold 64 s, warm 55 s, peak RSS 1.31 GB | **Cold passes at 2b1b0c9 only; warm and memory fail** |

## The research's six checks, in Rust (IDX-06)

From `crates/cartograph-index/tests/remotion.rs` (`CARTOGRAPH_REMOTION=<clone> cargo test --release -p cartograph-index --test remotion -- --ignored --nocapture`):

| Check | Result |
| --- | --- |
| 1. Tree equals git at sampled commits | Equal at 24 (paths and blob ids, tracked files), and at head |
| 2. Replayed links equal a from-scratch index | Equal at 24 of 24. From scratch takes 41 ms a commit |
| 3. Written intervals reproduce the link set at each sampled step | Equal at 24 of 24 |
| 4. Replay without the reverse index writes identical intervals | Identical over the first 400 steps: 33,075 resolutions without the reverse index, 2,318 with it. The full ablation would take about an hour at the measured cost per resolution, so it is bounded by `CARTOGRAPH_ABLATION=<steps>`; the property test runs it in full on every generated history |
| 5. An incremental run over the last 100 commits ends in the same state | Same intervals and open links. 1.3 s, including the walk of the 100 commits |
| 6. Resolutions | 420,590 (most in one step: 10,669) |

## Resolutions

The research's 60,039 came from a candidate-list resolver with no tsconfig and no `exports`. This slice resolves with oxc_resolver, so a resolution also depends on the importer's tsconfig (which TS picks by `include`, `exclude`, `files` and `references`) and on package manifests. Each resolution waits on every path the resolver asked about, logged by the virtual file system, so a package's `tsconfig.json` is a hot key: every reference in that package waits on it.

The large steps are almost all one package's `tsconfig.json` or `package.json` changing a resolution field: `packages/renderer/tsconfig.json` alone re-resolves 2,000 to 10,000 references at each of at least 8 commits. Version bumps re-resolve nothing, because manifests are served to the resolver as their resolution fields only (the phase 0 amendment), and `compilerOptions` is narrowed to the options oxc_resolver 11.24 reads (`allowJs`, `baseUrl`, `declarationDir`, `outDir`, `paths`, `rootDirs`).

## Extraction (IDX-05)

From `crates/cartograph-index/tests/extract_remotion.rs`:

| | |
| --- | --- |
| Distinct (blob, extractor kind) pairs | 103,810 (57,072 TS/JS, the rest package.json and tsconfig) |
| Cold, 8 workers | 6.5 to 7.6 s |
| Warm | 0.37 to 0.42 s, 100% from the cache, nothing parsed |
| Skipped | 100 over 1 MB, 1 binary, 1 generated, 0 timed out |
| Distinct specifiers per blob | regex 489,481; parser 486,457 |

**Superset check: pass.** The parser finds nothing the regex lacks. Every regex-only specifier is explained:

| Class | Count | Example |
| --- | --- | --- |
| Inside a template string (code samples in docs) | 1,582 | `"@remotion/light-leaks"` |
| Inside a string | 963 | `"@remotion/cli/config"` |
| Inside a comment | 460 | `"./weird-css.css"` |
| Inside a regular expression literal | 10 | `"@remotion\/effects\/tint"` |
| In a skipped generated file | 9 | `"@docusaurus/ErrorBoundary"` |
| Unexplained | 0 | |

## Walk (IDX-04)

`crates/cartograph-git/tests/remotion.rs`: 9,498 steps and 138,141 regular-file change events (no filter), 1.6 s with author and summary read per commit; equal to `git ls-tree -r` at 24 sampled commits. With the tracked-path filter, 108,444 events.

## The index file (IDX-07)

| | `2b1b0c9` | HEAD (`970849f`) |
| --- | --- | --- |
| Commits | 9,498 | 11,453 |
| Links, intervals | 36,037, 36,628 | 47,041, 47,727 |
| Resolutions | 420,590 | 606,655 |
| File size | 83 MB | 99 MB, of which the `fact` table is 79 MB |
| Cold: walk, extract, replay and write | 1.6 s, 6.5 s, 37.4 s (45.5 s) | 1.9 s, 10.1 s, 51.0 s (64 s) |
| Warm | 38.7 s | 55 s |
| Peak RSS | | 1.31 GB cold, 1.23 GB warm |

## What this changes

1. **Replay cost is the cost of a cold resolver.** Each importer's batch starts with an empty oxc cache so the logged paths are a complete key set; that costs about 85 µs a resolution, and replay is now 80% of a cold build and nearly all of a warm one. The 10 s warm mark needs one of: memoised resolutions across runs (keyed on the logged paths), or oxc reporting the manifests and tsconfigs it reads from its cache, which would let the cache stay warm.
2. **The resolution mark should be restated** for a resolver that models tsconfig and `exports`: of the order of 400,000 on remotion, with tsconfig edits as the hot keys. Finer keys for tsconfig ownership (which project claims a file) would cut most of it.
3. **The `fact` table dominates the index** (79 of 99 MB), mostly manifest facts repeated across 46,000 package.json versions. The index could hold facts for import blobs only and leave manifests in the shared cache.
4. **Peak memory** (1.3 GB) comes from holding every step and every blob's facts at once; the pipeline in IDX-07's efficiency note (bounded channel, extraction ahead of replay) is now needed, not optional.
5. **serde_json's `preserve_order`** is switched on for the whole workspace by oxc_resolver. Anything that renders JSON for comparison or golden files must not rely on map order; the API contract now sorts its keys, and manifest facts keep file order on purpose because `exports` condition order is semantic.
