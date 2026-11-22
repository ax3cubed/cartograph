## What changed

<!-- A short description of the change.
     The title becomes the squash commit on main, so write it as a Conventional Commit:
     feat(store): add schema v1 with application_id and user_version -->

## Why

<!-- The problem this solves. Link the issue if there is one, for example "Closes #123". -->

Plan items: <!-- for example IDX-03, SRV-05; or "none" -->

## How it was checked

- [ ] `cargo test --workspace` (Rust changes)
- [ ] `pnpm exec tsc --noEmit`
- [ ] `pnpm check`
- [ ] `pnpm build`

<!-- Anything else you did to check it: steps you clicked through, new assertions in lib/workspace.check.ts. -->

## Screenshots

<!-- Required for UI changes. Before and after where that helps. Delete this section otherwise. -->

## Checklist

- [ ] `CHANGELOG.md` is updated under `## [Unreleased]`, or the change is not user-facing
- [ ] The change is small and covers one thing
- [ ] UI changes follow `DESIGN.md` and use the terms in `PRODUCT.md`
- [ ] Deliberate simplifications carry a `ponytail:` comment naming the ceiling and the upgrade path
