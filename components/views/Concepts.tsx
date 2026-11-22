"use client";

import { useMemo, useState } from "react";
import { Boxes } from "lucide-react";
import { getConcepts, getContracts } from "@/lib/source";
import { chipText, confidence, type Confidence } from "@/lib/model";
import { LINKS, byId } from "@/lib/workspace";
import { ALIASES, CANDIDATES } from "@/lib/demo/views-extra";
import { toast } from "@/components/shell/toast";
import { Chip, NodeLink, Page, ViewHead, useHashState, type Tone } from "./shared";
import "./concepts.css";

const BACK = "/concepts";
interface Member { key: string; node?: string; name: string; where: string; how: string; kind: "exact" | "likely" | "guessed" | "overridden"; service: string }
const member = (key: string, name: string, where: string, service: string, c: Confidence | "override", note: string, node?: string): Member => ({
  key, node, name, where, service, kind: c === "override" ? "overridden" : c.level,
  how: c === "override" ? "Overridden · added by hand" : c.score >= 0.9 ? `Exact · ${note}` : chipText(c),
});
const TONE: Record<Member["kind"], Tone> = { exact: "ok", likely: "", guessed: "", overridden: "acc" };
const LAYERS = ["Contracts", "Data", "Events", "Code", "Docs"] as const;

export default function Concepts() {
  const concepts = useMemo(() => getConcepts(), []);
  const specs = useMemo(() => getContracts(), []);
  const h = useHashState();
  const concept = concepts.find((c) => c.id === (h.get("concept") ?? h.bare)) ?? concepts[0];
  const [decided, setDecided] = useState<Record<string, "added" | "kept">>({});
  const [saved, setSaved] = useState<Record<string, boolean>>({});

  const layers = useMemo(() => {
    if (!concept) return null;
    const nodes = new Set(concept.nodes);
    const ex = confidence(1);
    const L: Record<(typeof LAYERS)[number], Member[]> = { Contracts: [], Data: [], Events: [], Code: [], Docs: [] };
    for (const s of specs) for (const o of s.operations) {
      const mine = (o.handler && nodes.has(o.handler)) || o.callers.some((c) => nodes.has(c.node));
      if (mine) L[o.method === "EVENT" ? "Events" : "Contracts"].push(member(o.id, o.method === "EVENT" ? o.path : `${o.method} ${o.path}`, s.file, s.service, ex, "handler is in this concept", o.handler));
    }
    for (const l of LINKS) if (l.type === "db" && !l.removed && nodes.has(l.from)) { const t = byId.get(l.to)!; if (!L.Data.some((m) => m.key === t.id)) L.Data.push(member(t.id, t.name, t.path, t.service, l.confidence ?? ex, "database link", t.id)); }
    for (const id of concept.nodes) {
      const n = byId.get(id)!;
      const unsure = LINKS.filter((l) => (l.from === id || l.to === id) && l.confidence).map((l) => l.confidence!).sort((a, b) => a.score - b.score)[0];
      L.Code.push(member(id, n.name, n.path, n.service, unsure ?? ex, "named in the concept", id));
    }
    return L;
  }, [concept, specs]);

  const cand = concept ? CANDIDATES[concept.id] : undefined;
  const state = concept ? decided[concept.id] : undefined;
  const all = layers ? [...Object.values(layers).flat(), ...(cand && state === "added" ? [member(cand.node, cand.name, byId.get(cand.node)!.path, byId.get(cand.node)!.service, "override", "", cand.node)] : [])] : [];
  const n = (k: Member["kind"]) => all.filter((m) => m.kind === k).length;

  return (
    <Page icon={Boxes} when={concepts.length === 0} empty={{ title: "Concepts need the search index", body: "Concepts are found by grouping related code, contracts and events. The search index isn't built yet. Build it in retrieval settings, then come back.", action: { label: "Open retrieval settings", href: "/settings/retrieval" } }}>
      {concept && layers && (
        <main className="view-main" aria-labelledby="cn-h">
          <ViewHead id="cn" title="Concepts: what the code is about" purpose="Related code, contracts and events grouped by idea. Check the groups, then keep or fix them." />
          <div className="view-toolbar">
            <label className="view-select">Concept
              <select value={concept.id} onChange={(e) => h.set({ concept: e.target.value })}>
                {concepts.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}
              </select>
            </label>
            <div className="view-chips" aria-live="polite">
              <Chip tone="ok">{n("exact")} exact</Chip>
              {n("likely") > 0 && <Chip>{n("likely")} likely</Chip>}
              {n("guessed") > 0 && <Chip>{n("guessed")} guessed</Chip>}
              {n("overridden") > 0 && <Chip tone="acc">{n("overridden")} overridden</Chip>}
            </div>
          </div>
          <div>
            <span className="view-k">{all.length} members in {new Set(all.map((m) => m.service)).size} services</span>
            <h2 className="cn-h1">{concept.name}</h2>
            <span className="view-sub">{concept.summary} Also called {(ALIASES[concept.id] ?? []).map((a, i) => <span key={a}>{i ? ", " : ""}<span className="view-mono cn-alias">{a}</span></span>)}.</span>
          </div>
          <div className="cn-layers">
            {LAYERS.map((name) => {
              const items = name === "Code" && state === "added" && cand ? [...layers.Code, member(cand.node, cand.name, byId.get(cand.node)!.path, byId.get(cand.node)!.service, "override", "", cand.node)] : layers[name];
              return (
                <section key={name} className="cn-layer" aria-label={name}>
                  <div className="cn-lh"><b>{name}</b><span className="view-mono view-muted">{items.length}</span></div>
                  {items.length === 0 && <p className="view-note">Nothing matched in {name.toLowerCase()}.</p>}
                  {items.map((m) => (
                    <div key={m.key} className="cn-mem">
                      {m.node ? <NodeLink id={m.node} back={BACK} label={m.name} /> : <span className="view-mono cn-name">{m.name}</span>}
                      <span className="view-muted cn-where">{m.where}</span>
                      <Chip tone={TONE[m.kind]}>{m.how}</Chip>
                    </div>
                  ))}
                </section>
              );
            })}
          </div>
          {cand && !state && (
            <section className="cn-decide" aria-label="Needs a decision">
              <span className="view-k">Needs a decision</span>
              <span className="cn-why"><NodeLink id={cand.node} back={BACK} label={cand.name} /> in <NodeLink id={cand.node} back={BACK} /> {cand.why} (score {cand.score.toFixed(2)}).</span>
              <button type="button" className="btn view-btn" onClick={() => setDecided({ ...decided, [concept.id]: "kept" })}>Keep separate</button>
              <button type="button" className="btn view-btn primary" onClick={() => setDecided({ ...decided, [concept.id]: "added" })}>Add to {concept.name}</button>
            </section>
          )}
          {cand && state && (
            <div className="cn-decide" role="status">
              <span className="cn-why">{state === "kept" ? `${cand.name} kept separate for this session.` : saved[concept.id] ? `${cand.name} saved as a correction to ${concept.name}.` : `${cand.name} added to ${concept.name} in this session. Save it as a correction to keep it.`}</span>
              {state === "added" && !saved[concept.id] && <button type="button" className="btn view-btn primary" onClick={() => { setSaved({ ...saved, [concept.id]: true }); toast(`Saved a correction: ${cand.name} belongs to ${concept.name}`); }}>Save as correction</button>}
              <button type="button" className="btn view-btn" onClick={() => { const d = { ...decided }; delete d[concept.id]; setDecided(d); setSaved({ ...saved, [concept.id]: false }); }}>Undo</button>
            </div>
          )}
        </main>
      )}
    </Page>
  );
}
