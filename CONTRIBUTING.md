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
| `pnpm check` | Runs `lib/workspace.check.ts`: assertions over the demo data and the Ask engine |
| `pnpm build` | The production build succeeds |

If you change behaviour in `lib/`, add or update an assertion in `lib/workspace.check.ts`.

## Branches

Branch from `main` and name the branch after the kind of change:

| Prefix | Use for |
| --- | --- |
| `feat/…` | New behaviour |
| `fix/…` | Bug fixes |
| `docs/…` | Documentation only |
| `chore/…` | Tooling, dependencies, housekeeping |

## Commits

Write commit messages in the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) format:

```
feat(ask): answer "what changed" for a single file
fix(graph): keep pinned nodes in place during the morph
docs: explain the link interval model
```

## Pull requests

- Keep each pull request small and focused on one change. Two unrelated fixes are two pull requests.
- Fill in the pull request template: what changed, why, and how you checked it.
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
