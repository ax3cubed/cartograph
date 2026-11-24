// Run: pnpm check
import assert from "node:assert/strict";
import { readExcludes, readRetrieval, setExcludes, setRetrieval, setService } from "./yaml.ts";

const base = `# Northwind
version: 1
workspace:
  name: northwind # the shop
repos:
  - name: shop
    path: .
options:
  # skip generated code
  exclude:
    - "dist/**"
    - vendor/**
  max_file_bytes: 2000000
rules: []
`;
assert.deepEqual(readExcludes(base), ["dist/**", "vendor/**"]);
const two = setExcludes(base, ["dist/**", "gen/**"]);
assert.deepEqual(readExcludes(two), ["dist/**", "gen/**"]);
assert.equal(two, base.replace(`    - "dist/**"\n    - vendor/**`, `    - dist/**\n    - gen/**`), "only the list lines change");
assert.ok(two.includes("# skip generated code") && two.includes("# the shop"), "comments kept");

const none = setExcludes(base, []);
assert.ok(!none.includes("exclude") && none.includes("max_file_bytes: 2000000"), "the list goes, the sibling stays");
const bare = "version: 1\nrepos:\n  - name: a\n    path: .\n";
assert.equal(setExcludes(bare, []), bare);
const added = setExcludes(bare, ["tmp/**"]);
assert.equal(added, "version: 1\nrepos:\n  - name: a\n    path: .\noptions:\n  exclude:\n    - tmp/**\n");
assert.equal(setExcludes(added, []), bare, "an emptied options block goes too");

const svc = setService(bare, { name: "billing", repo: "a", path: "billing" });
assert.equal(svc, `${bare.slice(0, -1)}\nservices:\n  - name: billing\n    repo: a\n    path: billing\n`);
const again = setService(svc, { name: "billing", repo: "a", path: "billing/src" });
assert.ok(again.includes("path: billing/src") && !again.includes("path: billing\n"), "replaced by name");
const more = setService(again, { name: "auth", repo: "a" });
assert.equal((more.match(/- name:/g) ?? []).length, 3, "added beside the repo and billing entries");
assert.ok(more.indexOf("billing/src") < more.indexOf("name: auth"), "order kept");

// retrieval: (FE-13)
assert.deepEqual(readRetrieval(base), {});
assert.equal(setRetrieval(base, {}), base, "nothing to write leaves the file alone");
const ret = setRetrieval(base, { include: ["cards", "changes"], secret_scan: false });
assert.ok(ret.startsWith(base.trimEnd()) && ret.endsWith("retrieval:\n  include: [cards, changes]\n  secret_scan: false\n"), "appended, the rest as it was");
assert.deepEqual(readRetrieval(ret), { include: ["cards", "changes"], secret_scan: false });
const ret2 = setRetrieval(ret, { secret_scan: true });
assert.deepEqual(readRetrieval(ret2), { secret_scan: true });
assert.equal(setRetrieval(ret2, {}), base, "the section goes when it is back to the defaults");
assert.deepEqual(readRetrieval("version: 1\nretrieval:\n  include:\n    - docs\n    - \"cards\"\n"), { include: ["docs", "cards"] });
console.log("settings yaml ok");
