# idx-spike (IDX-02)

The indexer-side phase 0 measurements. Results and method are in
[`docs/research/phase0.md`](../../docs/research/phase0.md). Never shipped: it lives in the separate
`bench/` cargo workspace, so `git2` (a comparison baseline) stays out of the product's lockfile and
`cargo deny`.

```sh
bench/idx-spike/run.sh /some/scratch/folder   # clones remotion (about 4 GB), builds, runs everything
```

The folder gets the bare clone, the freshness fixture, the SQLite files and `results/idx02/*.json[l]`.
Keep it outside the repository. `RUNS=10` by default.

| Subcommand | What it measures |
| --- | --- |
| `walk <repo> <rev> gix\|git2\|git [--renames]` | First-parent walk plus a tree diff per commit. `--renames`: exact, then 50% similarity, 1,000 candidates (B7) |
| `agree <repo> <rev>` | Per-commit agreement of gix and libgit2 with `git log --raw` and `git log -M50%` |
| `resolve <repo> <rev>` | oxc_resolver over an in-memory file system built from the git tree (no checkout); what `file_dependencies` and `missing_dependencies` report (H1) |
| `replay <repo> <rev> --pairs N --gap G` | For commit pairs G apart: is a resolution whose dependencies didn't change still the same? |
| `fresh-fixture <dir> <n> [--extra repo@rev]` / `fresh <state.json>` | D1: HEAD against the indexed commit, commits behind capped at 1,000 |
| `sqlite <dir> remotion\|synthetic` | Intervals in SQLite: write, size, load, as-of query from disk |

Fixtures are public repositories, read as data through the object database. Nothing in them is
checked out, built or run, and `gix` is compiled without network features.
