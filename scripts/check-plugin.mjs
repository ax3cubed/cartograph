// SRV-22: the Claude Code marketplace and plugin manifests parse, name what they must, and carry the
// crate version, so a release tag pins what users install.
import { readFileSync } from "node:fs";

const read = (p) => JSON.parse(readFileSync(new URL(`../${p}`, import.meta.url), "utf8"));
const fail = (msg) => {
  console.error(`check-plugin: ${msg}`);
  process.exit(1);
};
const crate = /\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/s.exec(
  readFileSync(new URL("../Cargo.toml", import.meta.url), "utf8"),
)?.[1];
if (!crate) fail("no [workspace.package] version in Cargo.toml");

const market = read(".claude-plugin/marketplace.json");
const plugin = read("plugins/cartograph/.claude-plugin/plugin.json");
const mcp = read("plugins/cartograph/.mcp.json");
const hooks = read("plugins/cartograph/hooks/hooks.json");
const skill = readFileSync(new URL("../plugins/cartograph/skills/cartograph/SKILL.md", import.meta.url), "utf8");

const entry = market.plugins?.find((p) => p.name === "cartograph");
if (typeof market.name !== "string" || !market.owner?.name) fail("marketplace.json needs name and owner.name");
if (!entry || entry.source !== "./plugins/cartograph") fail("marketplace.json must list cartograph at ./plugins/cartograph");
if (plugin.name !== "cartograph") fail("plugin.json name must be cartograph");
for (const [where, v] of [["plugin.json", plugin.version], ["marketplace.json", entry.version]]) {
  if (v !== crate) fail(`${where} version ${v} isn't the crate version ${crate}`);
}
const server = mcp.mcpServers?.cartograph;
if (server?.command !== "cartograph" || JSON.stringify(server.args) !== '["mcp"]') fail(".mcp.json must run `cartograph mcp`");
const start = hooks.hooks?.SessionStart?.flatMap((h) => h.hooks ?? []) ?? [];
if (!start.some((h) => h.type === "command" && /cartograph status --line/.test(h.command) && /exit 0$/.test(h.command))) {
  fail("hooks.json needs a SessionStart command running `cartograph status --line` that always exits 0");
}
if (!/^---\nname: cartograph\ndescription: .+\n---\n/.test(skill)) fail("SKILL.md needs name and description front matter");
console.log(`check-plugin: ok (version ${crate})`);
