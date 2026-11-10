# Content-addressed fact replay

A research note on how Cartograph's indexer finds, for every link in a codebase, the commit that added it and the commit that removed it, without re-indexing the code at every commit.

**Status.** This is a design plus one measured experiment. The experiment script is [`spike.mjs`](spike.mjs) in this folder and runs on any git repository. The production indexer is stage 1 of the [roadmap](../ROADMAP.md) and is not built yet. Nothing here has been peer reviewed.

## The problem

Cartograph draws a link between two files (an import, a call, an HTTP request, an event, a database read) and says when that link appeared and when it went away. That needs the dependency graph at every point in history, across one repository or several.

The direct way is to check out each commit and index the tree. That costs one parse per file per commit:

```
naive work = Σ over commits of (source files in the tree at that commit)
```

On the repository measured below that sum is 41,016,305 parses. Almost all of them re-read a file that did not change.

## The idea

Split what the indexer knows into two kinds.

- **Facts** are what a file says: it imports `"./db"`, calls `charge()`, serves `POST /invoices`. A fact depends only on the file's bytes.
- **Edges** are what those facts mean: which file `"./db"` is. An edge depends on the rest of the tree at that commit.

Git already names a file's bytes with a blob id. So facts are extracted once per blob id and cached for good, and only resolution is redone as history moves, and only for the references a commit can affect.

## The algorithm

1. **Walk.** One streaming process per repository:
   `git log --first-parent --reverse --raw --no-abbrev --no-renames`.
   Each commit yields `(path, old blob id, new blob id)` for what it changed. Each commit gets an ordinal on its first-parent chain and a corrected date, `max(own date, parent's corrected date + 1)`, because committer dates are not monotonic.

2. **Extract.** Parse each blob id that is not in the fact cache, once, in parallel. Store `facts(blob id, extractor version, language)`: definitions and exports, unresolved references, and link descriptors (route served, URL called, topic published or consumed, table read or written). This is the only step that reads file contents.

3. **Merge timelines.** K-way merge the repositories' commit lists on `(corrected date, repository, ordinal)` into one workspace ordinal `t`. Intervals are keyed by `t`. The commit SHA is an attribute.

4. **Replay.** Walk `t` in order holding three structures: the tree (path → blob id), a definition index (candidate key → entity), and a reverse index (candidate key → references waiting on it).
   - For each changed path, diff its old and new fact sets.
   - A new reference computes its candidate keys. For `"./db"` from `src/a.ts` those are `src/db.ts`, `src/db.tsx`, `src/db/index.ts` and so on. It registers under each key and resolves against the tree.
   - A path that appears or disappears looks up only the references waiting on that key and re-resolves those.
   - An edge opens when its first supporting reference resolves and closes when its last one goes, writing the interval `[added_t, removed_t)`.

5. **Resolver inputs are versioned too.** `tsconfig.json` paths, `package.json` name and exports, `go.mod`, `composer.json` autoload and `pyproject.toml` are files in the tree. When one changes, the references in its directory scope are re-resolved from cached facts. Nothing is parsed again.

6. **Lineage.** The walk uses `--no-renames` so facts never depend on git's rename heuristic, which is quadratic and gives up silently past `diff.renameLimit`. Inside one commit, deleted and added paths are paired by identical blob id first, then by bounded content similarity within the same service. A pair keeps its entity id.

7. **Cross-service pass.** Provider descriptors (routes, topics, tables) feed per-service indexes. Consumer descriptors wait on them through the same reverse index. A cross-service edge is attributed to whichever commit, in either repository, completed the match or broke it.

8. **Roll up and anchor.** Each edge is lifted to every ancestor pair below the lowest common ancestor (file → module → service) as count deltas, so a service-level view never reads file rows. Every K timeline steps an anchor records the set of active edges, so a jump to any date costs one anchor read plus at most K deltas.

### Cost

| Step | Work | Parallel |
| --- | --- | --- |
| Walk | O(commits + change events) | Per repository |
| Extract | O(bytes of unique blobs), once per extractor version | Per blob |
| Replay | O(fact deltas + references re-resolved), independent of tree size | Sequential |
| Naive baseline | O(Σ tree size at each commit) parses | |

### Later runs

- **New commits.** Store the indexed tip per repository. Walk `tip..HEAD`, extract new blobs, replay. If a new commit's corrected date falls before the workspace's indexed tip, rewind to the anchor at or before it and replay forward from cached facts.
- **Rewritten history.** If the stored tip is no longer on the first-parent chain, find the last commit common to the old and new chains, truncate intervals after it and replay.
- **Extractor upgrade.** A new extractor version invalidates the facts for that language only.

