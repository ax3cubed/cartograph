# Dating every dependency without re-indexing history

A research note on how Cartograph's indexer finds, for every link between two files, the commit that added it and the commit that removed it, without indexing the code again at every commit.

**Status.** A method plus measured experiments. [`bench.mjs`](bench.mjs) in this folder runs all of them on any git repository or folder of repositories. The production indexer is stage 1 of the [roadmap](../ROADMAP.md) and is not built yet. Nothing here has been peer reviewed. The full write-up, with charts, the model and the prior-work table, is [`site/research.html`](../../site/research.html).

## Results in short

Measured on the public [remotion](https://github.com/remotion-dev/remotion) repository at commit `2b1b0c9` (35,439 commits, 9,498 on the first-parent chain, 9,075 tracked files at head), for import links and package links.

| Question | Result |
| --- | --- |
| Time to date every link over the whole first-parent history | 4.6 s median of 9 runs (best 3.1 s, worst 5.6 s) |
| Time to index every commit from scratch instead | about 25 minutes, extrapolated from 24 commits indexed that way |
| Speed-up | about 330× at the median. Three runs of the suite gave 266× to 364× |
| Same answer as the direct method? | Yes. Files and links identical at 24 of 24 sampled commits |
| Parses | 41,714,790 for every commit from scratch; 105,293 for changed paths only; 103,347 with a blob-id cache |
| What the reverse index saves | 60,039 resolutions instead of 37,817,975 (630× fewer), identical intervals, replay about 8× faster |
| 100 new commits on an indexed history | 105 ms |
| Jump to an arbitrary commit | 0.06 ms by scanning all 36,193 intervals. Snapshot anchors were slower at every spacing tried |
| A folder of 43 repositories on one timeline | 14,730 commits in 8.0 s median; 153 cross-repository links dated |
| How long links live | 74% still present after one year, 67% after two; median about 5 years |

Three results changed the design:

1. **The speed comes from processing only what each commit changed, not from content addressing.** On one first-parent chain the blob-id cache saves 1.8% of parses on top of that. It saves 2.6× when every commit is indexed, and it makes re-runs free.
2. **Snapshot anchors are not needed at this size.** A scan of the interval list is faster. They are dropped from the first version.
3. **Extraction is mostly git reading objects** (64% to 78% of one-worker time), not parsing. How well it spreads across cores could not be settled: the best worker count was 4, 2, 8 and 2 in four sweeps on a busy laptop.

## The method

Split what the indexer knows into two kinds.

- **Facts** are what a file says: it imports `"./db"`, or its `package.json` declares the name `@remotion/renderer`. A fact depends only on the file's bytes, so it is extracted once per git blob id.
- **Links** are what those facts mean: which file `"./db"` is. A link depends on the rest of the tree at that commit.

Then:

1. **Walk.** One streaming `git log --first-parent --diff-merges=first-parent --reverse --raw --no-abbrev --no-renames` per repository. Each commit yields the paths it changed and their new blob ids. Dates are made strictly increasing along the chain.
2. **Extract.** Facts for each blob id not seen before, in parallel. The only step that reads file contents.
3. **Merge.** Commits of all repositories on one timeline, ordered by corrected date.
4. **Replay.** Apply the steps in order, holding the tree, the references and a reverse index from key to the references waiting on it. A reference has an ordered list of candidate keys (`src/db.ts`, `src/db.tsx`, …, `src/db/index.ts`, or a package name) and resolves to the first one that exists. When a key appears or disappears, only the references waiting on it are re-resolved. Each link counts its supporting references, and an interval `[added, removed)` is written when the count leaves or returns to zero. A link that closes and reopens inside one commit never left.

Why step 4 is exact: a reference's resolution can only change when one of its candidate keys enters or leaves the tree, and the reverse index lists exactly the references registered under each key. By induction, the replayed links at any step equal the links computed from that step's tree alone. The experiments check this against an index that shares no state with the replay.

| Step | Work |
| --- | --- |
| Walk | Commits plus changed paths |
| Extract | Bytes of distinct blobs, once per extractor version |
| Replay | Fact changes plus references re-resolved. Independent of tree size |
| Index every commit (baseline) | The sum of the tree size over all commits |

## Run it

```sh
node docs/research/bench.mjs                                # self-check on a hand-made history
node docs/research/bench.mjs /path/to/repo --runs 9          # one repository
node docs/research/bench.mjs /path/to/folder                 # every repository under a folder, one timeline
node docs/research/bench.mjs /path/to/repo --quick           # timings and checks only, no baselines
```

It needs Node 22 and git 2.31 or later, has no dependencies, uses only git plumbing commands and writes nothing to the repository. It prints JSON (or writes it with `--json file`) and stops with an error if any comparison fails. If you run it, an issue with your numbers and hardware is welcome.

What it checks on every run:

- the replayed tree equals `git ls-tree` at each sampled commit and at head;
- the replayed links equal a from-scratch index of each sampled commit, read straight from git;
- the written intervals reproduce the link set at each sampled step;
- the replay without a reverse index writes identical intervals;
- an incremental update of the last 100 commits ends in the same state as the full run;
- anchor queries agree with a scan.

## Limits

- Extraction is a regular expression that finds import specifiers. It misses computed imports and can match text in strings or comments. A real parser will be slower per byte.
- Resolution is simplified: relative paths with the usual extensions and index files, and package names from `package.json`. No `tsconfig` paths, no `exports` maps.
- JavaScript and TypeScript only.
- The from-scratch index shares the extraction pattern and the candidate rule with the replay. It shows the replay loses nothing. It cannot catch a mistake both share.
- First-parent only. Work on a side branch appears at its merge.
- Order across repositories rests on commit dates, which can be wrong.
- Timings were taken on a laptop under load (load average 11 to 14). Counts are exact; times are noisy.
- One repository with a long history. The baseline total is extrapolated from 24 samples. Nothing is written to disk.

## Prior work

Every ingredient exists already. No published description of the whole combination was found in a web search (ACM and IEEE full text and patents were not searched).

- **[Stack graphs](https://arxiv.org/abs/2211.01224)** (Creager and van Antwerpen, 2023): per-file analysis keyed by git blob id, stored once and reused. Resolves one snapshot per query; no lifetimes.
- **[Rockskip](https://sourcegraph.com/docs/code-search/code-navigation/rockskip)** (Sourcegraph): first-parent replay, parses only changed files, stores symbols with the commits that added and deleted them. Symbols per path only; no references or links.
- **[LISA](https://files.ifi.uzh.ch/seal/people/alexandru/saner17-multirev.pdf)** (Alexandru, Panichella, Gall, SANER 2017): all revisions in one graph with revision ranges on syntax-tree vertices. Single-file analyses; no cross-file resolution.
- **[DiffBase](https://chenguang-zhu.github.io/assets/pdf/Wu2021Diffbase.pdf)** (Wu, Zhu, Li, ESEC/FSE 2021): program facts annotated with the versions in which they hold. Evaluated on short history fragments.
- **[Hercules](https://github.com/src-d/hercules)**: the same pipeline shape, for line survival and co-change.
- **Watch-mode module resolution** ([webpack](https://webpack.js.org/api/loaders/) missing dependencies, TypeScript failed lookup locations): the reverse index, applied to a working tree instead of commits.
- **Incremental view maintenance** ([DBSP](https://www.vldb.org/pvldb/vol16/p1601-budiu.pdf)): replay is a join maintained under inserts and deletes, with the reference count as the weight. First-match resolution is not monotonic, so the fit is loose there.
- **Temporal graph storage** ([DeltaGraph](https://arxiv.org/abs/1207.5777), [AeonG](https://arxiv.org/abs/2304.12212)): snapshots plus deltas, which is the anchor scheme tested and dropped here.
- **[git-pkgs](https://nesbitt.io/2026/01/01/git-pkgs-explore-your-dependency-history.html)** and **[codegraph PR 72](https://github.com/swalla02/codegraph/pull/72)**: the same question, at manifest level and as per-commit gains and losses.

The contribution is the combination and the measurements: resolved cross-file links with reference-counted lifetimes over a whole history, the reverse index applied to commit time, one timeline across repositories, and each part checked against the direct method.

## Next

- Replace the pattern with a real parser and repeat the timing and parallelism experiments.
- Run pinned public repositories at three sizes on a quiet machine, with at least ten runs.
- Add route, topic and table descriptors so cross-repository links cover HTTP, events and databases, and measure how many are found against a hand-labelled set.
- Write the intervals to SQLite and measure load time, size and query time from disk.
