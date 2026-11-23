"use client";

// FE-10: the selected node's details on the index. Header and figures from …/nodes/{id}, the links of the drawn
// slice (each opens the link inspector at file level), "Trace on the map" (…/reach), owners, renames and rewrites
// (G10) and symbols (G11). Names, paths and owners come from the repo: text only.
import { forwardRef, useState } from "react";
import { Route } from "lucide-react";
import type { GraphSlice, NodeDetail } from "@/lib/api";
import { useWsRead } from "@/lib/api/live";
import History from "../timeline/History";
import LinkInspector from "./LinkInspector";
import SymbolView from "./SymbolView";
import { linked } from "./table";

const KIND: Record<string, string> = { service: "Service", module: "Module", file: "File" };

export interface DetailsProps {
  ws: string;
  /** The `at=` of the drawn slice ("" is now). */
  at: string;
  s: GraphSlice;
  i: number;
  name: (nodeId: string) => string;
  dateOf: (sha: string) => string | undefined;
  onPick: (i: number) => void;
  onOpenId: (nodeId: string) => void;
  onTrace: (nodeId: string) => void;
  onJump: (sha: string) => void;
  onClose: () => void;
}

const Details = forwardRef<HTMLElement, DetailsProps>(function Details({ ws, at, s, i, name, dateOf, onPick, onOpenId, onTrace, onJump, onClose }, ref) {
  const id = s.nodes.id[i];
  const kind = s.nodes.kind[i];
  const [link, setLink] = useState<{ id: string; of: string } | null>(null);
  const d = useWsRead<NodeDetail>(ws, `nodes/${encodeURIComponent(id)}`, { at: at || "now" }).data;
  const links = linked(s, i);
  const linkOf = (a: number, b: number) => s.links.id[s.links.from.findIndex((f, j) => f === a && s.links.to[j] === b)];
  const file = kind === "file";
  const title = file ? s.nodes.name[i].split("/").pop() : s.nodes.name[i];
  const open = link && link.of === id ? link.id : null;

  return (
    <aside ref={ref} tabIndex={-1} className="lm-panel" aria-label={file ? "Selected file" : `Selected ${kind}`}>
      {open ? (
        <LinkInspector ws={ws} id={open} at={at} name={name} dateOf={dateOf} onBack={() => setLink(null)} onOpen={(n) => { setLink(null); onOpenId(n); }} />
      ) : (
        <>
          <h2>{title}</h2>
          <p className="lm-path">{file ? s.nodes.name[i] : d?.node.path || KIND[kind]}</p>
          {d && (
            <dl className="lm-facts">
              <div><dt>Used by</dt><dd>{d.metrics.fan_in}</dd></div>
              <div><dt>Uses</dt><dd>{d.metrics.fan_out}</dd></div>
              <div><dt>Changed links, 90 days</dt><dd>{d.metrics.churn}</dd></div>
              {d.owners.length > 0 && <div className="is-wide"><dt>Owners</dt><dd>{d.owners.join(", ")}</dd></div>}
              {d.added && <div className="is-wide"><dt>First seen</dt><dd><code>{d.added.sha.slice(0, 7)}</code>{dateOf(d.added.sha) ? ` · ${dateOf(d.added.sha)}` : ""}</dd></div>}
            </dl>
          )}
          <List title={file ? "Imports" : "Uses"} ids={links.out} s={s} onPick={onPick} why={file ? (j) => setLink({ id: linkOf(i, j), of: id }) : undefined} />
          <List title={file ? "Imported by" : "Used by"} ids={links.in} s={s} onPick={onPick} why={file ? (j) => setLink({ id: linkOf(j, i), of: id }) : undefined} />
          <div className="lm-actions">
            <button type="button" className="btn" onClick={() => onTrace(id)}><Route size={14} aria-hidden /> Trace on the map</button>
            <button type="button" className="btn" onClick={onClose}>Close</button>
          </div>
          <details className="lm-more">
            <summary>Renames and rewrites</summary>
            <History ws={ws} nodeId={id} kind={kind} dateOf={dateOf} onJump={onJump} />
          </details>
          {file && (
            <details className="lm-more">
              <summary>Symbols</summary>
              <SymbolView ws={ws} id={id} />
            </details>
          )}
        </>
      )}
    </aside>
  );
});
export default Details;

function List({ title, ids, s, onPick, why }: { title: string; ids: number[]; s: GraphSlice; onPick: (i: number) => void; why?: (i: number) => void }) {
  return (
    <section>
      <h3>{title} <span>{ids.length}</span></h3>
      {ids.length === 0 ? <p className="lm-none">None at this date.</p> : (
        <ul>{ids.map((i) => (
          <li key={i} className={why ? "has-why" : undefined}>
            <button type="button" onClick={() => onPick(i)}>{s.nodes.name[i]}</button>
            {why && <button type="button" className="lm-why" onClick={() => why(i)} aria-label={`Why this link: ${s.nodes.name[i]}`}>Why</button>}
          </li>
        ))}</ul>
      )}
    </section>
  );
}
