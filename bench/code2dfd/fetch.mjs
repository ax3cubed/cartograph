// INT-02: fetch the microSecEnD dataset (the Code2DFD ground truth) and the applications it models,
// then report per application: observed languages (by file extension at the pinned commit), build
// files, the dataset's own technology tags, and the shape of its ground truth.
//
//   node bench/code2dfd/fetch.mjs [out-dir]        default: $TMPDIR/cartograph-code2dfd
//
// Safety: every repository is untrusted. Clones are bare and blobless (`--filter=blob:none`), so no
// working tree is ever checked out, and only `git ls-tree` reads them. Nothing is built or executed,
// hooks are disabled, and no credential prompt can appear. Re-runs reuse the clones (cached).
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const DATASET = "https://github.com/tuhh-softsec/microSecEnD";
const DATASET_COMMIT = "63653e09177b6d757eada4521f8bace8a8c20766"; // main, 2025-01-06
// The dataset does not pin the applications. Its models were committed on 2023-01-26, so each app is
// pinned to the last commit on the modelled branch before that date, unless its traceability file
// links a specific commit (then that commit wins).
const MODELLED_BEFORE = "2023-01-26";

const out = process.argv[2] ?? join(tmpdir(), "cartograph-code2dfd");
mkdirSync(out, { recursive: true });
const env = { ...process.env, GIT_TERMINAL_PROMPT: "0", GIT_CONFIG_NOSYSTEM: "1" };
const git = (...args) =>
  execFileSync("git", ["-c", "core.hooksPath=/dev/null", "-c", "protocol.file.allow=never", ...args], {
    env, encoding: "utf8", maxBuffer: 1 << 28, stdio: ["ignore", "pipe", "pipe"],
  }).trim();

// 1. The dataset itself (small, so a normal blobless clone with a checkout of its own JSON files).
const ds = join(out, "microSecEnD");
if (!existsSync(ds)) git("clone", "--quiet", "--filter=blob:none", "--no-checkout", DATASET, ds);
git("-C", ds, "fetch", "--quiet", "origin", DATASET_COMMIT);
git("-C", ds, "-c", "advice.detachedHead=false", "checkout", "--quiet", DATASET_COMMIT, "--", "dataset");
const root = join(ds, "dataset");
const meta = JSON.parse(readFileSync(join(root, "dataset.json"), "utf8"));

const LANG = {
  java: "Java", kt: "Kotlin", groovy: "Groovy", scala: "Scala", ts: "TypeScript", tsx: "TypeScript",
  js: "JavaScript", jsx: "JavaScript", mjs: "JavaScript", go: "Go", py: "Python", cs: "C#", rb: "Ruby", php: "PHP",
};
const BUILD = /(^|\/)(pom\.xml|build\.gradle(\.kts)?|package\.json|go\.mod|requirements\.txt|pyproject\.toml|Dockerfile|docker-compose\.ya?ml)$/;

const results = [];
for (const app of readdirSync(root).filter((d) => meta[d]).sort()) {
  const m = meta[app];
  const slug = m.slug;
  const trace = readFileSync(join(root, app, `${app}_traceability.json`), "utf8");
  const shas = [...trace.matchAll(new RegExp(`github\\.com/${slug}/blob/([0-9a-f]{40})/`, "gi"))].map((x) => x[1]);
  const branch = m.branch ?? m.data?.default_branch ?? "master";
  const dir = join(out, "apps", `${app}.git`);
  let commit = null, error = null;
  try {
    if (!existsSync(dir)) git("clone", "--quiet", "--bare", "--filter=blob:none", `https://github.com/${slug}`, dir);
    const before = (ref) => git("-C", dir, "rev-list", "-1", `--before=${MODELLED_BEFORE}`, ref);
    // A branch renamed since (master -> main) falls back to the remote's default branch.
    if (shas[0]) commit = shas[0];
    else { try { commit = before(branch); } catch { commit = before("HEAD"); } }
  } catch (e) { error = String(e.stderr || e.message).split("\n")[0]; }

  const langs = {}, builds = {};
  let files = 0;
  if (commit) {
    for (const p of git("-C", dir, "ls-tree", "-r", "--name-only", commit).split("\n")) {
      files++;
      const ext = p.includes(".") ? p.slice(p.lastIndexOf(".") + 1).toLowerCase() : "";
      if (LANG[ext]) langs[LANG[ext]] = (langs[LANG[ext]] ?? 0) + 1;
      const b = BUILD.exec(p); if (b) builds[b[2]] = (builds[b[2]] ?? 0) + 1;
    }
  }
  const model = JSON.parse(readFileSync(join(root, app, `${app}.json`), "utf8"));
  const flowKinds = {};
  for (const f of model.information_flows) for (const s of f.stereotypes) flowKinds[s] = (flowKinds[s] ?? 0) + 1;
  results.push({
    app, slug, branch, commit, commit_source: shas[0] ? "traceability" : `last before ${MODELLED_BEFORE}`, error,
    github_language: m.data?.language ?? null, licence: m.data?.license?.spdx_id ?? null, tech: m.tech,
    files, languages: langs, build_files: builds,
    services: model.services.length, external_entities: model.external_entities.length,
    flows: model.information_flows.length, flow_stereotypes: flowKinds,
  });
  process.stderr.write(`${app}: ${commit ?? error}\n`);
}
const report = { dataset: DATASET, dataset_commit: DATASET_COMMIT, applications: results.length, results };
writeFileSync(join(out, "code2dfd-languages.json"), JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));
