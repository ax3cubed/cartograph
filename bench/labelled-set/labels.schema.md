# Hand-labelled cross-service links (INT-02, gates XS-07)

One YAML file per repository. Each file pins one commit and lists every cross-service link a careful
reader finds in that commit's source, with the lines that prove it. The matcher (XS-07) compares the
indexer's links at that commit with these labels and reports precision and recall per confidence level.

Labels are written by reading the source with `git show <commit>:<path>`. Nothing in a labelled
repository is built or run.

## File shape

```yaml
version: 1
repo: instana/robot-shop             # GitHub slug
commit: 55292e2199f2fb00a165b1f7d3045fe7f8922038   # full sha; labels hold for this commit only
licence: Apache-2.0
services:                            # the service split the labels assume (C6 rule 1 would write the same)
  - { id: cart, paths: [cart/] }
infra:                               # brokers and stores, so db and event links have a target
  - { id: redis, kind: kv }
links:
  - from: cart                       # "from depends on to" (every kind; events follow ruling N3)
    to: catalogue
    kind: http                       # imports | calls | http | event | db (wire names, ruling R28)
    op: GET /product/:sku            # http: METHOD path in the server's route syntax, normalised to :param
                                     # event: <exchange or topic>[:<routing key or queue>]
                                     # db: <database>[.<table or collection>]; null for a key-value store
    expected: likely                 # exact | likely | guessed, per H2's evidence classes
    why: host from env default, path built by concatenation   # one line: which H2 class and why
    evidence:                        # file:line at the pinned commit, both ends where there are two
      - cart/server.js:362
      - catalogue/server.js:82
    requires: null                   # an extractor the roadmap doesn't have yet (e.g. nginx-adapter);
                                     # such links are left out of precision until it exists
channels:                            # event links also as unambiguous channel edges (ruling N3)
  - { service: payment, role: publishes, channel: "robot-shop:orders", evidence: [payment/rabbitmq.py:29] }
not_links:                           # things that look like links and aren't; a found one is a false positive
  - { from: cart, to: cart, why: "/health is a probe, not a dependency" }
```

## Rules

- A link is labelled once per (from, to, kind, op). Two call sites of one operation are one link with
  two evidence lines.
- `expected` is the level the evidence class deserves under H2, not what any tool reports. Precision
  per level is `matched labels at that level / links the indexer reported at that level`.
- An unlabelled link the indexer reports is a false positive unless a reviewer adds it here, with
  evidence, in the same pull request.
- Health, metrics and readiness probes are not links.
- Load generators, tests and deployment scripts are out of scope.
