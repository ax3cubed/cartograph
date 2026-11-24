"use client";

import { useId, useState, type ReactNode } from "react";
import Link from "next/link";
import { CalendarClock, Check, ChevronRight, Copy, MapPin, ScanSearch, X } from "lucide-react";
import { AiMark } from "@/components/brand/AiMark";
import type { Answer, Handle, Part } from "@/lib/ask";
import { byId, commitBySha, dayMs, serviceColor } from "@/lib/workspace";
import { fmtDay, isoDay, partsText } from "./format";
import { fmtNum } from "@/lib/date";

/** Where a node or an answer opens. On the map these are callbacks; elsewhere they are links to the map. */
export interface Nav {
  onOpen: (id: string) => void;
  /** Off the map: the map URL for a node (`/#focus=<id>&back=…`). */
  hrefFor?: (id: string) => string;
}

/** A node as a link or a button, depending on where the chat is. */
function NodeLink({ id, nav, className, title, style, children }: { id: string; nav: Nav; className: string; title?: string; style?: React.CSSProperties; children: ReactNode }) {
  return nav.hrefFor
    ? <Link href={nav.hrefFor(id)} className={className} title={title} style={style}>{children}</Link>
    : <button type="button" className={className} title={title} style={style} onClick={() => nav.onOpen(id)}>{children}</button>;
}

/** Engine answer text with nodes and commits as clickable evidence. */
function EngineParts({ parts, nav }: { parts: Part[]; nav: Nav }) {
  return (
    <>
      {parts.map((p, i) => {
        if (typeof p === "string") return <span key={i}>{p}</span>;
        if ("node" in p) {
          const n = byId.get(p.node)!;
          return <NodeLink key={i} id={n.id} nav={nav} className="ref" title={n.path} style={{ ["--c" as string]: serviceColor(n.service) }}>{n.name}</NodeLink>;
        }
        const c = commitBySha.get(p.commit)!;
        return <code key={i} className="ref-commit" title={`${c.message} · ${c.author} · ${c.date}`}>{c.sha}</code>;
      })}
    </>
  );
}

/** A checked citation. Links to the map come only from here (decision G6), never from the model's text. */
function Cite({ h, nav }: { h: Handle; nav: Nav }) {
  const icon = <Check size={12} aria-hidden />;
  if (h.kind === "commit") {
    const c = commitBySha.get(h.id)!;
    return <code className="ask-cite" title={`${c.message} · ${c.author} · ${c.date}`}>{icon}{c.sha}</code>;
  }
  const node = h.kind === "node" ? h.id : h.id.split(">")[0];
  return <NodeLink id={node} nav={nav} className="ask-cite" title={`${h.label} (${h.kind}, ${h.handle})`}>{icon}{h.label}</NodeLink>;
}

/** Plain text of an answer, for Copy. */
export function answerText(a: Answer) {
  const body = a.checked && a.by
    ? a.checked.paragraphs.map((p) => p.map((s) => s.text).join(" ")).join("\n\n")
    : a.lines.map(partsText).join("\n");
  return `${a.title}\n\n${body}`;
}

function StepMark({ running }: { running: boolean }) {
  return <span className={`ask-step-mark${running ? " is-running" : ""}`} aria-hidden>{running ? <i /> : <Check size={12} />}</span>;
}

/** "How I worked this out": one quiet row that opens to the steps, showing the step in progress while it streams. */
function Steps({ a, working, model }: { a: Answer; working: boolean; model?: string }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const items = [...a.steps];
  if (a.context) items.push(`Gathered ${a.context.handles.length} pieces of evidence, ${fmtNum(a.context.tokens)} tokens.`);
  if (working) items.push(`Wording the answer with ${model ?? "the model"}`);
  else if (a.checked && a.by) items.push(`Checked ${a.checked.stats.cited} citations and kept ${a.checked.stats.kept}.`);
  if (!items.length) return null;
  return (
    <div className="ask-steps">
      <button type="button" className="ask-steps-row" aria-expanded={open} aria-controls={id} onClick={() => setOpen((o) => !o)}>
        <StepMark running={working} />
        <span className="ask-steps-sum">
          <b>{working ? `${items[items.length - 1]}…` : "How I worked this out"}</b>
          <span aria-hidden> · </span>{working ? `step ${items.length}` : `${items.length} steps`}
        </span>
        <ChevronRight size={14} aria-hidden className="ask-steps-chev" />
      </button>
      {open && (
        <ol id={id} className="ask-steps-list">
          {items.map((s, i) => <li key={i}><StepMark running={working && i === items.length - 1} />{s}</li>)}
        </ol>
      )}
    </div>
  );
}

const EVIDENCE_SHOWN = 6;

/** The nodes an answer lights on the map, as chips that open them there. */
function Evidence({ nodes, nav }: { nodes: string[]; nav: Nav }) {
  const ids = [...new Set(nodes)].filter((id) => byId.has(id));
  if (!ids.length) return null;
  return (
    <div className="ask-ev" aria-label="Evidence on the map" role="group">
      <span className="ask-ev-k">Evidence</span>
      {ids.slice(0, EVIDENCE_SHOWN).map((id) => {
        const n = byId.get(id)!;
        return <NodeLink key={id} id={id} nav={nav} className="ref" title={`${n.path} · open on the map`} style={{ ["--c" as string]: serviceColor(n.service) }}>{n.name}</NodeLink>;
      })}
      {ids.length > EVIDENCE_SHOWN && <span className="ask-ev-more">+{ids.length - EVIDENCE_SHOWN} more</span>}
    </div>
  );
}

