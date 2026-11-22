// Link logic for the map: weak links, corrections, bundles, trace reach, filters and the YAML a correction would write.
import { COMMITS, LINKS, NODES, byId, descendants, isActive } from "@/lib/workspace";
import { HIDDEN_BELOW, confidence, type Correction, type CorrectionAction, type Link, type LinkType } from "@/lib/model";
import { WEAK_LINKS } from "@/lib/demo/map";
import { getConcepts, getContracts, getEntrypoints } from "@/lib/source";

export const ALL_LINKS: Link[] = [...LINKS, ...WEAK_LINKS];
export const score = (l: Link) => l.confidence?.score ?? 1;
export const level = (l: Link) => l.confidence?.level ?? "exact";
export const lkey = (l: { from: string; to: string }) => `${l.from}>${l.to}`;
export interface EffLink extends Link { fix?: Correction }
export const DESC = new Map(NODES.map((n) => [n.id, descendants(n.id)]));
export const KINDS: Record<LinkType, string> = { imports: "Import", calls: "Call", http: "HTTP", event: "Event", db: "Database" };

const SCORE = { exact: 1, likely: 0.75, guessed: 0.45 } as const;

/** The links the map draws once corrections are applied. `removed` keeps what was hidden, so it can be put back. */
export function applyCorrections(corrections: Correction[]) {
  let links: EffLink[] = ALL_LINKS.map((l) => ({ ...l }));
  const removed: { link: Link; fix: Correction }[] = [];
  for (const c of [...corrections].reverse()) {
    if (c.action === "add") {
      const day = c.at;
      const sha = [...COMMITS].reverse().find((x) => x.date <= day)?.sha ?? COMMITS[0].sha;
      links.push({ from: c.target, to: c.to!, type: (c.value as LinkType) ?? "calls", added: sha, fix: c });
      continue;
    }
    const i = links.findIndex((l) => lkey(l) === c.target);
    if (i < 0) continue;
    const l = links[i];
    if (c.action === "remove") { removed.push({ link: l, fix: c }); links = links.filter((_, j) => j !== i); }
    else if (c.action === "retarget" && c.to && byId.has(c.to)) links[i] = { ...l, to: c.to, fix: c };
    else if (c.action === "set") links[i] = { ...l, fix: c, confidence: confidence(SCORE[c.value as keyof typeof SCORE] ?? 1, { kind: "history", note: `Set by hand. ${c.reason}` }) };
  }
  return { links, removed };
}

/** Links between two nodes at any level, active at t. */
export const between = (links: EffLink[], a: string, b: string, t: number) =>
  links.filter((l) => DESC.get(a)!.has(l.from) && DESC.get(b)!.has(l.to) && isActive(l, t));

/** Nodes the filters leave visible: used-by count and link kind apply to what each node's subtree receives. */
export function visibleIds(links: EffLink[], depth: number, t: number, type: LinkType | "all", minUsedBy: number, showWeak: boolean) {
  const rank = { workspace: 0, service: 1, module: 2, file: 3 };
  const act = links.filter((l) => isActive(l, t) && (type === "all" || l.type === type) && (showWeak || score(l) >= HIDDEN_BELOW));
  const pass = new Set<string>();
  for (const n of NODES) {
    if (n.kind === "workspace" || rank[n.kind] > depth) continue;
    const d = DESC.get(n.id)!;
    const used = act.filter((l) => d.has(l.to) && !d.has(l.from)).length;
    const touch = type === "all" || act.some((l) => d.has(l.to) || d.has(l.from));
    if (used >= minUsedBy && touch) pass.add(n.id);
  }
  for (const id of [...pass]) for (let p = byId.get(id)!.parent; p; p = byId.get(p)!.parent) pass.add(p);
  return pass;
}

/** Neighbours of a scope over the effective links, like workspace.neighbours. */
export function neighboursOf(links: EffLink[], scopeId: string, t: number) {
  const scope = DESC.get(scopeId)!;
  const dependsOn = new Map<string, EffLink[]>(), usedBy = new Map<string, EffLink[]>();
  const push = (m: Map<string, EffLink[]>, k: string, l: EffLink) => m.set(k, [...(m.get(k) ?? []), l]);
  for (const l of links) {
    if (!isActive(l, t)) continue;
    const fi = scope.has(l.from), ti = scope.has(l.to);
    if (fi && !ti) push(dependsOn, l.to, l);
    if (ti && !fi) push(usedBy, l.from, l);
  }
  return { dependsOn, usedBy };
}

/** Decision D6: what a trace can reach, following links from the start nodes. */
export function traceStart(id: string): string[] {
  if (byId.has(id)) return [...DESC.get(id)!];
  const ep = getEntrypoints().find((e) => e.id === id);
  if (ep) return [ep.node];
  const op = getContracts().flatMap((s) => s.operations).find((o) => o.id === id);
  if (op) return op.handler ? [op.handler] : [];
  return getConcepts().find((c) => c.id === id)?.nodes ?? [];
}
export function reach(starts: string[], links: EffLink[], t: number) {
  const nodes = new Set(starts), ls = new Set<string>();
  const live = links.filter((l) => isActive(l, t) && score(l) >= HIDDEN_BELOW);
  for (let grew = true; grew; ) {
    grew = false;
    for (const l of live) if (nodes.has(l.from)) { ls.add(lkey(l)); if (!nodes.has(l.to)) (nodes.add(l.to), (grew = true)); }
  }
  return { nodes, links: ls };
}

// ---------- the override file ----------
export interface Draft { action: CorrectionAction; link: { from: string; to: string; type: LinkType }; newTo?: string; level?: string; reason: string }
const q = (s: string) => JSON.stringify(s);
/** The lines a correction adds to cartograph.overrides.yaml (decision H3). */
export function yamlOf(d: Draft) {
  const { from, to, type } = d.link;
  const key = d.action === "retarget" ? `{ kind: ${type}, from: ${from}, to: ${to}, new_to: ${d.newTo || "…"} }`
    : d.action === "set" ? `{ kind: ${type}, from: ${from}, to: ${to}, confidence: ${d.level ?? "exact"} }`
    : `{ kind: ${type}, from: ${from}, to: ${to} }`;
  return [`- ${d.action}: ${key}`, `  reason: ${q(d.reason.trim() || "…")}`];
}
export function toCorrection(d: Draft, at: string): Correction {
  const { from, to } = d.link;
  const base = { id: `local-${Date.now()}`, reason: d.reason.trim(), by: "You", at };
  if (d.action === "add") return { ...base, action: "add", target: from, to, value: d.link.type };
  const target = `${from}>${to}`;
  if (d.action === "retarget") return { ...base, action: "retarget", target, to: d.newTo };
  if (d.action === "set") return { ...base, action: "set", target, value: d.level ?? "exact" };
  return { ...base, action: "remove", target };
}
