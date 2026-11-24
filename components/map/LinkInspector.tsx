"use client";

// Board G07 on the index (FE-10): what is behind one link, from …/links/{id}: its evidence, which layer won (H2, H3)
// and the interval it is live in. Evidence snippets are the repo's text: shown in <code> as text, never linked.
import { ArrowRight, Check, ChevronLeft, GitCommitHorizontal } from "lucide-react";
import type { EvidenceKind, Layer, LinkDetail } from "@/lib/api";
import { useWsRead } from "@/lib/api/live";
import { opText, roles, siteLabel } from "./linkEvidence";
import "./map.css";

const EV: Record<EvidenceKind, string> = { parsed: "Parsed", contract: "Contract", name_match: "Name match", env: "Environment", history: "History", model: "Model" };
/** H3: the layer whose answer the map shows. */
const LAYER: Record<Layer, string> = {
  base: "Parsed from the source", config: "From cartograph.yaml", contract: "From a contract",
  correction: "From a correction (the override file)", overlay: "From a working-tree overlay",
};
const KIND = { imports: "Import", calls: "Call", http: "HTTP", event: "Event", db: "Table read" } as const;
/** The server's truncation is the cap; this is a guard for a very long note. */
const cut = (s: string) => (s.length > 400 ? `${s.slice(0, 399)}…` : s);

export const confText = (level: string, score: number) => `${level[0].toUpperCase()}${level.slice(1)} · ${score.toFixed(2)}`;

export default function LinkInspector({ ws, id, at, name, dateOf, onBack, onOpen }: {
  ws: string; id: string; at: string;
  name: (nodeId: string) => string;
  dateOf: (sha: string) => string | undefined;
  onBack: () => void;
  onOpen: (nodeId: string) => void;
}) {
  const r = useWsRead<LinkDetail>(ws, `links/${encodeURIComponent(id)}`, { at: at || "now" });
  const d = r.data;
  return (
    <section className="map-insp" aria-label="Link details">
      <button type="button" className="panel-back btn" onClick={onBack}><ChevronLeft size={14} aria-hidden /> Back</button>
      {r.error ? <p className="lm-none" role="alert">{r.error.what}. {r.error.try_}.</p> : !d ? <p className="lm-none" aria-busy="true">Loading the link…</p> : (
        <>
          <div className="map-chips">
            <span className="map-chip is-kind">{KIND[d.link.kind]}</span>
            <span className={`map-chip is-${d.link.confidence.level}`}>{confText(d.link.confidence.level, d.link.confidence.score)}</span>
            {d.layer === "correction" && <span className="map-chip is-fixed">Overridden</span>}
          </div>
          <h2 className="map-insp-title"><span className="lm-mono">{name(d.link.from)}</span><ArrowRight size={16} aria-hidden /><span className="lm-mono">{name(d.link.to)}</span></h2>
          {d.operation && <p className="map-sub"><code>{opText(d.operation)}</code></p>}
          {d.layer && (
            <>
              <span className="map-k">Which answer the map shows</span>
              <p className="map-sub">{LAYER[d.layer]}.</p>
            </>
          )}
          {/* HTTP links give their evidence by role, with the code; the rest give a short list. */}
          {roles(d).length > 0 ? roles(d).map((g) => (
            <div key={g.role}>
              <span className="map-k">{g.title}</span>
              <ul className="map-ev">
                {g.sites.map((s, i) => (
                  <li key={i}><Check size={15} aria-hidden />
                    <span><code>{siteLabel(s)}</code> <span className="map-note">{s.kind}</span>
                      {s.snippet && <pre className="map-snip">{s.snippet}</pre>}</span>
                  </li>
                ))}
              </ul>
            </div>
          )) : (
            <>
              <span className="map-k">Evidence · {d.link.confidence.evidence.length} {d.link.confidence.evidence.length === 1 ? "source" : "sources"}</span>
              <ul className="map-ev">
                {d.link.confidence.evidence.map((e, i) => (
                  <li key={i}><Check size={15} aria-hidden />
                    <span><b>{EV[e.kind]}</b> {e.file && <code>{e.file}{e.line ? `:${e.line}` : ""}</code>}<br /><span className="map-note">{cut(e.note)}</span></span>
                  </li>
                ))}
              </ul>
            </>
          )}
          <span className="map-k">History</span>
          <dl className="map-hist">
            <dt className="is-up">Opened</dt>
            <dd><GitCommitHorizontal size={13} aria-hidden /> <code>{d.opened.sha.slice(0, 7)}</code> · {d.opened.author}{dateOf(d.opened.sha) ? ` · ${dateOf(d.opened.sha)}` : ""}</dd>
            <dt>Status</dt>
            <dd>{d.closed ? <>Closed in <code>{d.closed.sha.slice(0, 7)}</code> · {d.closed.author}{dateOf(d.closed.sha) ? ` · ${dateOf(d.closed.sha)}` : ""}</> : "Live, not closed"}</dd>
            {d.replaced && <><dt>Replaces</dt><dd><code>{d.replaced}</code>, closed in the same commit</dd></>}
          </dl>
          <div className="map-actions">
            <button type="button" className="btn" onClick={() => onOpen(d.link.from)}>Open {name(d.link.from).split("/").pop()}</button>
            <button type="button" className="btn" onClick={() => onOpen(d.link.to)}>Open {name(d.link.to).split("/").pop()}</button>
          </div>
        </>
      )}
    </section>
  );
}
