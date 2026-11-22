// Answers questions by walking the graph. Every claim carries the nodes/commits it came from,
// so the UI can show the evidence. These same functions are the tools an LLM agent would call later.

import type { McpToolName } from "./model.ts";
import {
  COMMITS, LINKS, NODES, byId, commitBySha, dayMs, descendants, isActive, type Link, type LinkType,
} from "./workspace.ts";

export type Part = string | { node: string } | { commit: string };
export interface Answer {
  q: string;
  title: string;
  lines: Part[][];
  nodes: string[]; // evidence to highlight on the map
  links: string[]; // `${from}>${to}`
  steps: string[]; // how the answer was reached
  by?: string; // model that worded `lines`; absent when the graph engine's own wording is shown
  plan?: Plan; // which kind of question the router decided this was
  asOf?: number; // the date the question was asked about (ms)
  context?: Context; // what was retrieved and sent to the model (board A06)
  checked?: Checked; // the model's wording after citation checks (decision G5)
  notice?: string; // something the reader should know about how this answer was produced
}

export type Plan = "history" | "path" | "impact" | "describe" | "none";

// ---------- retrieval and citations (decisions G5, G8) ----------

/** One row a tool returned this turn. The model cites `handle`, never a raw id. */
export interface Handle {
  handle: string; // e1, e2…
  kind: "node" | "link" | "commit";
  id: string; // node id, `${from}>${to}`, or sha
  label: string;
  from: number; // the interval in which the object is true, in ms
  to: number | null; // null = still true
}
export interface ToolRun { tool: McpToolName; args: string; rows: string[] } // rows are handles
export interface Block { name: string; tokens: number }
export interface Context {
  system: string;
  messages: { role: string; content: string }[];
  handles: Handle[];
  tools: ToolRun[];
  blocks: Block[];
  budget: number; // tokens
  tokens: number; // tokens sent
  leftOut: number; // map nodes not sent
  ultra: boolean;
  seeds: string[]; // node ids named in the question
}

export interface CheckedCitation { handle: string; ok: boolean; why?: string }
export interface Sentence { text: string; cites: CheckedCitation[]; supported: boolean }
export interface Checked {
  paragraphs: Sentence[][];
  stats: { sentences: number; unsupported: number; cited: number; kept: number; dropped: number };
}

const CITE = /\[(e\d+)\]/g;

/** Decision G5 as a pure function. A citation stands only if its handle came from this turn and its interval
 *  covers `t`. Failed citations are stripped; a sentence left with none is marked unsupported. */
export function checkCitations(answerText: string, handles: Pick<Handle, "handle" | "from" | "to">[], t: number): Checked {
  const by = new Map(handles.map((h) => [h.handle, h]));
  const ok = new Set<string>(), bad = new Set<string>();
  const paragraphs = answerText.split(/\n+/).map((p) => p.trim()).filter(Boolean).map((p) =>
    // a citation after the full stop belongs to the sentence before it
    p.replace(/([.!?])\s*((?:\[e\d+\]\s*)+)/g, (_, end: string, cs: string) => `${cs.trim()}${end} `)
      .split(/(?<=[.!?])\s+/)
      .map((raw): Sentence => {
        const cites = [...new Set([...raw.matchAll(CITE)].map((m) => m[1]))].map((handle): CheckedCitation => {
          const h = by.get(handle);
          const c = !h ? { handle, ok: false, why: "not in this turn's results" }
            : h.from > t || (h.to !== null && h.to <= t) ? { handle, ok: false, why: "not true on the date asked about" }
            : { handle, ok: true };
          (c.ok ? ok : bad).add(handle);
          return c;
        });
        const text = raw.replace(CITE, "").replace(/\s+([.,;:!?])/g, "$1").replace(/\s{2,}/g, " ").trim();
        return { text, cites, supported: cites.some((c) => c.ok) };
      })
      .filter((s) => s.text),
  ).filter((p) => p.length);
  const all = paragraphs.flat();
  return { paragraphs, stats: { sentences: all.length, unsupported: all.filter((s) => !s.supported).length, cited: ok.size + bad.size, kept: ok.size, dropped: bad.size } };
}

const DAY = 86_400_000;
export const linkKey = (l: Link) => `${l.from}>${l.to}`;
const name = (id: string) => byId.get(id)!.name;
const svc = (id: string) => byId.get(id)!.service;
const fmt = (t: number) => new Date(t).toLocaleString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });

const VERB: Record<LinkType, [string, string]> = {
  imports: ["imports", "importing"],
  calls: ["calls", "calling"],
  http: ["calls over HTTP", "calling over HTTP"],
  event: ["sends events through", "sending events through"],
  db: ["queries", "querying"],
};

// ---------- mention + date parsing ----------

const ALIASES = NODES.filter((n) => n.kind !== "workspace")
  .flatMap((n) => {
    const a = [n.name.toLowerCase()];
    if (n.kind === "file") a.push(n.name.replace(/\.\w+$/, "").toLowerCase());
    if (n.kind === "service") a.push(n.id, n.name.replace(/-service$/, ""));
    return a.filter((s) => s.length >= 3).map((s) => [s, n.id] as const);
  })
  // longest alias first so "createInvoice.ts" wins over "invoices"; files before modules on ties
  .sort((a, b) => b[0].length - a[0].length || (byId.get(b[1])!.kind === "file" ? 1 : -1));

export function mentions(q: string): string[] {
  let text = q.toLowerCase();
  const found: { id: string; at: number }[] = [];
  for (const [alias, id] of ALIASES) {
    const re = new RegExp(`(^|[^\\w.-])${alias.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?=$|[^\\w-]|\\.(?!\\w))`);
    const m = re.exec(text);
    if (!m) continue;
    const at = m.index + m[1].length;
    found.push({ id, at });
    text = text.slice(0, at) + " ".repeat(alias.length) + text.slice(at + alias.length);
  }
  return [...new Set(found.sort((a, b) => a.at - b.at).map((f) => f.id))];
}

const MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
export function since(q: string, t: number): number {
  const s = q.toLowerCase();
  const rel = /(last|past)\s+(\d+)?\s*(day|week|month|year)s?/.exec(s);
  if (rel) return t - (+rel[2] || 1) * { day: 1, week: 7, month: 30, year: 365 }[rel[3] as "day"] * DAY;
  const thisP = /this\s+(week|month|year)/.exec(s);
  if (thisP) return t - { week: 7, month: 30, year: 365 }[thisP[1] as "week"] * DAY;
  const mon = [...s.matchAll(/(?:since|from|in|after)\s+([a-z]{3})[a-z]*\.?\s*(\d{4})?/g)].find((m) => MONTHS.includes(m[1]));
  if (mon) {
    const m = MONTHS.indexOf(mon[1]);
    let y = mon[2] ? +mon[2] : new Date(t).getUTCFullYear();
    if (!mon[2] && Date.UTC(y, m, 1) > t) y--;
    return Date.UTC(y, m, 1, 12);
  }
  return t - 90 * DAY;
}

// ---------- tools ----------

/** Everything that depends on `id`, directly or transitively, grouped by hop. */
export function impact(id: string, t: number) {
  const start = descendants(id);
  const seen = new Set(start);
  const levels: { nodes: string[]; links: Link[] }[] = [];
  let frontier = start;
  while (frontier.size) {
    const next = new Set<string>();
    const ls: Link[] = [];
    for (const l of LINKS) {
      if (!isActive(l, t) || !frontier.has(l.to) || seen.has(l.from)) continue;
      ls.push(l);
      next.add(l.from);
    }
    next.forEach((n) => seen.add(n));
    if (next.size) levels.push({ nodes: [...next], links: ls });
    frontier = next;
  }
  return levels;
}

/** Shortest chain of active links between two scopes, following dependency direction first. */
export function path(a: string, b: string, t: number): { links: Link[]; dir: "forward" | "backward" | "mixed" } | null {
  const from = descendants(a), to = descendants(b);
  const active = LINKS.filter((l) => isActive(l, t));
  const bfs = (step: (n: string) => [string, Link][]) => {
    const prev = new Map<string, Link | null>();
    const queue = [...from];
    queue.forEach((n) => prev.set(n, null));
    while (queue.length) {
      const n = queue.shift()!;
      if (to.has(n)) {
        const chain: Link[] = [];
        for (let cur = n; prev.get(cur); ) {
          const l = prev.get(cur)!;
          chain.unshift(l);
          cur = l.from === cur ? l.to : l.from;
        }
        return chain;
      }
      for (const [m, l] of step(n)) if (!prev.has(m)) (prev.set(m, l), queue.push(m));
    }
    return null;
  };
  const fwd = bfs((n) => active.filter((l) => l.from === n).map((l) => [l.to, l]));
  if (fwd) return { links: fwd, dir: "forward" };
  const back = bfs((n) => active.filter((l) => l.to === n).map((l) => [l.from, l]));
  if (back) return { links: back, dir: "backward" };
  const any = bfs((n) => active.flatMap((l): [string, Link][] => (l.from === n ? [[l.to, l]] : l.to === n ? [[l.from, l]] : [])));
  return any ? { links: any, dir: "mixed" } : null;
}

