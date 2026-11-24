# Link precision (XS-07)

Run `cargo run -p cartograph-eval` to print this table; add `--write` to rewrite this page and `precision.json`. The gate runs as a test, so `cargo test -p cartograph-eval` fails when it is not met.

## Set (a): the hand-labelled TS, Go and Python workspace

Seven services under `eval/fixtures/set-a/workspace`, committed into a throwaway repository and indexed by the real pipeline, compared with `eval/fixtures/set-a/expected.json`. Only the TypeScript adapters (Express routes; fetch, axios, got and ky calls) exist today, so the Go and Python services are in the labels to keep recall honest: every link into or out of them is a miss. The fixture also holds traps: hostless calls, one route in three services, a pattern route, an external host and an unknown host. A link's level is the pair of files' level, so an operation takes its file pair's. Recall by level or evidence is that row's true positives over all labelled links.

| Level | Predicted | True positives | Precision | Recall |
|---|---:|---:|---:|---:|
| exact | 0 | 0 | n/a | 0.00 |
| likely | 15 | 15 | 1.00 | 0.62 |
| guessed | 3 | 3 | 1.00 | 0.12 |
| weak | 0 | 0 | n/a | 0.00 |

| Evidence | Predicted | True positives | Precision | Recall |
|---|---:|---:|---:|---:|
| literal | 11 | 11 | 1.00 | 0.46 |
| template | 14 | 14 | 1.00 | 0.58 |
| inferred | 2 | 2 | 1.00 | 0.08 |

**24 labelled links.** Gate: exact precision at least 0.95, likely at least 0.75; a level with nothing predicted is n/a and passes.

Overall: 18 predicted, 18 true, precision 1.00, recall 0.75.

Missed:

- inventory -> orders http get /orders/{}  (Go call sites are not extracted)
- notifier -> users http get /users/{}  (Python call sites are not extracted)
- orders -> inventory http get /stock/{}  (Go routes are not extracted)
- orders -> inventory http post /stock/reserve  (Go routes are not extracted)
- orders -> notifier http post /notify  (Python routes are not extracted)
- users -> notifier http post /notify  (Python routes are not extracted)

False positives:

- none

## Set (b): the 17 Code2DFD applications

Pending. It needs Java and Spring adapters, which do not exist yet.
