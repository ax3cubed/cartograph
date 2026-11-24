# Contributing to Cartograph

Thanks for considering a contribution. Cartograph is an early prototype, so there is a lot of room to shape it. This guide covers setup, the checks to run, and the conventions the project follows.

By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Set up

You need Node 22 or newer and pnpm 11. `pnpm check` uses Node's built-in type stripping, which needs Node 22.6 or newer.

```sh
git clone https://github.com/ax3cubed/cartograph.git
cd cartograph
pnpm install
pnpm dev
```

Open http://localhost:3000. The app runs on the bundled demo workspace, so there is nothing else to configure.

Use pnpm for everything. `pnpm-lock.yaml` is the lockfile. Commit changes to it when dependencies change, and do not commit lockfiles from other package managers.

## Before you open a pull request

Run these three commands. CI runs the same ones.

```sh
pnpm exec tsc --noEmit
pnpm check
pnpm build
```

| Command | What it checks |
| --- | --- |
| `pnpm exec tsc --noEmit` | Types across the project |
| `pnpm check` | Runs `lib/workspace.check.ts`: assertions over the demo data, the Ask engine and the AI helpers |
| `pnpm build` | The production build succeeds |

If you change behaviour in `lib/`, add or update an assertion in `lib/workspace.check.ts`.

### End-to-end and visual tests

`pnpm test:e2e` builds the app, the `cartograph` binary that embeds it and the golden fixture, then runs Playwright against the real binary in Chromium, Firefox and WebKit: each route's smoke test and axe scan, and the journeys in `e2e/journeys.spec.ts`. Any browser request off 127.0.0.1 is aborted and fails the test. Run `pnpm exec playwright install chromium firefox webkit` once first. Add `--no-build` to reuse `out/` and `target/debug`, and pass Playwright arguments after it (`pnpm test:e2e --no-build -g "keyboard"`).

Screenshots depend on the host's fonts and rasteriser, so they are only taken in one pinned image, `mcr.microsoft.com/playwright` for linux/amd64 (the digest is in `scripts/e2e.mjs`). On any other browser the visual tests are skipped, not failed. With Docker running:

```sh
pnpm test:e2e --visual            # compare with the baselines in e2e/__screenshots__
pnpm test:e2e --visual --update   # rewrite them after an intended change; review and commit the PNGs
```

CI runs the same image. A failed comparison uploads the expected, actual and diff images as the `e2e-results` artifact. When you bump `@playwright/test`, change the image tag and digest in `scripts/e2e.mjs` to the same version and update the baselines.

## Branches

Branch from `main`. Never push to `main`: every change lands through a pull request.

Name the branch `<type>/<plan-id>-<description>`, or `<type>/<description>` when the work has no item in the implementation plan (`docs/architecture.html`):

```
feat/idx-03-store-schema
fix/timeline-knob-offset
docs/contributing-commits
```

- `<type>` is one of the commit types below.
- `<plan-id>` is the plan item in lowercase, such as `idx-03`, `srv-05` or `fe-02`.
- The description is a few lowercase words joined by hyphens.
- At most 60 characters. `dependabot/…` and `release/vX.Y.Z` are named by tools and are allowed as they are.

The `pre-push` hook and CI check the name. To fix one, run `git branch -m <new-name>`.

## Commits

Commit messages follow [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/). The `commit-msg` hook checks each commit, and the `pre-push` hook checks every commit on the branch again before it leaves your machine. CI does not check them.

```
<type>(<scope>): <subject>

<body: what changed and why, wrapped at 100 characters>

Refs: IDX-03
```

| Part | Rule |
| --- | --- |
| Type | `feat` new behaviour · `fix` a bug fix · `perf` faster, same behaviour · `refactor` neither · `docs` · `test` · `build` dependencies and tooling · `ci` · `chore` housekeeping · `revert` · `style` formatting only |
| Scope | Optional. When present, one of: `indexer` `store` `git` `parse` `query` `server` `api` `cli` `mcp` `ai` · `ui` `ask` `map` `timeline` `views` `settings` `shell` `design` · `site` `docs` `architecture` `research` `bench` `deps` `ci` `github` `release` `security` |
| Subject | Imperative and lowercase: "add the store schema", not "Added" or "Adds". No full stop. The whole first line is at most 72 characters |
| Body | Optional. Says why, not how. A blank line before it |
| Footer | `Refs: IDX-03` for the plan items the commit works on, `Closes #123` for an issue, and `BREAKING CHANGE: …` (or `!` after the type) for a change that breaks a file format, the API or the CLI |

