# Labelled set for link precision (XS-07 gate)

Hand-labelled cross-service links in public, permissively licensed microservice repositories written
in the languages the roadmap extracts (TypeScript/JavaScript, Go, Python). Started in phase 0 by INT-02
because the Code2DFD dataset (microSecEnD) is all Java and Spring. Schema: [labels.schema.md](labels.schema.md).

| Repository | Commit | Licence | Languages | Link kinds it exercises | Status |
| --- | --- | --- | --- | --- | --- |
| instana/robot-shop | `55292e2` | Apache-2.0 | JS, Python, Go, Java, PHP | http, event (RabbitMQ), db (Redis, MongoDB, MySQL), nginx gateway | [labelled](robot-shop.yaml): 21 links, 2 channel edges |
| GoogleCloudPlatform/microservices-demo | `c7cb6d5` | Apache-2.0 | Go, Python, C#, Node | gRPC (11 services), Redis | to label; needs a ruling on how gRPC maps onto `http` |
| open-telemetry/opentelemetry-demo | `fc7f2a6` | Apache-2.0 | TypeScript, Go, Python, Java, C#, Elixir, more | gRPC, HTTP, Kafka, Postgres, Valkey | to label; the TypeScript-heavy one |
| GoogleCloudPlatform/bank-of-anthos | `db35fea` | Apache-2.0 | Python, Java | REST, Postgres | to label |
| thangchung/go-coffeeshop | `22d3319` | MIT | Go | gRPC, RabbitMQ, Postgres | to label |

Commits are the default-branch heads on 22 Nov 2026 (robot-shop: last commit, archived). Clone with
`git clone --bare --filter=blob:none` and read with `git show <commit>:<path>`; never build or run them.

Target before XS-07 gates CI: at least 150 labelled links, at least 30 per level the indexer emits, and
each of http, event and db in at least two languages.
