# Phase 0: spikes, benchmarks and setup

The measured results behind phase 0 of [the implementation plan](../architecture.html): one result per exit-gate line, the method, and where the raw output is. Each benchmark can be re-run from `bench/`.

**Status.** All 9 items were run on 22 Nov 2026. Raw output lives in the session scratch folder (`phase0/`, written `phase0/` below). It isn't committed, so the paths are only useful on this machine; the commands that regenerate it are in each section.

## Machine and load

| | |
| --- | --- |
| Hardware | Apple M4 (10 cores: 4 performance, 6 efficiency), 24 GB, internal SSD with 21–34 GB free during the runs |
| OS and tools | macOS 26 (Darwin 25.6.0), rustc 1.100.0, git 2.52.0, Node 22.16.0, pnpm 11.23.0, Chrome 158 |
| Load | **Not a quiet machine.** The 1-minute load average was 4.7 to 5.4 before any of this work started (a virtual machine and the desktop), and 7 to 40 while the spikes ran side by side. The load is recorded with every timed run in the JSONL files. Counts and agreement figures are exact; times are upper bounds |

## Exit gate

| Gate | Measured | Result | Decision |
| --- | --- | --- | --- |
| oxc_resolver over a virtual file system reports `file_dependencies` and `missing_dependencies`, or the candidate-list fallback is chosen (IDX-02) | A custom `FileSystem` over a git tree works. Both sets are reported: 31,349 resolutions at `2b1b0c9` in 51 ms, with a mean of 0.66 file and 3.46 missing dependencies each. **They leave out the `package.json` and `tsconfig.json` files the resolver reads.** At a 2,000-commit gap, 44 of 56,079 answers changed although none of their reported dependencies did, all because an `exports` field changed. Adding the nearest manifests, compared on resolution fields only, gives 0 of 31,703 | **Pass, with an amendment** | H1 holds. No fallback needed; IDX-06 also keys each resolution on the manifests |
| gix walk and renames agree with `git log -M50%` on remotion's 9,498 commits; time against `git log --raw` and libgit2 | Changed paths agree on 9,498 of 9,498 commits. Renames differ on 19 commits: gix found 2,291, git 2,344, 2,278 in both. libgit2 differs on 88 commits. Walk with tree diff, median of 10: git 555 ms, **gix 733 ms**, libgit2 11,458 ms. With renames: git 1,740 ms, gix 2,584 ms, libgit2 15,688 ms | **Paths pass. Renames: near-agreement, not exact** | B7 holds: gix. libgit2 is not a better rename oracle. The rename gate becomes "a documented difference", and the proposed 2× overhead mark becomes 3.5× (see below) |
| Freshness for 30 repos under 100 ms, no network syscall | 30 repos, one of them the remotion clone 1,000+ commits behind. Median of 10 cold processes: 51 ms on one thread, 36 ms on 8; 12 to 15 ms when the repos have a commit-graph. Run under a sandbox that denies every network operation, and the binary has no socket symbols. 43 repos: 63 ms; 50 repos: 67 ms on one thread | **Pass** | D1 holds. The time is in counting commits behind, not in reading HEAD (see below) |
| sigma 4 at 60k nodes and 200k links: p95 frame under 33 ms, pick under 50 ms, load under 2 s, heap under 600 MB, integrated GPU | p95 frame 16.7 ms with `hideEdgesOnMove`, **33.4 ms with edges drawn while moving**; pick p95 30.5 ms; load 1,387 ms; heap 227 MB. Apple M4 GPU through ANGLE Metal (real GPU, not SwiftShader) | **Pass, with `hideEdgesOnMove`** | I1 holds. FE-08 sets `hideEdgesOnMove` and keeps level graphs in memory. Re-run on Intel UHD |
| Code2DFD fetched, languages confirmed; labelled set started (INT-02) | microSecEnD at `63653e0`: 17 apps, all Java and Spring (685 Java files). Flows carry no operation. The labelled set has started: robot-shop, 21 links | **Pass** | H2 changes benchmark; N4 takes the recommended default |
| genai, keyring and ACP checked feature by feature, each fail with a named fallback (AI-01) | genai 0.7.5 passed every feature on 5 provider shapes against a local mock. keyring 4.2.0 set, get and delete pass on macOS headless; **delete fails after the binary is rebuilt**. ACP: a client can offer no capabilities, but claude-agent-acp keeps its own tools unless vendor `_meta` switches it off | **Pass, every fail has a fallback** | G3 holds, if AI-07 always builds an explicit `ServiceTarget` (genai reads env keys by default). G4 holds, with a tombstone fallback for delete. G2/R8 confirmed |
| npm install fails naming pnpm; a GPL or AGPL crate fails CI; a failing test blocks merge on Linux, macOS and Windows | npm stops with `EBADENGINE … "npm":"please-use-pnpm"` and writes nothing. An AGPL canary (`forceatlas2`) fails `cargo deny` on both licence and ban. The CI matrix covers all three OSes | **Pass locally. CI not yet run on GitHub** | Branch protection on `main` must require the checks |

### Decisions this changes

No gate failed outright. These results change something in a decision or a proposed mark:

1. **H1 / IDX-06: the reverse index also keys on manifests.** oxc_resolver's dependency sets miss `package.json` and `tsconfig.json`. Each resolution also depends on:
   - the nearest `package.json` above each dependency
   - the nearest `package.json` and `tsconfig.json` above the importer

   Compare a manifest on the fields resolution reads (`name`, `main`, `module`, `types`, `exports`, `imports`, `browser`, `type`, `extends`, `compilerOptions`, `references`), not its bytes. Otherwise every version bump invalidates every importer. The candidate-list fallback isn't needed.
2. **B7 / IDX-18: renames are close to git, not identical.**
   - gix agrees with `git log -M50%` on 99.8% of commits and finds 97.2% of git's renames. Its extra 13 are mostly ties broken differently; most of its misses are binary files and cross-package moves.
   - libgit2 is further off (88 commits), so "use libgit2 for diffs" would not help.
   - The rename gate becomes "within 1% of commits, the differences listed", not "identical".
   - The proposed overhead mark (under 2× the walk) fails at 3.5× (2,584 ms against 733 ms), the same as git's own (3.1×). Proposed new mark: under 4× the walk, which is under 3 s for all of remotion.
3. **D1 / IDX-15: counting commits behind is the cost.**
   - Reading HEAD is about 0.3 ms a repo. One repo 1,000+ commits behind, with no commit-graph, costs 34 ms of the 51.
   - IDX-15 should read the commit-graph when present (which brings the total to 12 ms) and check repos on a few threads.
   - Or, if a repo blows the budget, answer "behind" and count after the answer, as `dirty` already does.
