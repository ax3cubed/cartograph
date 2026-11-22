# Code2DFD / microSecEnD fetch (INT-02)

Decision H2 names "the 17 Code2DFD applications" as the precision benchmark. The ground truth is the
**microSecEnD** dataset (Schneider, Özen, Chen, Scandariato, MSR 2023,
[github.com/tuhh-softsec/microSecEnD](https://github.com/tuhh-softsec/microSecEnD)): manually made
dataflow diagrams of 17 open-source Java/Spring microservice applications. The extraction tool is
[github.com/tuhh-softsec/code2DFD](https://github.com/tuhh-softsec/code2DFD).

```sh
node bench/code2dfd/fetch.mjs /path/to/scratch      # default: $TMPDIR/cartograph-code2dfd
```

It clones the dataset at a pinned commit, then each application bare and blobless at the commit the
dataset modelled (the sha in the traceability file if there is one, otherwise the last commit on the
modelled branch before the dataset was committed, 2023-01-26). It counts files per language at that
commit with `git ls-tree` and writes `code2dfd-languages.json`. It never checks out a working tree,
builds or runs anything; hooks are off and credential prompts are disabled. About 75 MB on disk;
clones are reused on re-runs.

Results and the recommendation are in `docs/research/phase0.md` (INT-02).