/** Plain-language story of each commit's relationship changes inside (from, to]. */
export function stories(scopeId: string | null, from: number, to: number) {
  const scope = scopeId ? descendants(scopeId) : null;
  const touches = (l: Link) => !scope || scope.has(l.from) || scope.has(l.to);
  const out: { sha: string; parts: Part[]; added: Link[]; removed: Link[] }[] = [];
  for (const c of [...COMMITS].reverse()) {
    const d = dayMs(c.date);
    if (d <= from || d > to) continue;
    const added = LINKS.filter((l) => l.added === c.sha && touches(l));
    const removed = LINKS.filter((l) => l.removed === c.sha && touches(l));
    if (!added.length && !removed.length) continue;
    const parts: Part[] = [];
    const sep = () => parts.length && parts.push("; ");
    const usedAdds = new Set<Link>();
    for (const r of removed) {
      const swap = added.find((a) => a.from === r.from && !usedAdds.has(a));
      sep();
      if (swap) {
        usedAdds.add(swap);
        parts.push({ node: r.from }, ` stopped ${VERB[r.type][1]} `, { node: r.to }, ` and now ${VERB[swap.type][0]} `, { node: swap.to });
      } else parts.push({ node: r.from }, ` no longer ${VERB[r.type][0]} `, { node: r.to });
    }
    for (const a of added.filter((x) => !usedAdds.has(x))) {
      sep();
      parts.push({ node: a.from }, ` now ${VERB[a.type][0]} `, { node: a.to });
    }
    out.push({ sha: c.sha, parts, added, removed });
  }
  return out;
}

/** Files in a scope ordered callers-first: a suggested reading order. */
export function readingOrder(scopeId: string, t: number) {
  const files = [...descendants(scopeId)].filter((id) => byId.get(id)!.kind === "file");
  const inside = new Set(files);
  const internal = LINKS.filter((l) => isActive(l, t) && inside.has(l.from) && inside.has(l.to));
  const indeg = new Map(files.map((f) => [f, internal.filter((l) => l.to === f).length]));
  const order: string[] = [];
  const queue = files.filter((f) => indeg.get(f) === 0);
  while (queue.length) {
    const f = queue.shift()!;
    order.push(f);
    for (const l of internal.filter((x) => x.from === f)) {
      indeg.set(l.to, indeg.get(l.to)! - 1);
      if (indeg.get(l.to) === 0) queue.push(l.to);
    }
  }
  return [...order, ...files.filter((f) => !order.includes(f))]; // ponytail: cycles just go last
}

// ---------- the question router ----------

const list = (ids: string[], max = 6): Part[] => {
  const shown = ids.slice(0, max).flatMap((id, i): Part[] => [i ? ", " : "", { node: id }]);
  return ids.length > max ? [...shown, ` and ${ids.length - max} more`] : shown;
};

export const SUGGESTIONS = [
  "What breaks if I change schema.ts?",
  "How does checkout reach stripeAdapter?",
  "What changed in billing since June?",
  "What does auth-service do?",
];

/** Tools the composer offers after "/". Each forces one of the engine's answer plans instead of guessing it from the wording. */
export const TOOLS = [
  { id: "impact", label: "/impact", hint: "What breaks if a file, module or service changes", args: "@node" },
  { id: "path", label: "/path", hint: "How one node reaches another", args: "@from @to" },
  { id: "describe", label: "/describe", hint: "What a node does, and what it uses and is used by", args: "@node" },
  { id: "changes", label: "/changes", hint: "Links added and removed since a date", args: "[@node] since June" },
] as const;
export type ToolId = (typeof TOOLS)[number]["id"];
// The words each tool's plan is routed on below.
const TOOL_WORDS: Record<ToolId, string> = { impact: "impact", path: "path", describe: "describe", changes: "changes since" };

/** A node picked with "@": the text inserted for it and the exact id it stands for. */
export interface Ref { token: string; id: string }