4. **I1 / FE-08:** `hideEdgesOnMove: true` at file level is required, not optional. Keep each level's graph alive: rebuilding the 60k level costs 0.9 s.
5. **Jump to an arbitrary commit from disk** (SRV-04, proposed under 5 ms over 200k links): 5.2 ms median and 8.3 ms p95 on a fresh connection, returning 115k live rows. Marginal. Loading all 230k intervals takes 23 ms, so SRV-04 should hold them in memory per generation (the research's 0.06 ms scan) rather than query SQLite per jump.
6. **G3 / AI-07:**
   - Always pass an explicit `ServiceTarget`, never a model name, so no ambient `*_API_KEY` is read.
   - Supply our own reqwest client with no proxy and no redirects.
   - Depend on `keyring-core` and the per-platform stores (genai 0.7.5, keyring 4.2.0).
7. **G4:** a rebuilt, ad-hoc-signed binary can't delete its own keychain item. Overwrite the item with an empty tombstone, and recheck once signing (N6) exists.
8. **H2 and N4:** precision per level is measured on the hand-labelled TypeScript, Go and Python set. Code2DFD figures come later, at service-pair level, once Java adapters exist.

## IDX-02 · Indexer benchmarks (B7, H1, D1, SQLite)

**Fixture.** remotion, a bare clone (4.0 GB) read at `2b1b0c9feb7b`: 9,498 first-parent commits, the same commit as `docs/research/README.md`. No commit-graph for the walks.

**Re-run it.** `bench/idx-spike/run.sh <work-dir>` does the clone, the build and every experiment, with 10 runs each and the load average recorded per run. Raw output: `phase0/results/idx02/`:
- `walk.jsonl`, `agree.json`
- `resolve.json`, `replay.jsonl`
- `fresh.jsonl`, `net-symbols.txt`
- `sqlite.jsonl`, `hardware.json`

During the runs the 1-minute load average was 5.5 to 11.

### B7 · First-parent walk, tree diff, renames

Each implementation walks the 9,498 commits and diffs each tree against its first parent's. 138,257 changed paths in all. Times are the median of 10 runs, interleaved, with the min and max in brackets.

| | git log --raw (bench.mjs's walk), parsed | gix 0.90 | libgit2 (git2 0.21, bench only) |
| --- | --- | --- | --- |
| Walk and diff | 555 ms (541–1,185) | 733 ms (722–778) | 11,458 ms (10,955–13,908) |
| Plus renames: exact, then 50%, 1,000 candidates | 1,740 ms (1,675–2,261) | 2,584 ms (2,410–3,485) | 15,688 ms (15,478–18,177) |
| Commits whose changed paths match git's | 9,498 / 9,498 | 9,498 / 9,498 | 9,498 / 9,498 |
| Renames found | 2,344 | 2,291 (2,278 also in git) | 2,472 (2,331 also in git) |
| Commits whose renames differ from git's | | 19 | 88 |

- **gix is in-process with no pipe, but not faster than git's own C here.** 733 ms against 555 ms. Both are well under the research's whole-pipeline figure, because the walk is a small part of it. The research's 64–78% "git reads" were blob reads through `cat-file`, which this spike doesn't time; that cost lands in extraction (IDX-05).
- gix's peak memory with renames was 1.5 GB, against 300 MB without. The resource cache keeps blob buffers. IDX-18 should bound it.
- **Examples where gix differs:**
  - a binary `video.mp4 → output.mp4` that git pairs and gix doesn't
  - cross-package test moves (`packages/core/src/test/crf.test.ts → packages/renderer/…`)
  - one tie broken differently (`MotionBlurExample.tsx →` `CameraMotionBlurExample` in gix, `TrailExample` in git)
- **The full list** is in `agree.json`.

### H1 · oxc_resolver over the git tree

- **The file system.** `TreeFs` implements oxc_resolver 11.24.3's `FileSystem` trait (`read`, `metadata`, `symlink_metadata`, `read_link`, `canonicalize`) over a map built from the tree at a commit. Blob text is read from the object database on demand.
  - Workspace packages become `node_modules/<name>` links, as `pnpm install` would make them. 131 packages at `2b1b0c9`.
  - `ResolverGeneric::new_with_file_system` takes it, so **a custom file system can be supplied**.
  - The tree map builds in 13 ms.
- **At `2b1b0c9`.**
  - 8,944 source files, 31,349 import references (the regex bench.mjs uses).
  - Resolved: 20,346 internal, 9,741 not found (npm packages, which aren't in git), 1,262 Node built-ins.
  - Time: 51 ms with a cold cache, median of 10. That's 1.6 µs a resolution, in memory.
  - Every non-builtin resolution reports dependencies: a mean of 0.66 found files and 3.46 missing candidate paths. So `resolve_with_context` **does report `file_dependencies` and `missing_dependencies`.**
- **Are those dependencies enough to drive replay?** For 24 commit pairs at each gap, the spike resolved every import of every importer whose blob is the same at both commits. Then it counted answers that changed although no reported dependency changed.

| Gap | Resolutions | Answers that changed | Changed, dependencies untouched (oxc's sets only) | Same, after adding the nearest manifests |
| --- | --- | --- | --- | --- |
| 1 commit | 341,587 | 1 | 0 | 0 |
| 50 commits | 299,708 | 1 | 0 | 0 |
| 2,000 commits | 75,654 | 80 | **44** | **0** |

All 44 are packages whose `exports` changed: `@remotion/renderer/client` stopped resolving, and `@remotion/google-fonts/*` changed error. oxc reads `package.json` but doesn't list it as a dependency. Comparing manifests on resolution fields only keeps the re-resolve share low: at a 50-commit gap, 10.8% of resolutions need re-resolving, against 3.5% on oxc's sets alone.

### D1 · Freshness check

- **The fixture** (`bench/idx-spike` `fresh-fixture`) is 50 repos made locally with `git fast-import`, plus the remotion clone indexed at `2b1b0c9` and 1,000+ commits behind. The other repos are 0, 3 or 40 commits behind, or 1,500 (past the cap), and half have packed refs.
- **The check** per repo: open with gix (isolated, no global config), read HEAD, compare with the indexed commit, and count first-parent commits behind up to 1,000.
- **No network.** Every timed run went through `sandbox-exec` with `(deny network*)`, and `nm -u` finds no `connect`, `getaddrinfo`, `socket` or `sendto` in the binary. gix is built without network features.

| Repos | No commit-graph, 1 thread | 8 threads | Commit-graph, 1 thread | 8 threads |
| --- | --- | --- | --- | --- |
| 30 | 51.2 ms (p95 58.9) | 36.4 ms (p95 43.0) | 15.4 ms (p95 17.8) | 8.5 ms (p95 9.3) |
| 43 | 63.4 ms | 37.9 ms | 19.4 ms | 9.2 ms |
| 50 | 67.2 ms (p95 102.6) | 37.9 ms | 21.0 ms | 9.4 ms |

- **Method.** Medians are over 10 separate processes; the check is the first thing each process does. The in-process repeats are within 1 ms of these.
- **Per repo.** Up to date: 0.3–0.5 ms. Synthetic 1,500 behind: about 4 ms. remotion, 1,000+ behind with no commit-graph: 34 ms.
- **What's left out.** The OS page cache was warm (no `purge` without root), and `dirty` isn't included (the plan computes it after the answer).

### SQLite · intervals on disk

rusqlite 0.43 with bundled SQLite. STRICT tables, WAL, one transaction, an index on `(from_seq, to_seq, link)`. The rows are synthetic, with the stated counts and a seeded generator.

| Set | Intervals | File | Write (median of 10) | Load every interval | As of a random step, new connection | Same, warm (p50 / p95) | Live rows |
| --- | --- | --- | --- | --- | --- | --- | --- |
| remotion-sized: 9,075 files, 33k links, 9,498 steps | 36,193 | 2.0 MB | 32 ms | 3.5 ms | 1.0 ms (p95 1.5) | 0.6 / 1.3 ms | about 18k |
| decision's fixture: 60k files, 200k links, 10,000 steps | 230,000 | 13.3 MB | 220 ms | 23 ms | 5.2 ms (p95 8.3) | 3.9 / 8.1 ms | about 115k |

## FE-07 · sigma 4 at 60k nodes and 200k links (decision I1)

**Result: I1 holds, with `hideEdgesOnMove` on.** With edges drawn while the camera moves, the
workspace-zoom p95 frame is 33.4 ms, just over the 33 ms mark, so it fails without that lever.

| Mark (proposed) | Measured (DPR 2, 1440×900) | Measured (DPR 1) | Pass |
| --- | --- | --- | --- |
| p95 frame < 33 ms at workspace zoom, `hideEdgesOnMove` on | 16.7 ms (vsync-bound; finished frame p95 5.9 ms) | 16.7 ms (finished 7.9 ms) | **pass** |
| same, edges drawn while moving | 33.4 ms (worst run 50 ms); finished frame p95 34.6 ms, median 32.4 ms | 33.4 ms; finished p95 27.0 ms | **fail** (marginal) |
| pick < 50 ms (mousemove → `enterNode`) | median 24.0, p95 30.5, max 145 ms (n = 200, 100% hit) | median 16.9, p95 30.8 ms (82% hit) | **pass** |
| load < 2 s (fetch, parse, graph build, `new Sigma`, first frame) | median 1,387 ms, max 1,603 ms | median 1,212 ms | **pass** |
| JS heap < 600 MB (after forced GC) | 227 MB | 228 MB | **pass** |
| level swap: 60k → 62-node service graph | 251 ms | 222 ms | no mark |
| level swap: back to 60k | 876 ms | 701 ms | no mark |

Also recorded: Chrome process-tree RSS 1.1–1.3 GB (includes the GPU process). The payload is 5.2 MB of
columnar JSON. Fetch plus `JSON.parse` takes about 17 ms. The graphology build takes about 270 ms, and
`new Sigma` (indexing and GPU upload) about 975 ms, which is most of the load time and most of the swap
back to 60k. Render CPU per frame is under 1 ms, so the edges-on cost is GPU fill: 200k anti-aliased
edges at 2880×1800 device pixels.

Re-run: `node bench/map/run.mjs` (see `bench/map/README.md`).

**Method.** `bench/map/generate.mjs` makes a seeded fixture (seed 7): 60,000 nodes in 62 Zipf-sized
clusters on a golden-angle spiral, 200,000 unique directed links (85% inside a cluster), plus the
62-node service graph. `bench/map/run.mjs` bundles `bench/map/bench.mjs` with esbuild, serves it on
127.0.0.1, and drives Chrome 158 over CDP: 10 page loads per configuration, each with a 2.4 s
`camera.animate` sequence that stays at whole-workspace zoom (all 60k nodes drawn) and one that pans
and zooms to 0.3. Each sequence runs with `hideEdgesOnMove` off and on, edge labels always off.
- *frame* is the interval between `requestAnimationFrame` callbacks. On a 60 Hz display it can only be
  16.7 or 33.3 ms, so the 33.4 ms p95 means "every frame took two vsyncs".
- *finished frame* times each sigma render up to a synchronous 1-pixel `readPixels`, so the GPU work
  counts and the number isn't rounded to vsync. It's an upper bound, because it defeats pipelining.
- *pick* sends 20 CDP mouse moves per run to node centres at workspace zoom, each after moving to an
  empty corner.

Versions: sigma 4.0.6, graphology 0.26.0, esbuild 0.28.2, all pinned in `bench/map/package.json`.

**Hardware and load.** Apple M4 (10 cores, an integrated GPU), 24 GB, macOS (Darwin 25.6). WebGL
renderer: `ANGLE (Apple, ANGLE Metal Renderer: Apple M4)`. Headless Chrome (`--headless=new`) used the
real GPU, not SwiftShader, so no headed run was needed. The machine was **not quiet**: phase 0 Rust
builds ran in parallel, with load averages of 9–27 during the DPR 2 set and 5–27 during the DPR 1 set.
Frame and pick outliers (the 50 ms worst run and the 145 ms pick max) line up with the load peaks.

**Caveats.**
- The plan asks for an M1 Air and an Intel UHD laptop. The M4 is integrated but much faster than
  Intel UHD, so an Intel UHD machine would very likely fail the edges-on mark by more, and might
  approach it even with edges hidden. Re-run on that class of GPU before FE-08 ships.
- At DPR 1, 18% of picks missed, because nodes are about 1.5 px wide at workspace zoom and the pick
  went to empty space. Latency counts only the hits.
- The GPU timer extension was present but returned no results through sigma's debug hook.

**Decision I1: holds**, on two conditions:
- FE-08 sets `hideEdgesOnMove: true` (or thins edges) at the file level of the whole-workspace view.
- FE-08 keeps each level's sigma instance or graph in memory: rebuilding the 60k level costs about
  0.9 s.

The Cosmos.gl fallback isn't needed on this hardware. It stays the named fallback if the Intel UHD
re-run fails with edges hidden.

Raw results: `phase0/fe07/results/headless-dpr2.json` and `…/headless-dpr1.json` (every run, plus a summary). Fixture: `…/phase0/fe07/fixture/`.

## AI-01 · genai, keyring and ACP (decisions G2, G3, G4)

**Method.** `bench/ai-spike` is a throwaway crate in the bench workspace. It starts a mock provider on `127.0.0.1:<ephemeral>` inside the process. The mock speaks OpenAI chat-completions SSE, Anthropic messages SSE, Gemini `streamGenerateContent?alt=sse` and Ollama `/api/chat` NDJSON, and it records every request's path, headers and body. Each provider has its own base URL on the mock, set through an explicit `ServiceTarget`. Every call goes through our own `reqwest::Client`, built with no proxy and no redirects. No real keys were used, and nothing left the machine.

The pinned versions are genai `=0.7.5`, keyring `=4.2.0` and agent-client-protocol `=5.5.1` (schema 1.21.2).

**Running it.** `cargo test -p ai-spike` runs the automated checks: 6 tests, all passing. `cargo run -p ai-spike` writes the JSON report. The real-key calls are `#[ignore]` tests in `tests/live.rs`; they take their keys from `OPENAI_KEY`, `ANTHROPIC_KEY`, `GEMINI_KEY`, `LIVE_OLLAMA_URL` and `LIVE_COMPAT_URL`.

**Raw output.** The report is at `phase0/ai01/report.json` (with `run.log` and `metadata.json` beside it). The claude-agent-acp sources read are in `.../phase0/ai01/acp-src/`.

### genai, per provider and feature (against the mock)

The mock replays each provider's documented wire format, so "pass" means genai builds the right request and parses that format. It does not prove the live service still matches the format. That is what the ignored live tests are for.

| Feature | OpenAI | Anthropic (native) | Gemini | Ollama (native) | OpenAI-compatible (LM Studio/vLLM shape) |
|---|---|---|---|---|---|
| Streaming text (2 chunks, "Hello") | pass | pass | pass | pass | pass |
| Streaming tool calls, 2 in parallel, args reassembled | pass (interleaved deltas) | pass (`input_json_delta`) | pass (call ids are synthesised: `call#find#0`) | pass (ids synthesised) | pass |
| Usage in stream: prompt / completion / cached | pass 120/30/100 (`stream_options.include_usage` sent) | pass 120/30/100 (+`cache_creation_tokens`) | pass 120/30/100 | pass 120/30, **no cached figure** (Ollama's API has none) | pass 120/30/100, if the server sends a usage chunk |
| Reasoning effort (`ReasoningEffort::Low`) | pass: `reasoning_effort:"low"` | pass: `thinking:{enabled, budget_tokens:1024}` | pass: `thinkingConfig.thinkingBudget:1000` | pass: `think:"low"` | pass: `reasoning_effort:"low"` (the server may ignore it) |
| Cache breakpoints | n/a (automatic) | **pass**: `MessageOptions`/`CacheControl::Ephemeral` gives `system[0].cache_control:{type:"ephemeral"}`. TTL variants `Ephemeral5m`/1h exist, as does `Tool::with_cache_control` | n/a | n/a | n/a |
| Custom base URL per provider | pass | pass | pass | pass | pass |
| Our own `reqwest::Client` (`ClientBuilder::with_reqwest`) | pass | pass | pass | pass | pass |
| Cancel by dropping the stream | pass: the mock saw the connection close within 2 s (tested on the OpenAI shape; the transport path is the same for all) | ← same | ← same | ← same | ← same |

genai had no fails, so no provider needs a fallback today. The standing fallback for any provider that fails a live test is still a thin native call for that provider, behind the AI-07 `ModelClient` trait.

### Findings that change AI-07 and AI-09

1. **genai reads ambient env vars by default. Confirmed.**
   - A default `Client` sent `ANTHROPIC_API_KEY` from the environment.
   - So did a `Client` whose `AuthResolver` returns `Ok(None)`, because genai falls back to `default_auth`, which is `AuthData::from_env("ANTHROPIC_API_KEY")`. The same holds for `OPENAI_API_KEY`, `GEMINI_API_KEY` and the rest (`src/client/config.rs:230-256`).
   - Only an explicit `ServiceTarget {endpoint, auth, model}` passed to `exec_chat_stream` skips the resolver chain. With one, the mock received our key, not the env key.

   **Rule for AI-07:** always build a `ServiceTarget` from our key store and registry, and never pass a model-name string. (A `ServiceTargetResolver` that overwrites `auth` would also work, but genai still reads the env var first.) This is what decision G4 needs: a key only ever comes from our store, or from `CARTOGRAPH_AI_KEY`.
2. **genai's default `system-proxy` feature lets reqwest honour `HTTP(S)_PROXY`.** AI-09's outbound policy must therefore always supply its own client (`with_reqwest`, `.no_proxy()`, `redirect::Policy::none()`, a resolved-and-pinned address). The spike does exactly that. Building genai with `default-features = false` also drops `system-proxy`; we would then enable TLS on our own reqwest.
3. **The request is sent lazily.** `exec_chat_stream` returns before any request is made, and the request goes out on the first poll. This doesn't matter for the Ask loop, but a "test connection" button has to poll the stream.
4. **The "Exact"/"Likely" engine still needs native fields.** Anthropic citations and server tools are not in genai's common surface, so we would reach them through `with_extra_body` or a native call. G3 doesn't depend on them.

### keyring 4.2.0 (G4)

| Check | macOS 26 (this machine) | Windows | Linux |
|---|---|---|---|
| set, get, delete, then get gives `NoEntry` (service `cartograph-spike`) | **pass**, headless, no dialog | not testable here. `v1` feature uses `windows-native-keyring-store` (Credential Manager) | not testable here. `v1` uses **zbus Secret Service only**, with no keyutils fallback. With no Secret Service running, `Entry::new` gives `NoDefaultStore`, which is G4's case for the 0600 file fallback |
| Binary replaced by an upgrade (ad-hoc signed rebuild with different bytes) | **get: pass, no dialog. delete: FAIL** with `Invalid attempt to change the owner of this item` (errSecInvalidOwnerEdit). set (overwrite) passed. Only the original binary could delete the item | n/a | n/a |

**Delete fallback (named).** When delete fails with the owner error, AI-07's `KeyStore::delete` overwrites the item with an empty tombstone, treats an empty value as "no key", and tells the user that the old item stays in Keychain Access until they remove it. The real fix comes with signing (N6): a stable Developer ID designated requirement should keep the ACL valid across upgrades, which needs re-checking once a signed build exists.

keyring 4's own docs advise apps to depend on `keyring-core` plus the specific stores they want, not on `keyring`. AI-07 should do that, which also allows adding `linux-keyutils-keyring-store` as a Linux tier before the file fallback.

Testing with the item already deleted showed no leftovers: `security find-generic-password -s cartograph-spike` returns 44 (not found).

### ACP (G2, AI-23, risk R8): a reading, not run

- **Client side.** `agent_client_protocol::v1::schema::ClientCapabilities::default()` serialises to `{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false,"auth":{"terminal":false}}`. Every capability defaults to off, so a client can offer none (unit-tested).
- **Agent side: claude-agent-acp** (Apache-2.0, `src/v1/acp-agent.ts`). It advertises:
  - `loadSession: true`
  - `promptCapabilities {image, embeddedContext}`
  - `mcpCapabilities {http, sse, acp}`
  - `auth {logout, status}` and `providers {}`
- **Client capabilities don't turn the agent's own tools off.** Its built-in tools are the full `claude_code` preset, which reads and writes the disk and runs Bash itself, unless `session/new` carries `_meta.claudeCode.options.tools: []` or the legacy `_meta.disableBuiltInTools: true`.
- **It loads project settings from the working folder.** It sets `settingSources: ["user","project","local"]`, so it loads project settings and hooks from the cwd. G2's empty working folder is required, not optional.
- **Bypass mode is on by default** unless `_meta.claudeCode.options.allowDangerouslySkipPermissions: false` is sent. Cartograph must send it.
- **Permission asks still reach the client.** The adapter calls `session/request_permission` for its own tool use through the SDK's `canUseTool`, and rejecting every non-Cartograph request (G2) works there.
- **R8 sizing.** These switches are vendor `_meta` extensions, not ACP spec. Each agent needs its own switch and its own test (AI-23's "shipped per agent only after a spike proves its own tools are off" stands). Codex and Gemini CLI were not read.

### Licences (cargo metadata over the spike's full tree, 345 packages)

- genai, keyring, keyring-core and the native stores are MIT OR Apache-2.0. agent-client-protocol and its schema are Apache-2.0.
- No GPL, AGPL or non-commercial licence anywhere. The only LGPL mention is `r-efi` (`MIT OR Apache-2.0 OR LGPL-2.1-or-later`), where we can choose MIT.
- **HD-12's allow list will need these** (from the rustls, aws-lc, ICU and webpki chain): `Unicode-3.0`, `CDLA-Permissive-2.0` (webpki-root-certs), `ISC`, `BSD-3-Clause`, `Zlib`, `BSL-1.0`, `0BSD`, `MIT-0`, `CC0-1.0`, `Apache-2.0 WITH LLVM-exception`.
- `aws-lc-sys` (reqwest 0.13's default rustls provider) needs cmake and a C compiler at build time. HD-02's CI images must have them, or AI-07 should select the `ring` provider.

## INT-02 · Code2DFD and the labelled set

**Result: the belief is confirmed. All 17 applications are Java and Spring.** Code2DFD's ground truth can be turned into service-to-service labels with a kind, but not into operation labels. So it can't tell an exact link from a likely one on the same pair of services. **Recommendation for ruling N4:** take the recommended default. CI gates on the hand-labelled TypeScript, Go and Python set, which phase 0 has started. Code2DFD becomes a secondary benchmark at service level, run once Java adapters exist.

### What the dataset is

- **Name and source.** Decision H2's "17 Code2DFD apps" is the **microSecEnD** dataset: Schneider, Özen, Chen and Scandariato, *microSecEnD: A Dataset of Security-Enriched Dataflow Diagrams for Microservice Applications*, MSR 2023, doi 10.1109/MSR59073.2023.00030.
  - Dataset: `github.com/tuhh-softsec/microSecEnD`, pinned at `63653e09177b6d757eada4521f8bace8a8c20766` (2025-01-06). Its models were committed on 2023-01-26.
  - Extraction tool: `github.com/tuhh-softsec/code2DFD`, at `c65b4a081ed2ca1618319e5dabf9ecf590988059`. It's Python, and was not run.
- **Licence.** Both repositories ship the Apache-2.0 text, under a "Copyright 2021 Google LLC" header copied from a template.
- **Size.** 17 applications, 149 services, 374 information flows.
- **How the ground truth is written.** Each application comes in four forms: `<app>.json` (`services`, `external_entities`, `information_flows`), a PlantUML/Graphviz `.txt` file, a `.png`, and a `.py` file in their metamodel.
  - A flow is `{sender, receiver, stereotypes[], tagged_values{}}`.
  - Stereotypes are security and technology tags. Flow stereotypes, across all 374 flows: `restful_http` 358, `authenticated` and `authorized` 104 each, `load_balanced_link` 37, `circuit_breaker_link` 27, `jdbc` 16, `feign_connection` 14, `message_producer_kafka` 2, `message_consumer_kafka` 3, `message_producer_rabbitmq` 2, `message_consumer_rabbitmq` 2.
  - A `_traceability.json` file maps each node and edge to a GitHub `file#line`.
- **The applications are not pinned.** 843 of 860 traceability links point at a branch name (`master`, `main` or `with_database`), not a commit. Only 6 of the 17 applications link a specific commit.
  - `fetch.mjs` pins each application to that commit where one exists. Otherwise it uses the last commit on the modelled branch before 2023-01-26.
  - Some ground truth may already have drifted from the code.
  - Five applications keep their configuration in a separate repository (`*-config`, which the traceability files link to), so their ground truth spans two repositories.

### Languages as observed

Files counted with `git ls-tree -r` at the pinned commit: `bench/code2dfd/fetch.mjs`, with the raw output in `phase0/int02/results/code2dfd-languages.json`.

| App | Commit | Licence (GitHub) | Files | Java | Other source | Build | Flows (kafka/rabbit/jdbc) |
|---|---|---|---|---|---|---|---|
| anilallewar/microservices-basics-spring-boot | `ac0a249` | Apache-2.0 | 231 | 37 | Groovy 1, JavaScript 8 | Gradle | 29 (0/0/0) |
| apssouza22/java-microservice | `9172a90` | MIT | 240 | 117 | - | Maven | 34 (2/0/0) |
| callistaenterprise/blog-microservices | `1681bdb` | Apache-2.0 | 165 | 37 | - | Gradle | 42 (0/0/0) |
| ewolff/microservice | `99d9d07` | Apache-2.0 | 114 | 42 | - | Maven | 12 (0/0/0) |
| ewolff/microservice-kafka | `27f3a13` | Apache-2.0 | 95 | 52 | - | Maven | 11 (3/0/3) |
| fernandoabcampos/spring-netflix-oss-microservices | `8668c4d` | none stated | 73 | 26 | - | Maven | 23 (0/0/0) |
| georgwittberger/apache-spring-boot-microservice-example | `5b97bd9` | MIT | 41 | 15 | JavaScript 1 | Maven | 5 (0/0/0) |
| jferrater/Tap-And-Eat-MicroServices | `3ad20b8` | Apache-2.0 | 114 | 35 | - | Maven | 16 (0/0/0) |
| koushikkothagal/spring-boot-microservices-workshop | `8b01c6d` | none stated | 44 | 19 | - | Maven | 6 (0/0/0) |
| mdeket/spring-cloud-movie-recommendation | `5aa5ee9` | none stated | 93 | 37 | - | Maven | 17 (0/0/3) |
| mudigal-technologies/microservices-sample | `18c2976` | none stated | 155 | 30 | JavaScript 3, TypeScript 20 | Maven | 33 (0/4/2) |
| piomin/sample-spring-oauth2-microservices | `fd577d9` | none stated | 52 | 30 | - | Maven | 12 (0/0/2) |
| rohitghatol/spring-boot-microservices | `a3c9df9` | Apache-2.0 | 145 | 25 | JavaScript 8 | Gradle | 26 (0/0/2) |
| shabbirdwd53/Springboot-Microservice | `e0faba5` | none stated | 83 | 29 | - | Maven | 17 (0/0/0) |
| spring-petclinic/spring-petclinic-microservices | `ec7ea22` | Apache-2.0 | 176 | 46 | JavaScript 21 | Maven | 27 (0/0/0) |
| sqshq/piggymetrics | `fd5ee3c` | MIT | 204 | 97 | JavaScript 8 | Maven | 36 (0/0/4) |
| yidongnan/spring-cloud-netflix-example | `3b86bf0` | MIT | 65 | 11 | - | Gradle | 28 (0/0/0) |

- **Totals:** Java 685 files, JavaScript 49, TypeScript 20 (one Angular front end), Groovy 1. All 17 are Spring Boot or Spring Cloud.
- **Frameworks**, from the dataset's tags and a `git grep` of the pinned trees (`results/java-patterns.json`):
  - Eureka in 15 of 17 applications.
  - Spring MVC mappings: 224 matches. RestTemplate or WebClient: 114. JPA, JDBC or Mongo repositories: 64.
  - Zuul in 10 applications. Feign: 16 matches.
  - Kafka in 1 application, RabbitMQ in 1, Spring Cloud Stream in 1.
  - Most of it is the Netflix OSS era: Hystrix, Ribbon, Zuul and Turbine.
- **Licences.** 7 of the 17 state no licence. They can be cloned for evaluation, but can't be vendored as fixtures.

### Converting the labels

| Our label | From microSecEnD | Possible? |
|---|---|---|
| service A → service B | `sender` and `receiver` | **Yes, with direction rules.** Flows are drawn in the direction data moves, not the direction of dependency. For example, `config -> account_service` means account_service depends on config, and `rabbitmq -> turbine` follows the same pattern. Each stereotype needs its own reversal rule. |
| kind | `restful_http` → http; `message_*_kafka` and `_rabbitmq` → event; `jdbc` and database nodes → db | **Yes.** |
| op (method and path, topic, table) | Absent. Flow `tagged_values` carry only a protocol, a load balancer or a circuit breaker, except 5 event flows that name a topic or exchange. Endpoints are listed per service, not per flow. | **No.** Only the 5 topics. |

- **Infrastructure flows.** 231 of the 374 flows touch infrastructure: config server, Eureka, monitoring, Turbine and gateways. These are framework wiring that comes from YAML (`spring.cloud.config.uri`, `eureka.client.serviceUrl`, `zuul.routes`), not calls in code.
- **Scoring rule.** Either score them with a Spring configuration reader, or leave them out of both sides.

### Can it measure exact, likely and guessed?

Only partly, and only at the level of a service pair and a kind.

- The dataset labels flows, not confidence. Precision for a level would be: of the links the indexer marks at that level, the share whose service pair and kind match a flow.
- **The catch is that ops aren't labelled.** A link at "exact" with the wrong operation still matches the right pair of services and counts as correct. That overstates precision, and most at the exact level, where H2's definition rests on the operation matching at both ends.
- Few labels per level: only 9 event flows and 16 jdbc flows in all.
- Most HTTP calls go through Eureka service ids or Feign names, which resolve through `spring.application.name`. Under H2 that is mostly **likely**, so the exact level would be measured on very few links.

### Estimate for the Java .scm adapters

This assumes tree-sitter-java (MIT), vendored and pinned. There are about 20 to 25 query patterns:

| Adapter | Patterns | Notes |
|---|---|---|
| Spring MVC routes | ~6 | Class-level `@RequestMapping` prefix plus method-level `@Get/Post/Put/Delete/PatchMapping`, and `@RequestMapping(method=…)`. The prefix join is done in Rust, not in the query. |
| Feign clients | ~3 | `@FeignClient(name/value/url)` on an interface, plus its method mappings, become client operations. |
| RestTemplate and WebClient | ~4 | `getForObject`, `postForEntity`, `exchange` and `retrieve()` with a literal or concatenated `http://<service-id>/path`. |
| Kafka | ~3 | `@KafkaListener(topics=…)` and `KafkaTemplate.send(topic, …)`. |
| RabbitMQ | ~4 | `@RabbitListener(queues=…)`, `RabbitTemplate.convertAndSend(exchange, key, …)`, and `Queue`/`Binding` beans. |
| Spring Cloud Stream (legacy) | ~2 | `@StreamListener` and `@EnableBinding`. Optional, since it appears in one application. |
| DB | ~3 | `@Entity`/`@Table`, `*Repository` interfaces, and `spring.datasource.url`. |

**Size:** the adapters alone are **M**. Counting the Spring configuration resolver they depend on, the item is **L**. That resolver handles `application.yml`/`bootstrap.yml` and `.properties`, profiles, `${…}` placeholders, `spring.application.name` as Eureka ids, and `zuul.routes`.

**Risks:**
- Placeholders resolved from remote config repositories.
- Spring Boot 1.x Netflix idioms that new code no longer uses.
- The direction mapping from data flow to dependency.
- Unpinned ground truth.

### The labelled set (started)

- **Location:** `bench/labelled-set/`, holding the schema (`labels.schema.md`), the candidate list (`README.md`) and the first file, `robot-shop.yaml`.
- **robot-shop.yaml.** `instana/robot-shop` at `55292e2199f2fb00a165b1f7d3045fe7f8922038`, Apache-2.0, written in JS, Python, Go, Java and PHP. It holds 21 links, each verified line by line with `git show`:
  - 6 code HTTP links with method and path
  - 6 nginx gateway links, marked `requires: nginx-adapter`
  - 1 RabbitMQ event link, plus 2 channel edges
  - 8 db links
  - Expected levels: 1 exact, 20 likely. Most real calls take their host from an env default, which is H2's "likely".
- **Candidates for the rest, all checked with the GitHub API** (`results/labelled-candidates.json`):

  | Repository | Commit | Licence | Languages |
  |---|---|---|---|
  | GoogleCloudPlatform/microservices-demo | `c7cb6d5` | Apache-2.0 | Go, Python, C#, Node; gRPC |
  | open-telemetry/opentelemetry-demo | `fc7f2a6` | Apache-2.0 | TypeScript first; Kafka |
  | GoogleCloudPlatform/bank-of-anthos | `db35fea` | Apache-2.0 | Python, Java; REST, Postgres |
  | thangchung/go-coffeeshop | `22d3319` | MIT | Go; gRPC, RabbitMQ |

  - **Open point:** gRPC has no wire kind. R28 lists imports, calls, http, event and db. Two candidates rely on gRPC, so XS-07 should rule whether gRPC is `http` with op `grpc <Service>/<Method>`.
- **Target before the gate:** at least 150 links and at least 30 per emitted level.

### Decisions this changes

- **H2 (holds, but the benchmark changes).** Precision per level is measured on the hand-labelled TypeScript, Go and Python set. Code2DFD is reported later, at service-pair and kind level only, once the Java adapters (M, or L with the config resolver) land in XS-07 or later.
- **N4.** Adopt the recommended default. Also, don't show "Exact" for an evidence class until the labelled set holds at least 30 links at that level.

Raw files:
- `phase0/int02/results/code2dfd-languages.json`
- `phase0/int02/results/java-patterns.json`, where the `rest_controller` column reads 0 because of a regex quirk; ignore it
- `phase0/int02/results/labelled-candidates.json`

Clones total 75 MB, under `phase0/int02/`.

## Setup items

### HD-01 · One lockfile, pinned toolchains

- `rust-toolchain.toml` pins Rust 1.100.0 with clippy and rustfmt. `Cargo.lock` is committed; CI builds with `--locked`.
- `package.json` gains `packageManager: pnpm@11.23.0+sha512.…` and `engines` `node >=22`, `pnpm >=11`. `.node-version` holds `22` and CI reads it.
- **npm fails, naming pnpm.** `engines.npm` is `"please-use-pnpm"` and `.npmrc` sets `engine-strict=true`, so `npm install` stops with `EBADENGINE … Required: {"npm":"please-use-pnpm"}` before it writes anything (tested in `phase0/npmguard/`). A `preinstall` guard (`scripts/only-pnpm.mjs`) covers yarn. The guard alone wasn't enough: npm 10 runs the root `preinstall` after it has already written `node_modules` and `package-lock.json`.
- CI's `lockfiles` job fails if `package-lock.json`, `yarn.lock` or `npm-shrinkwrap.json` is tracked, and runs `npm install` expecting it to fail.
- Every action in `.github/workflows/ci.yml` is pinned to a commit SHA with the tag in a comment.
- No stray `package-lock.json` was present in this worktree.

### HD-02 · Test harness and CI skeleton

`.github/workflows/ci.yml` has four jobs:

| Job | Runs on | Steps |
| --- | --- | --- |
| `lockfiles` | Linux | the HD-01 checks |
| `rust` | Linux, macOS, Windows | `cargo fmt --check`, `clippy -D warnings`, `cargo build --locked`, `cargo nextest run` (unit, insta snapshot and proptest tests, plus the contract check), doc tests; on Linux also fmt, clippy and tests for the `bench/` workspace |
| `web` | Linux, macOS, Windows | `pnpm install --frozen-lockfile`, `tsc --noEmit`, `pnpm check` (`lib/workspace.check.ts`), the generated-types diff (Linux), `next build` |
| `supply-chain` | Linux | HD-12 below |

Run locally: `cargo build`, `cargo test` (11 tests), `cargo clippy -D warnings`, `cargo fmt --check`, `pnpm exec tsc --noEmit`, `pnpm check` and `next build` all pass. The workflow itself has not run on GitHub yet. "Blocks merge" also needs branch protection on `main` requiring these checks, which is a repository setting, not a file.

Not done here: Playwright, eslint and `cargo llvm-cov` (no DOM tests or lint config exist yet).

### IDX-01 · Rust workspace

`Cargo.toml` at the root, crates under `crates/`:

| Crate | Now |
| --- | --- |
| `cartograph-core` | ids (`RepoId`, `NodeId`, `LinkId`, `Seq`, `Oid`), the error contract (`Explain`: what, why, try and `exit_code()` mapping to 0/1/2/3 per E1 and E2), the data folder (`~/.cartograph`, `CARTOGRAPH_HOME`, Linux `XDG_DATA_HOME`, synced folders refused, per B1), logging (tracing, a JSON file under `<data>/logs/`, stderr only with `-v`) |
| `cartograph-api` | the INT-01 contract (below) |
| `cartograph-discovery`, `-git`, `-parse`, `-facts`, `-store`, `-index`, `-query` | empty, with the dependency edges from the plan fixed (store and facts don't depend on each other; query reads store and api) |
| `cartograph-cli` | the `cartograph` binary: clap, logging, the error renderer, exit codes, and `cartograph paths` |

- Tests: the exit-code mapping, an insta snapshot of one rendered three-line error, proptest on `Oid::parse`, and the data-folder rules.
- The release profile has `lto = "thin"` and `codegen-units = 1`. `panic = "abort"` waits for IDX-13.
- Deferred: tracing-appender rotates by time, not size. So "7 files of 10 MB" is approximated as daily files with 7 kept.
- Deferred: the server, MCP, agents and AI crates arrive with their items.

### HD-12 · Supply chain

- **`deny.toml`.** It holds a licence allow list (MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0, Unicode-3.0, CDLA-Permissive-2.0 and the like). Anything else fails, which covers GPL, LGPL, AGPL, PolyForm, SSPL, BUSL and every non-commercial licence.
  - Banned crates: `git2` and `libgit2-sys` (B7), `forceatlas2` (I2), and `reqwest` and `ureq`, so no network client is in the product tree until the Ask crate is added as their only wrapper.
  - Advisories, yanked crates and unknown registries or git sources also fail.
  - Result today: `advisories ok, bans ok, licenses ok, sources ok`.
- **Canary.** Adding `forceatlas2 = "=0.8.0"` (AGPL-3.0-only) to a copy of the workspace fails twice: `error[rejected] … AGPL-3.0-only` and `error[banned]`. The output is in `phase0/results/hd12/canary-agpl.txt`.
- **`cargo audit --deny warnings`**: 118 crates, no advisories.
- **`pnpm audit --prod`**: no known vulnerabilities.
- **npm licences.** `scripts/check-licenses.mjs` checks `pnpm licenses ls --prod` against the same allow list it reads from `deny.toml`. It found two outside the list, both pulled in by `next`, and allows them as named, build-time-only exceptions:
  - `@img/sharp-libvips-*`, LGPL-3.0: next/image's optimiser, which a static export never runs.
  - `caniuse-lite`, CC-BY-4.0: browser data for the build tooling.
  - FE-01 should confirm neither ends up in the exported bundle.
- **pnpm install policy** (`pnpm-workspace.yaml`):
  - `allowBuilds: {}`, so no dependency runs install scripts. The current tree needs none: sharp and tailwind's oxide ship prebuilt binaries.
  - `minimumReleaseAge: 10080`, 7 days. Seven packages already in the lockfile were younger than that and are listed as exact-version exceptions to delete once they age.
  - Note: pnpm 11 renamed `onlyBuiltDependencies` to `allowBuilds`.
- **SBOM.**
  - `cargo cyclonedx` (spec 1.6) lists the binary's crates: 71 components for `cartograph-cli`.
  - `pnpm sbom --sbom-format cyclonedx --prod` lists 125 npm packages. It's built into pnpm 11, so `@cyclonedx/cyclonedx-npm` isn't needed.
  - CI uploads both as an artifact. Merging them into one file per release artefact and generating THIRD-PARTY-NOTICES belong to the release job (HD-13).
  - Output: `phase0/results/hd12/sbom/`.
- **Dependabot.** `.github/dependabot.yml` groups updates weekly for cargo, npm and actions.
- **Needed when AI-07 lands.** reqwest 0.13's default TLS backend, `aws-lc-sys`, needs cmake and a C compiler. GitHub's runners have both; otherwise choose the `ring` provider.

### INT-01 · The HTTP API contract

- **Where it lives.** The types are in `crates/cartograph-api`, written with schemars:
  - `types.rs`: every wire type
  - `events.rs`: the two event streams
  - `routes.rs`: the route table, with each route's kind (read, write or stream), query parameters, body, response and owner
- **Generating it.**
  - `cargo run -p cartograph-api --bin gen-contract` writes `crates/cartograph-api/openapi.json` (OpenAPI 3.1) and `schema.json` (draft-07).
  - `pnpm gen:types` turns `schema.json` into `lib/api/types.gen.ts` with json-schema-to-typescript 16.0.0, as the "type generator" row of the rulings asks. That gives 127 schemas.
  - `lib/api/index.ts` adds the generic `Envelope<T>`, which JSON Schema can't express.
- **Checks.**
  - `cargo test` fails when either generated file is stale.
  - CI fails when `types.gen.ts` differs from a fresh generation.
  - Tests assert:
    - every route is under `/api/v1` (except `/api/version`)
    - GET and DELETE never take a body
    - every PUT and DELETE is a write
    - there are exactly 21 writes, so changing the write list fails a test
    - no camelCase or kebab-case names appear on the wire
- **Coverage.** The table has 88 operations on 76 paths: 65 reads, 21 writes and 2 event streams (`GET /api/v1/events`, and Ask reattach); `POST …/ask/runs` also answers with a stream. Every row of the Interfaces section has a route.
  - Six POSTs that change nothing are marked read: `…/impact`, `…/config/preview`, `/discover`, `/agents/preview`, `…/models` and `…/test`. The Interfaces table marks their rows "write" as a group; the closed write list is the 21.
  - Routes from the analysts' plans that the table replaces are listed in `routes::REPLACED`:
    - `POST /api/session` is dropped (R4)
    - `/api/w/:ws/sync` becomes `POST …/jobs`
    - `/api/fs/dirs` becomes `/api/v1/fs/list` (R25)
    - `…/nodes/:id/summary?t=` becomes `…/nodes/{id}?include=summary&at=`
    - `/api/files/{id}/symbols` becomes `…/nodes/{id}/symbols`
    - `/api/v1/ai/migrate` becomes the key PUT
    - `/api/v1/ai/settings` becomes `selection` plus `privacy`
    - the per-workspace stream becomes the global one (R15)
    - `update --check` is dropped (R16)

**Reconciling `lib/model.ts` and `lib/workspace.ts`** (snake_case on the wire, R5). What changes for the front end when it moves to the generated types:

| TypeScript today | On the wire | Why |
| --- | --- | --- |
| `Repo.indexedCommit`, `Freshness.indexVersion`, `builtAt`, `keySaved`, `addedLinks`, `toolCalls`, `fanIn` | `indexed_commit`, `format_version`, `built_at`, `key_saved`, `added_links`, `tool_calls`, `fan_in` | R5 |
| `IndexState "needs-migrate"`, `Evidence.kind "name-match"` | `needs_migrate`, `name_match` | snake_case for enum values too |
| `Repo.behind` | `behind` capped at 1,000, plus `behind_capped`; `dirty` may be null | IDX-15, and the `dirty` open question |
| `Freshness.repos[] {name, behind}`, `error {what, action}` | `{repo, name, behind, behind_capped}`, `Problem {what, why, try}` | E1's three parts everywhere; `try` is the wire name |
| `Link {from, to, type, added, removed?, confidence?}` | `Link {id, from, to, kind, added: {sha, seq}, removed, confidence}` | R28 says `kind`. Commits carry their timeline step. Confidence is always sent; the server sends `level` (R20), so `levelOf` in TypeScript goes |
| `Node.service ""` for the root, `lang "TS"` | `service: null`, `lang: "ts"` | |
| `Correction {target, value}` | `Correction {key: LinkKey, to, confidence, reason, stale}` | H3 and R6: the nested key object (R28) |
| `Spec` | `Contract`, served from `…/contracts` | named after its route |
| `ToolCall {args: string, result: string}`, `AskTurn.answer: string` | `ToolCall {call_id, args: object, rows, handles, ms}`, `answer: Sentence[]` with citations by handle | G5 and the run events |
| `AppearanceSettings` | `Prefs.appearance`, beside keymap, pins and Ask placement | R1: preferences move to the server |
| `getMcpTools()` | `McpTool` gains `ask` (one of the five Ask tools) | F1 |

`lib/source.ts`'s getters map one-to-one onto read routes:

| Getter | Route |
| --- | --- |
| `getWorkspaces` | `/workspaces` |
| `getRepos` | `…/repos` |
| `getFreshness` | `…/freshness` |
| `getContracts` | `…/contracts` |
| `getEntrypoints` | `…/entrypoints` |
| `getConcepts` | `…/concepts` |
| `getCoverage` | `…/coverage` |
| `getCorrections` | `…/corrections` |
| `getOwners` | `…/owners` |
| `getActivity` | `/activity` |
| `getCommits` | `…/commits` |
| `getCommitImpact` | `…/commits/{sha}/impact` |
| `getMcpTools` | `/mcp/tools` |
| `getProviders` | `/ai/providers` |
| `getAskHistory` | `…/ask/threads` |
| `search` | `…/search` |

`lib/model.ts` and `lib/source.ts` are unchanged in this phase. The switch is FE-02's.
