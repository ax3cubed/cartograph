"use client";

// Board A3 (concepts) on the index: named ideas and the files that carry them (…/concepts). Concepts are written by the
// AI step, which isn't built yet, so an index holds none and this view says so (O1). Names and summaries are model
// output over repo text: rendered as text. Deciding a concept's members is a write the server doesn't take yet (N7).
import { Boxes } from "lucide-react";
import type { Concept } from "@/lib/api";
import { useWsRead } from "@/lib/api/live";
import { Chip, NodeLink, Page, ViewHead, useHashState, useNames, useView } from "./shared";
import "./concepts.css";

const BACK = "/concepts";

const HEAD = { id: "cn", title: "Concepts: what the code is about", purpose: "Related code, contracts and events grouped by idea." };

export default function Concepts() {
  const { ws, indexing } = useView();
  const res = useWsRead<Concept[]>(ws, "concepts");
  const concepts = res.data ?? [];
  const names = useNames(ws);
  const h = useHashState();
  const concept = concepts.find((c) => c.id === (h.get("concept") ?? h.bare)) ?? concepts[0];

  return (
    <Page head={HEAD} icon={Boxes} indexing={indexing} when={!res.loading && !res.error && concepts.length === 0}
      empty={{ title: "Concepts need the search index", body: "Concepts are found by grouping related code, contracts and events. The search index that finds them isn't built yet, so there are none to show. Open retrieval settings to see what it needs.", action: { label: "Open retrieval settings", href: "/settings/retrieval" } }}>
      {concept && (
        <main className="view-main" aria-labelledby="cn-h">
          <ViewHead {...HEAD} />
          <div className="view-toolbar">
            <label className="view-select">Concept
              <select value={concept.id} onChange={(e) => h.set({ concept: e.target.value })}>
                {concepts.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}
              </select>
            </label>
            <div className="view-chips"><Chip>{concept.domain}</Chip></div>
          </div>
          <div>
            <span className="view-k">{concept.nodes.length} members</span>
            <h2 className="cn-h1">{concept.name}</h2>
            <span className="view-sub">{concept.summary}</span>
          </div>
          <section className="cn-layer" aria-label="Members">
            {concept.nodes.map((id) => <div key={id} className="cn-mem"><NodeLink id={id} back={BACK} label={names(id)} /></div>)}
          </section>
        </main>
      )}
    </Page>
  );
}