Examples:

```
feat(store): add schema v1 with application_id and user_version
fix(timeline): put the scrubber knob on the cursor line in month view
perf(query): hold link intervals in memory per index generation
build(deps): pin oxc_resolver to 11.6
feat(api)!: move graph slices to snake_case
```

The hooks are installed by `pnpm install`. If they're missing, run `pnpm run prepare`. Check a branch's commits by hand with `pnpm lint:commits`.

## Pull requests

- **Squash merge only.** The pull request title becomes the one commit on `main`, so write it as a Conventional Commit, the commit you want in the history. Nothing checks the title, so get it right before you merge.
- **Base every pull request on `main`.** Don't stack one pull request on another's branch. GitHub only retargets a stacked pull request when its base branch is deleted, so it can merge into the wrong branch. If you need work from an open pull request, branch from that branch and still target `main`. The diff shrinks once the first one merges.
- Keep each pull request small and focused on one change, ideally one plan item. Two unrelated fixes are two pull requests.
- Fill in the pull request template: what changed, why, the plan items, and how you checked it.
- Add screenshots for any change to the UI.
- Add a line to `CHANGELOG.md` under `## [Unreleased]` for anything a user would notice.
- For a large change, open an issue first so the approach can be agreed before you write the code.

## Design rules

The look and behaviour of the app are pinned. Read these two files before changing anything a user can see:

- [DESIGN.md](DESIGN.md) sets the visual language: the dark navy ground, one colour per service, electric blue for selection and the scrubber, lime for added and coral for removed, Sora for UI text, JetBrains Mono for paths and commit SHAs, and the morph between layouts as the one authored motion.
- [PRODUCT.md](PRODUCT.md) sets the purpose, the two kinds of user and the terminology (node, link, fan-in, fan-out, cross-service, churn).

Changes must stay within both. Use the terms from PRODUCT.md in the UI and in code. If you think a rule should change, open an issue and make the case there before sending code.

## Next.js version

The project is on Next.js 16, which differs from earlier versions in APIs and conventions. `AGENTS.md` points to the guides shipped in `node_modules/next/dist/docs/`. Check them before relying on behaviour you remember from an older release.

## The `ponytail:` convention

Cartograph prefers the simplest thing that works today. When code takes a deliberate shortcut, it is marked with a comment that starts with `ponytail:`. The comment names the ceiling of the shortcut and the upgrade path. An example from `lib/workspace.ts`:

```ts
// ponytail: hand-authored data, swap for a generated JSON when the indexer exists.
```

Follow the same convention:

- If you simplify on purpose, add a `ponytail:` comment that says where the simple version stops being enough and what replaces it.
- If your change removes a shortcut, remove its comment.
- `grep -rn "ponytail:" app components lib` lists the known shortcuts. It is a good place to look for work.

## Where help is most wanted

The [roadmap](docs/ROADMAP.md) lists seven stages. Each one is open for contribution:

1. Core indexer (Rust CLI, git history to one SQLite index, TypeScript/JavaScript imports first)
2. Breadth (Go, Python and PHP; rename lineage; several repositories; incremental updates)
3. Real-data UI (the app reads the index; level of detail; WebGL)
4. Cross-service links (HTTP routes, events and tables matched across services)
5. Symbols and calls
6. AI (provider registry, tool-calling Ask agent, MCP server)
7. Hardening (fuzzing, fault injection, performance gates)

Stage 1 unblocks most of the rest, so help there has the widest effect. Bug reports and fixes for the current prototype are welcome at any time.

## Releases

- Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the version is 0.x, a minor release may contain breaking changes.
- `CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Pull requests add their entries under `## [Unreleased]`.
- At release time the maintainer moves the Unreleased entries under a new version heading with the release date, updates the link references at the bottom of the file, and tags the commit `vX.Y.Z`.

## Reporting bugs and security issues

Use the issue forms for bugs and feature requests. Do not open a public issue for a vulnerability. Follow [SECURITY.md](SECURITY.md).

## Licence

Cartograph is released under the [MIT licence](LICENSE). Contributions you submit are licensed under the same terms.