### Correctness check

The replayed state at any commit must equal a from-scratch index of that commit's tree. This differential test is the indexer's main oracle, and the experiment below already asserts it at `HEAD`.

## Measurement

`spike.mjs` implements steps 1, 2 and 4 for import references, without resolution. It was run read-only on a private TypeScript monorepo.

| | |
| --- | --- |
| Commits in the repository | 35,439 |
| Commits on the first-parent chain | 9,498 |
| Source files at `HEAD` | 8,944 |
| Source change events | 61,434 |
| Distinct blobs parsed | 57,060 |
| Parses for a commit-by-commit indexer | 41,016,305 |
| Reduction | 719× |

| Phase | Time | Rate |
| --- | --- | --- |
| Walk | 1,304 ms | 7,281 commits/s |
| Extract (632 MB of source) | 2,978 ms | 19,162 blobs/s, 212 MB/s |
| Replay (46,566 reference intervals) | 122 ms | about 503,000 events/s |
| **Total** | **4.4 s** | |
| Peak memory | 587 MB | |

Machine: Apple M4, 10 cores, git 2.52, Node 22, single thread, warm disk cache, one run.

### What this shows and what it does not

It shows that the walk and the blob-keyed cache hold up on a real history, and that replay is a small part of the total.

It does not show the final indexer's speed:

- Extraction was a regular expression, not a syntax tree. A real parser is slower per byte.
- There was no resolution, no lineage, no cross-service pass and no index written to disk.
- It is one repository, one run and one machine. No variance is reported.
- The naive baseline is counted, not timed.

### Reproduce it

```sh
node docs/research/spike.mjs /path/to/a/git/repository
```

It needs Node 22 and git, uses only git plumbing commands and writes nothing. It prints the counts and timings as JSON and fails if the replayed end state differs from a from-scratch extraction of `HEAD`. If you run it, an issue with your numbers and hardware is welcome.

## Prior work and what is new

Each ingredient exists already:

- Content-addressed archives and deduplication by blob: [Software Heritage](https://arxiv.org/pdf/2011.07824).
- Per-file analysis with cross-file resolution deferred: GitHub's [stack graphs](https://github.blog/open-source/introducing-stack-graphs/) (the project is now [archived](https://github.com/github/stack-graphs)).
- Added and removed commit hops replayed oldest first: Sourcegraph's [Rockskip](https://sourcegraph.com/docs/code-navigation/rockskip). Rockskip re-parses changed paths and tracks symbols only.
- Anchors plus deltas for graphs that change over time: [DeltaGraph](https://arxiv.org/abs/1207.5777) and [AeonG](https://arxiv.org/abs/2304.12212).
- Corrected commit dates: git's [commit-graph](https://git-scm.com/docs/commit-graph) format.

What is specific to Cartograph is the combination: facts keyed by blob id, a reverse index that re-resolves only the references a commit can affect, and one merged timeline across repositories so that cross-service links get a first and last commit too. It is an engineering synthesis, not a published result.

## Open questions

- Parser throughput on real corpora. Published figures are sparse: [oxc](https://github.com/oxc-project/bench-javascript-parser-written-in-rust) reports its own benchmark, and no official tree-sitter benchmark was found.
- Whether `git log --raw` or `git diff-tree --stdin` is the faster walk. No published comparison was found.
- Precision and recall of static cross-service detection. No numbers were found for TypeScript, Go, Python or PHP.
- How deep constant propagation has to go to recover URLs and topic names in practice.
- Index size on very large histories.

## Planned evaluation

Cold and incremental runs on public repositories pinned to commit SHAs in three size tiers (django and laravel/framework, kubernetes and vscode, linux and chromium), timed with [hyperfine](https://github.com/sharkdp/hyperfine) over at least ten runs, with hardware, OS and git version reported alongside every number. Starting targets, to be replaced by measured baselines:

| Measure | Target |
| --- | --- |
| History walk | ≥ 5,000 commits/s |
| Replay with resolution | ≥ 200,000 events/s |
| Cold index, up to 50,000 commits | ≤ 2 min |
| Update with up to 100 new commits | ≤ 1 s |
| Peak memory, small and medium tiers | ≤ 1.5 GB |

These are targets, not results.