/** Copy, re-ask as of another date, show on map, inspect retrieval. */
function Actions({ a, t, lit, onShow, showHref, onInspect, onReask }: {
  a: Answer; t: number; lit: boolean; onShow: () => void; showHref?: string; onInspect: () => void; onReask: (asOf: number) => void;
}) {
  const [copied, setCopied] = useState(false);
  const [dating, setDating] = useState(false);
  const [day, setDay] = useState(isoDay(a.asOf ?? t));
  const dateId = useId();
  const copy = async () => {
    try { await navigator.clipboard.writeText(answerText(a)); setCopied(true); setTimeout(() => setCopied(false), 1600); } catch { /* clipboard blocked */ }
  };
  return (
    <>
      <div className="ask-acts">
        <button type="button" className="ask-act" onClick={copy} aria-live="polite">
          {copied ? <Check size={14} aria-hidden /> : <Copy size={14} aria-hidden />}{copied ? "Copied" : "Copy"}
        </button>
        <button type="button" className="ask-act" aria-expanded={dating} onClick={() => setDating((d) => !d)}>
          <CalendarClock size={14} aria-hidden />As of another date
        </button>
        {a.nodes.length > 0 && (showHref
          ? <Link className="ask-act" href={showHref}><MapPin size={14} aria-hidden />Show on map</Link>
          : <button type="button" className="ask-act" aria-pressed={lit} onClick={onShow}><MapPin size={14} aria-hidden />{lit ? "Shown on map" : "Show on map"}</button>)}
        {a.context && <button type="button" className="ask-act" onClick={onInspect}><ScanSearch size={14} aria-hidden />Inspect retrieval</button>}
      </div>
      {dating && (
        <form className="ask-reask" onSubmit={(e) => { e.preventDefault(); if (day) { onReask(dayMs(day)); setDating(false); } }}>
          <label htmlFor={dateId}>Ask again as of</label>
          <input id={dateId} type="date" value={day} onChange={(e) => setDay(e.target.value)} />
          <button type="submit" className="ask-small" disabled={!day}>Re-ask</button>
          <button type="button" className="ask-icon" aria-label="Cancel" onClick={() => setDating(false)}><X size={14} aria-hidden /></button>
        </form>
      )}
    </>
  );
}

/** One exchange: the person's compact question, then the answer with its mark, steps, text, evidence and actions. */
export function TurnView({ a, model, working, lit, t, nav, onShow, showHref, onInspect, onReask }: {
  a: Answer; model?: string; working: boolean; lit: boolean; t: number; nav: Nav;
  onShow: () => void; showHref?: string; onInspect: () => void; onReask: (asOf: number) => void;
}) {
  const dropped = a.checked?.paragraphs.flat().flatMap((s) => s.cites).filter((c) => !c.ok) ?? [];
  const modelWorded = !!(a.by && a.checked);
  const byline = modelWorded ? `${a.by} · worded from the map's evidence` : working ? `Answered from the map · asking ${model}` : "Answered from the map";
  return (
    <li className={`ask-turn-live${lit ? " is-lit" : ""}`}>
      <p className="ask-q">{a.q}</p>
      <article className="answer ask-msg" aria-busy={working}>
        <div className="ask-by">
          <span className="ask-avatar"><AiMark size={16} active={working} /></span>
          <span className="ask-by-name">{byline}</span>
          {a.asOf !== undefined && <span className="ask-chip">As of {fmtDay(a.asOf)}</span>}
        </div>
        <h3 className="ask-title">{a.title}</h3>
        <Steps a={a} working={working} model={model} />
        {a.notice && <p className="ask-notice">{a.notice}</p>}
        <div className="answer-body">
          {modelWorded ? a.checked!.paragraphs.map((p, i) => (
            <p key={i} className="ask-text">
              {p.map((s, j) => (
                <span key={j} className={!working && !s.supported ? "ask-unsupported" : undefined}>
                  {s.text}
                  {s.cites.filter((c) => c.ok).map((c) => <Cite key={c.handle} h={a.context!.handles.find((h) => h.handle === c.handle)!} nav={nav} />)}
                  {!working && !s.supported && <span className="ask-flag">No checked source</span>}{" "}
                </span>
              ))}
            </p>
          )) : a.lines.map((l, i) => <p key={i} className="ask-text"><EngineParts parts={l} nav={nav} /></p>)}
        </div>
        {!working && dropped.length > 0 && (
          <div className="ask-dropped" role="status">
            <X size={14} aria-hidden />
            <span>{dropped.length} citation{dropped.length > 1 ? "s were" : " was"} dropped: {dropped.map((c, i) => <span key={c.handle}>{i ? ", " : ""}<s>[{c.handle}]</s> {c.why}</span>)}.</span>
          </div>
        )}
        {(a.checked && a.context && a.by || a.context?.ultra) && (
          <div className="ask-meta">
            {a.checked && a.by && <span className="ask-chip ok"><Check size={12} aria-hidden />{a.checked.stats.kept} of {a.checked.stats.cited} citations kept</span>}
            {a.context?.ultra && <span className="ask-chip acc">{a.context.leftOut} nodes left out</span>}
          </div>
        )}
        <Evidence nodes={a.nodes} nav={nav} />
        {working
          ? <p className="ask-working">Still writing. Stop keeps what has arrived.</p>
          : <Actions a={a} t={t} lit={lit} onShow={onShow} showHref={showHref} onInspect={onInspect} onReask={onReask} />}
      </article>
    </li>
  );
}
