// Commit messages follow Conventional Commits 1.0.0, enforced locally (lefthook commit-msg) and on
// every pull request (CI "Conventions"). The PR title is linted with the same rules, because it
// becomes the squash commit on main. See CONTRIBUTING.md, "Commits".
export default {
  extends: ["@commitlint/config-conventional"],
  // Dependabot writes "build(deps): Bump x from 1 to 2"; its capital B is the only rule it breaks.
  ignores: [(message) => /^(build|ci)\(deps\): bump /i.test(message)],
  rules: {
    "type-enum": [2, "always", ["feat", "fix", "perf", "refactor", "docs", "test", "build", "ci", "chore", "revert", "style"]],
    // Scope is optional; when present it names the area, from this list.
    "scope-enum": [2, "always", [
      // the Rust binary, by crate and concern
      "indexer", "store", "git", "parse", "query", "server", "api", "cli", "mcp", "ai",
      // the app
      "ui", "ask", "map", "timeline", "views", "settings", "shell", "design",
      // everything else
      "site", "docs", "architecture", "research", "bench", "deps", "ci", "github", "release", "security",
    ]],
    "scope-case": [2, "always", "kebab-case"],
    "subject-case": [2, "never", ["sentence-case", "start-case", "pascal-case", "upper-case"]],
    "subject-full-stop": [2, "never", "."],
    "header-max-length": [2, "always", 72],
    "body-leading-blank": [2, "always"],
    "body-max-line-length": [2, "always", 100],
    "footer-leading-blank": [2, "always"],
    "footer-max-line-length": [2, "always", 100],
  },
};
