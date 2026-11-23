"use client";

// Board G11 on the index (FE-10): a file's symbols from …/nodes/{id}/symbols. The index has none until symbols are
// extracted (stage 5); the route answers an empty list until then, and this says so.
import { Braces } from "lucide-react";
import type { Node } from "@/lib/api";
import { useWsRead } from "@/lib/api/live";
import EmptyState from "@/components/shell/EmptyState";
import "./map.css";

export default function SymbolView({ ws, id }: { ws: string; id: string }) {
  const r = useWsRead<Node[]>(ws, `nodes/${encodeURIComponent(id)}/symbols`);
  if (!r.data) return r.error ? <p className="lm-none" role="alert">{r.error.what}.</p> : <p className="lm-none" aria-busy="true">Loading symbols…</p>;
  if (!r.data.length) {
    return <EmptyState icon={Braces} title="No symbols yet" body="Functions and classes show here once the index reads symbols, which this version doesn't do yet." />;
  }
  return (
    <ul className="map-sym-list" aria-label="Symbols">
      {r.data.map((s) => <li key={s.id} className="lm-mono">{s.name}</li>)}
    </ul>
  );
}
