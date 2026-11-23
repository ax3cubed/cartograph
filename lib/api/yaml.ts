// FE-13: a small line-based YAML editor for cartograph.yaml. The browser sends whole YAML text (ConfigWrite); these
// functions change only the lines of the keys they own, so comments, other keys and key order stay as written.
// Pure, so `pnpm check` runs them under node (yaml.check.ts).

const indentOf = (l: string) => l.length - l.trimStart().length;
const isBlank = (l: string) => !l.trim() || l.trimStart().startsWith("#");
/** Quote a scalar when YAML would read it as something else. */
export const scalar = (v: string) => (/^[\w./@*-][\w./@* -]*$/.test(v) && !/^(true|false|null|yes|no|~|-?\d)/i.test(v) && !v.endsWith(" ") ? v : JSON.stringify(v));
const unquote = (v: string) => {
  const t = v.trim();
  if (t.startsWith('"')) { try { return JSON.parse(t) as string; } catch { return t.slice(1, -1); } }
  if (t.startsWith("'")) return t.slice(1, -1).replace(/''/g, "'");
  return t.replace(/\s+#.*$/, "");
};

/** The line range [start, end) of the block under `key` at `indent` inside lines [from, to). end excludes trailing blank lines. */
function block(lines: string[], key: string, indent: number, from = 0, to = lines.length) {
  const re = new RegExp(`^ {${indent}}${key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:(\\s|$)`);
  for (let i = from; i < to; i++) {
    if (!re.test(lines[i])) continue;
    let end = i + 1, last = i + 1;
    for (; end < to; end++) {
      const l = lines[end];
      if (isBlank(l)) continue;
      if (indentOf(l) <= indent && !(indentOf(l) === indent && l.trimStart().startsWith("- "))) break;
      last = end + 1;
    }
    return { start: i, end: last };
  }
  return null;
}

/** `options.exclude` as a list of globs. */
export function readExcludes(yaml: string): string[] {
  const lines = yaml.split("\n");
  const opt = block(lines, "options", 0);
  if (!opt) return [];
  const ex = block(lines, "exclude", indentOf(lines[opt.start + 1] ?? "  ") || 2, opt.start + 1, opt.end);
  if (!ex) return [];
  const inline = lines[ex.start].split(/exclude:/)[1].trim();
  if (inline.startsWith("[")) return inline.replace(/^\[|\]\s*(#.*)?$/g, "").split(",").map(unquote).filter(Boolean);
  return lines.slice(ex.start + 1, ex.end).filter((l) => l.trimStart().startsWith("- ")).map((l) => unquote(l.trimStart().slice(2)));
}

/** `yaml` with `options.exclude` set to `globs` (removed when empty); everything else is left as it was. */
export function setExcludes(yaml: string, globs: string[]): string {
  const lines = yaml.split("\n");
  const opt = block(lines, "options", 0);
  const child = opt ? (indentOf(lines.slice(opt.start + 1, opt.end).find((l) => !isBlank(l)) ?? "  ") || 2) : 2;
  const pad = " ".repeat(child);
  const body = globs.length ? [`${pad}exclude:`, ...globs.map((g) => `${pad}  - ${scalar(g)}`)] : [];
  if (!opt) {
    if (!globs.length) return yaml;
    const tail = lines.length && lines[lines.length - 1] === "" ? lines.length - 1 : lines.length;
    return [...lines.slice(0, tail), "options:", ...body, ...lines.slice(tail)].join("\n");
  }
  const ex = block(lines, "exclude", child, opt.start + 1, opt.end);
  if (ex) lines.splice(ex.start, ex.end - ex.start, ...body);
  else lines.splice(opt.end, 0, ...body);
  // options: with nothing left under it goes too (the manifest skips an empty options)
  const o = block(lines, "options", 0)!;
  if (lines.slice(o.start + 1, o.end).every(isBlank)) lines.splice(o.start, o.end - o.start);
  return lines.join("\n");
}

export interface ServiceEntry { name: string; repo: string; path?: string }

/** `yaml` with a `services:` entry for `s.name` added, or replaced when one with that name exists. */
export function setService(yaml: string, s: ServiceEntry): string {
  const lines = yaml.split("\n");
  const entry = (pad: string) => [`${pad}- name: ${scalar(s.name)}`, `${pad}  repo: ${scalar(s.repo)}`, ...(s.path ? [`${pad}  path: ${scalar(s.path)}`] : [])];
  const svc = block(lines, "services", 0);
  if (!svc) {
    const tail = lines.length && lines[lines.length - 1] === "" ? lines.length - 1 : lines.length;
    return [...lines.slice(0, tail), "services:", ...entry("  "), ...lines.slice(tail)].join("\n");
  }
  const items: { start: number; end: number }[] = [];
  for (let i = svc.start + 1; i < svc.end; i++) {
    if (lines[i].trimStart().startsWith("- ")) { if (items.length) items[items.length - 1].end = i; items.push({ start: i, end: svc.end }); }
  }
  const first = items[0];
  const pad = first ? " ".repeat(indentOf(lines[first.start])) : "  ";
  const named = items.find((it) => lines.slice(it.start, it.end).some((l) => unquote(l.replace(/^\s*(- )?name:/, "")) === s.name && /^\s*(- )?name:/.test(l)));
  if (named) {
    let end = named.end;
    while (end > named.start + 1 && isBlank(lines[end - 1])) end--;
    lines.splice(named.start, end - named.start, ...entry(pad));
  } else lines.splice(svc.end, 0, ...entry(pad));
  return lines.join("\n");
}
