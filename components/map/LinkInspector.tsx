"use client";

import { useState } from "react";
import { ArrowRight, Check, ChevronLeft, GitCommitHorizontal } from "lucide-react";
import { byId, commitBySha, isActive } from "@/lib/workspace";
import { chipText, type Evidence } from "@/lib/model";
import { KINDS, level, between, type EffLink } from "./links";
import { serviceHue } from "./domains";
import "./map.css";

const EV: Record<Evidence["kind"], string> = { parsed: "Parsed", contract: "Contract", "name-match": "Name match", env: "Environment", history: "History", model: "Model" };
const fmt = (iso: string) => new Date(iso + "T12:00:00Z").toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });

export function ConfChip({ l }: { l: EffLink }) {
  const c = l.confidence ?? { level: "exact" as const, score: 1, evidence: [] };
  return <span className={`map-chip is-${c.level}`}>{chipText(c)}</span>;
}

const Dot = ({ id }: { id: string }) => <i className="map-dot" style={{ background: serviceHue(byId.get(id)!.service) }} aria-hidden />;

/** G07: what is behind a line on the map. A bundle lists its links first. */
export default function LinkInspector({ a, b, links, t, onClose, onOpen, onCorrect }: {
  a: string; b: string; links: EffLink[]; t: number;
  onClose: () => void; onOpen: (id: string) => void; onCorrect: (l: EffLink) => void;
}) {
  const bundle = between(links, a, b, t);
  const [pick, setPick] = useState<number | null>(null);
  const l = bundle.length === 1 ? bundle[0] : pick !== null ? bundle[pick] : null;
  const A = byId.get(a)!, B = byId.get(b)!;

  if (!l) {
    return (
      <section className="panel-body map-insp" aria-label="Link details">
        <div className="map-insp-head"><h2><Dot id={a} />{A.name}<ArrowRight size={16} aria-hidden /><Dot id={b} />{B.name}</h2></div>
        <p className="map-sub">{bundle.length ? `${bundle.length} links` : "No links at this date"}. Pick one to see its evidence.</p>
        <ul className="map-list">
          {bundle.map((x, i) => (
            <li key={i}><button type="button" onClick={() => setPick(i)}>
              <span className="map-path">{byId.get(x.from)!.name} → {byId.get(x.to)!.name}</span>
              <span className="map-chip">{KINDS[x.type]}</span><ConfChip l={x} />
              {x.fix && <span className="map-chip is-fixed">Overridden</span>}
            </button></li>
          ))}
        </ul>
      </section>
    );
  }
  const F = byId.get(l.from)!, T = byId.get(l.to)!;
  const ev: Evidence[] = l.confidence?.evidence.length ? l.confidence.evidence : [{ kind: "parsed", note: `${F.name} refers to ${T.name} in its source.`, file: F.path }];
  const opened = commitBySha.get(l.added), closed = l.removed ? commitBySha.get(l.removed) : undefined;
  return (
    <section className="panel-body map-insp" aria-label="Link details">
      <div className="map-insp-head">
        <div className="map-chips">
          <span className="map-chip is-kind">{KINDS[l.type]}</span>
          {F.service !== T.service && <span className="map-chip">Cross-service</span>}
          <ConfChip l={l} />
          {l.fix && <span className="map-chip is-fixed">Overridden</span>}
        </div>
      </div>
      {bundle.length > 1 && <button type="button" className="panel-back" onClick={() => setPick(null)}><ChevronLeft size={14} aria-hidden /> All {bundle.length} links</button>}
      <h2 className="map-insp-title"><Dot id={l.from} />{F.name}<ArrowRight size={16} aria-hidden /><Dot id={l.to} />{T.name}</h2>
      <span className="map-k">Evidence · {ev.length} {ev.length === 1 ? "source" : "sources"}</span>
      <ul className="map-ev">
        {ev.map((e, i) => (
          <li key={i}><Check size={15} aria-hidden />
            <span><b>{EV[e.kind]}</b> {e.file && <code>{e.file}{e.line ? `:${e.line}` : ""}</code>}<br /><span className="map-note">{e.note}</span></span>
          </li>
        ))}
      </ul>
      {l.fix && (
        <div className="map-fixed"><b>Corrected by {l.fix.by}, {fmt(l.fix.at)}</b><span>{l.fix.action}: {l.fix.reason}</span></div>
      )}
      <span className="map-k">History</span>
      <dl className="map-hist">
        <dt className="is-up">Opened</dt>
        <dd>{opened ? <><GitCommitHorizontal size={13} aria-hidden /> <code>{opened.sha}</code> · {opened.author} · {fmt(opened.date)}</> : "Added by a correction"}</dd>
        <dt>Status</dt><dd>{isActive(l, t) ? "Active, not closed" : closed ? `Closed in ${closed.sha}, ${fmt(closed.date)}` : "Not active on this date"}</dd>
      </dl>
      <div className="map-actions">
        <button type="button" className="btn" onClick={() => onCorrect(l)}>Mark link as wrong</button>
        <button type="button" className="btn primary" onClick={() => onOpen(l.from)}>Open {F.name}</button>
        <button type="button" className="btn" onClick={() => onOpen(l.to)}>Open {T.name}</button>
      </div>
    </section>
  );
}