/** Splits composer text into the question, the "/" tool (only at the very start) and the "@" nodes still present, in order. */
export function parseComposer(text: string, refs: Ref[]): { text: string; tool?: ToolId; ids: string[] } {
  const m = /^\/(\w+)\s*/.exec(text);
  const tool = m && TOOLS.some((t) => t.id === m[1]) ? (m[1] as ToolId) : undefined;
  const ids = refs
    .map((r) => ({ id: r.id, at: text.indexOf(r.token) }))
    .filter((r) => r.at >= 0)
    .sort((a, b) => a.at - b.at)
    .map((r) => r.id);
  return { text: tool ? text.slice(m![0].length) : text, tool, ids };
}

/** `opts.ids` are nodes picked with "@" (exact ids, in order); `opts.tool` is a "/" command. */
export function ask(q: string, t: number, opts: { ids?: string[]; tool?: ToolId } = {}): Answer {
  let ids = opts.ids?.length ? [...new Set(opts.ids)] : mentions(q);
  if (opts.tool && opts.tool !== "path") ids = ids.slice(0, 1); // one subject, so the path plan can't take over
  const s = opts.tool ? TOOL_WORDS[opts.tool] : q.toLowerCase();
  const base = { q, nodes: [] as string[], links: [] as string[], steps: [] as string[] };
  if (opts.tool === "path" && ids.length < 2) {
    return { ...base, plan: "none", title: "Pick two nodes", lines: [["/path needs a start and an end. Type @ to pick each one, for example: /path @checkout @stripeAdapter.ts"]] };
  }

  if (/chang|happen|done|new|since|recent|histor|last (week|month|year)|this (week|month|year)/.test(s) && !/if i change|break|impact|affect/.test(s)) {
    const scope = ids[0] ?? null;
    const from = since(q, t);
    const st = stories(scope, from, t);
    const added = st.flatMap((x) => x.added), removed = st.flatMap((x) => x.removed);
    const authors = new Set(st.map((x) => commitBySha.get(x.sha)!.author));
    const crossNow = LINKS.filter((l) => isActive(l, t) && svc(l.from) !== svc(l.to) && (!scope || descendants(scope).has(l.from) || descendants(scope).has(l.to))).length;
    const crossThen = LINKS.filter((l) => isActive(l, from) && svc(l.from) !== svc(l.to) && (!scope || descendants(scope).has(l.from) || descendants(scope).has(l.to))).length;
    return {
      ...base, plan: "history",
      title: `What changed${scope ? ` around ${name(scope)}` : ""} since ${fmt(from)}`,
      lines: st.length
        ? [
            [`${st.length} commit${st.length > 1 ? "s" : ""} by ${authors.size} ${authors.size > 1 ? "people" : "person"} changed ${added.length + removed.length} links: ${added.length} added, ${removed.length} removed. Cross-service links went from ${crossThen} to ${crossNow}.`],
            ...st.map((x) => [{ commit: x.sha }, " ", ...x.parts, "."] as Part[]),
          ]
        : [[`No relationship changes${scope ? " around " : ""}`, ...(scope ? [{ node: scope } as Part] : []), ` between ${fmt(from)} and ${fmt(t)}.`]],
      nodes: [...new Set([...added, ...removed].flatMap((l) => [l.from, l.to]))],
      links: [...added, ...removed].map(linkKey),
      steps: [`Read git history from ${fmt(from)} to ${fmt(t)}.`, `Kept commits that added or removed a link${scope ? ` touching ${name(scope)}` : ""}.`, "Paired a removal and an addition from the same file in one commit as a replacement."],
    };
  }

  if (ids.length >= 2) {
    const [a, b] = ids;
    const p = path(a, b, t);
    if (!p) return { ...base, plan: "path", title: `${name(a)} → ${name(b)}`, lines: [["No chain of links connects ", { node: a }, " and ", { node: b }, ` on ${fmt(t)}.`]], nodes: [a, b] };
    const first = p.links[0];
    const chain: Part[] = [{ node: descendants(a).has(first.from) ? first.from : first.to }];
    let cur = chain[0] as { node: string };
    for (const l of p.links) {
      const fwd = l.from === cur.node;
      const nxt = fwd ? l.to : l.from;
      chain.push(fwd ? ` ${VERB[l.type][0]} ` : ` ← is used by (${l.type}) `, { node: nxt });
      cur = { node: nxt };
    }
    const services = new Set(p.links.flatMap((l) => [svc(l.from), svc(l.to)]));
    const network = p.links.filter((l) => l.type === "http" || l.type === "event").length;
    return {
      ...base, plan: "path",
      title: `How ${name(a)} reaches ${name(b)}`,
      lines: [
        [...chain, "."],
        [`${p.links.length} hop${p.links.length > 1 ? "s" : ""} across ${services.size} service${services.size > 1 ? "s" : ""}${network ? `, ${network} of them over the network (HTTP or events), so a break there surfaces at runtime, not at build time` : ""}.`],
        ...(p.dir === "backward" ? [["Direction: it is ", { node: b }, " that depends on ", { node: a }, ", not the other way round."] as Part[]] : []),
        ...(p.dir === "mixed" ? [["They're related through shared dependencies rather than a direct chain of calls."] as Part[]] : []),
      ],
      nodes: [...new Set(p.links.flatMap((l) => [l.from, l.to]))],
      links: p.links.map(linkKey),
      steps: [`Breadth-first search over active links on ${fmt(t)}, following "depends on" first, then "used by", then either.`],
    };
  }

  if (ids.length === 1 && /break|impact|affect|if i (change|touch|edit|remove|delete)|depends? on|blast|risk|who uses|used by/.test(s)) {
    const id = ids[0];
    const levels = impact(id, t);
    const all = levels.flatMap((l) => l.nodes);
    const services = new Set(all.map(svc));
    const links = levels.flatMap((l) => l.links);
    const network = links.filter((l) => l.type === "http" || l.type === "event");
    return {
      ...base, plan: "impact",
      title: `If you change ${name(id)}`,
      lines: all.length
        ? [
            [`${all.length} file${all.length > 1 ? "s" : ""} in ${services.size} service${services.size > 1 ? "s" : ""} depend on it, directly or indirectly.`],
            ...levels.map((l, i): Part[] => [i ? `${i + 1} hops away: ` : "Directly: ", ...list(l.nodes)]),
            ...(network.length ? [[`${network.length} of these links cross the network (`, ...list(network.map((l) => l.from), 3), "), so compile-time checks won't catch a break there. Test those callers."] as Part[]] : []),
          ]
        : [["Nothing depends on ", { node: id }, ` on ${fmt(t)}. It can change in isolation.`]],
      nodes: [id, ...all],
      links: links.map(linkKey),
      steps: [`Started at ${name(id)}${byId.get(id)!.kind !== "file" ? " and every file inside it" : ""}.`, `Followed "used by" links backwards, one hop at a time, until nothing new appeared (${levels.length} hop${levels.length === 1 ? "" : "s"}).`],
    };
  }

  if (ids.length === 1) {
    const id = ids[0];
    const n = byId.get(id)!;
    const scope = descendants(id);
    const out = LINKS.filter((l) => isActive(l, t) && scope.has(l.from) && !scope.has(l.to));
    const inn = LINKS.filter((l) => isActive(l, t) && scope.has(l.to) && !scope.has(l.from));
    const last = [...COMMITS].reverse().find((c) => dayMs(c.date) <= t && LINKS.some((l) => (l.added === c.sha || l.removed === c.sha) && (scope.has(l.from) || scope.has(l.to))));
    const order = n.kind === "file" ? [] : readingOrder(id, t);
    return {
      ...base, plan: "describe",
      title: `What ${n.name} does`,
      lines: [
        [{ node: id }, `: ${n.summary}`],
        out.length ? ["It relies on ", ...list([...new Set(out.map((l) => l.to))])] : ["It relies on nothing outside itself."],
        inn.length ? ["It's used by ", ...list([...new Set(inn.map((l) => l.from))])] : ["Nothing outside it uses it yet."],
        ...(order.length > 1 ? [["Read it in this order (callers first): ", ...list(order, 8)] as Part[]] : []),
        ...(last ? [["Its links last changed in ", { commit: last.sha }, ` (${last.message}).`] as Part[]] : []),
      ],
      nodes: [id, ...out.map((l) => l.to), ...inn.map((l) => l.from)],
      links: [...out, ...inn].map(linkKey),
      steps: ["Read the node's summary.", `Collected links crossing its boundary on ${fmt(t)}.`, ...(order.length > 1 ? ["Ordered its files so each one is read before the files it calls."] : [])],
    };
  }

  return {
    ...base, plan: "none",
    title: "Try naming a service, module or file",
    lines: [["I answer from the map, so I need something to look at. For example:"], ...SUGGESTIONS.map((x) => [x])],
  };
}

export const describeCommit = (sha: string) => commitBySha.get(sha)!;
