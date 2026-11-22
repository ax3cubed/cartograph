#!/usr/bin/env node
// Branch names follow <type>/<optional plan id>-<short-kebab-description>, for example
// feat/idx-03-store-schema or fix/timeline-knob-offset. Run by the lefthook pre-push hook and by CI
// ("Conventions") on every pull request. See CONTRIBUTING.md, "Branches".
import { execFileSync } from "node:child_process";

const TYPES = "feat|fix|perf|refactor|docs|test|build|ci|chore|revert|style";
const RULE = new RegExp(`^(${TYPES})/([a-z]{2,4}-\\d{2}-)?[a-z0-9]+(-[a-z0-9]+)*$`);
// Branches other tools name for us.
const ALLOWED = [/^dependabot\//, /^release\/v\d+\.\d+\.\d+$/];

const name =
  process.argv[2] ||
  process.env.GITHUB_HEAD_REF ||
  execFileSync("git", ["rev-parse", "--abbrev-ref", "HEAD"], { encoding: "utf8" }).trim();

if (name === "main") {
  console.error("✗ Don't push to main. Branch off main and open a pull request.");
  process.exit(1);
}
if (ALLOWED.some((r) => r.test(name))) process.exit(0);
if (name.length > 60 || !RULE.test(name)) {
  console.error(`✗ Branch "${name}" doesn't follow the naming rule.
  Format:   <type>/<plan-id>-<description>   or   <type>/<description>
  Types:    ${TYPES.replaceAll("|", ", ")}
  Examples: feat/idx-03-store-schema   fix/timeline-knob-offset   docs/contributing-commits
  Lowercase letters, digits and hyphens, at most 60 characters.
  Rename:   git branch -m <new-name>`);
  process.exit(1);
}
