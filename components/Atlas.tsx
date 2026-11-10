"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ArrowLeft, ArrowRight, ChevronDown, ChevronLeft, ChevronRight, ChevronUp, Search, Star,
} from "lucide-react";
import Graph, { type Depth, type Layout } from "./Graph";
import Ask from "./Ask";
import type { Answer } from "@/lib/ask";
import Panel, { Delta } from "./Panel";
import {
  COMMITS, NODES, SERVICES, WORKSPACE, byId, children, dayMs, metrics, serviceColor, type LinkType,
} from "@/lib/workspace";

const DAY = 86_400_000;
const TODAY = dayMs("2026-11-07"); // ponytail: pinned "now" for the mock; use Date.now() with real data
const YEARS = [2025, 2026];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const yearStart = (y: number) => Date.UTC(y, 0, 1, 12);
const yearEnd = (y: number) => Date.UTC(y, 11, 31, 12);

export default function Atlas() {
  const [layout, setLayout] = useState<Layout>("clusters");
  const [depthSetting, setDepthSetting] = useState<Depth | "auto">("auto");
  const [autoDepth, setAutoDepth] = useState<Depth>(2);
  const onDepth = useCallback((d: Depth) => setAutoDepth(d), []);
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [focusId, setFocusId] = useState<string | null>(null);
  const [scopeId, setScopeId] = useState<string>(WORKSPACE);
  const [t, setT] = useState(TODAY);
  const [typeFilter, setTypeFilter] = useState<LinkType | "all">("all");
  const [minUsedBy, setMinUsedBy] = useState(0);
  const [expanded, setExpanded] = useState<Set<string>>(new Set(["billing"]));
  const [pinned, setPinned] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (window.matchMedia("(max-width: 1180px)").matches) setExpanded(new Set()); // tree starts collapsed on narrow screens
    try { setPinned(new Set(JSON.parse(localStorage.getItem("cartograph:pins") ?? "[]"))); } catch { /* storage unavailable */ }
  }, []);
  const togglePin = (id: string) => {
    const next = new Set(pinned);
    next.has(id) ? next.delete(id) : next.add(id);
    setPinned(next);
    try { localStorage.setItem("cartograph:pins", JSON.stringify([...next])); } catch { /* storage unavailable */ }
  };

  // click = select (panel + emphasis), double-click / Focus button = open the ego view
  const select = (id: string) => {
    if (focusId) return open(id);
    setScopeId(scopeId === id ? WORKSPACE : id);
  };
  const open = (id: string) => {
    setScopeId(id);
    if (id === WORKSPACE) return back();
    setFocusId(id);
  };
  const back = () => setFocusId(null);
  const showAnswer = (a: Answer | null) => {
    setAnswer(a);
    if (a) setFocusId(null); // answers are shown on the map, where the evidence lives
  };
  const highlight = useMemo(() => (answer && answer.nodes.length ? { nodes: new Set(answer.nodes), links: new Set(answer.links) } : null), [answer]);

  const year = new Date(t).getUTCFullYear();
  const setYear = (y: number) => setT(Math.min(TODAY, Math.max(yearStart(y), Math.min(t + (y - year) * 365 * DAY, yearEnd(y)))));
  const prevCommit = [...COMMITS].reverse().find((c) => dayMs(c.date) < t);
  const nextCommit = COMMITS.find((c) => dayMs(c.date) > t && dayMs(c.date) <= TODAY);
  const head = [...COMMITS].reverse().find((c) => dayMs(c.date) <= t);
  const pct = (x: number) => ((x - yearStart(year)) / (yearEnd(year) - yearStart(year))) * 100;

  const search = (v: string) => {
    setQuery(v);
    const hit = NODES.find((n) => n.path === v || n.name === v);
    if (hit) { open(hit.id); setQuery(""); }
  };

  const focusNode = focusId ? byId.get(focusId)! : null;

  return (
    <div className="app">
      <header className="top">
        <div className="brand">
          <svg viewBox="0 0 32 32" aria-hidden><circle cx="16" cy="16" r="14" /><circle cx="16" cy="16" r="4.5" /><circle cx="25" cy="10" r="2.6" /><circle cx="8" cy="22" r="2.6" /><path d="M16 16 25 10M16 16 8 22" /></svg>
          <span>cartograph</span>
        </div>
        <div className="nav-field">
          <span>Workspace</span>
          <b>{WORKSPACE} · {SERVICES.length} repos</b>
        </div>
        <label className="search">
          <Search size={16} aria-hidden />
          <input
            list="nodes"
            value={query}
            onChange={(e) => search(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== "Enter") return;
              const q = query.toLowerCase();
              const hit = NODES.find((n) => n.name.toLowerCase().includes(q) || n.path.toLowerCase().includes(q));
              if (q && hit) { open(hit.id); setQuery(""); }
            }}
            placeholder="Search service, module or file"
            aria-label="Search the workspace"
          />
          <datalist id="nodes">{NODES.map((n) => <option key={n.id} value={n.path}>{n.kind}</option>)}</datalist>
        </label>
        <details className="pins">
          <summary><Star size={15} fill="var(--accent)" stroke="var(--accent)" aria-hidden /> {pinned.size} pinned</summary>
          <div className="pins-menu">
            {pinned.size ? [...pinned].map((id) => (
              <button key={id} onClick={(e) => { open(id); (e.currentTarget.closest("details") as HTMLDetailsElement).open = false; }}>
                <i style={{ ["--c" as string]: serviceColor(byId.get(id)!.service) }} aria-hidden />{byId.get(id)!.name}
              </button>
            )) : <p>Pin a node from its panel to keep it here.</p>}
          </div>
        </details>
      </header>

      <main className="stage">
        <nav className="rail" aria-label="Workspace tree">
          {SERVICES.map((s) => (
            <TreeRow key={s.id} id={s.id} depth={0} t={t} scopeId={scopeId} expanded={expanded}
              onToggle={(id) => { const n = new Set(expanded); n.has(id) ? n.delete(id) : n.add(id); setExpanded(n); }}
              onPick={select} />
          ))}
        </nav>

        <section className="canvas" aria-label="Map">
          <div className="canvas-bar">
            {focusNode ? (
              <div className="focus-title">
                <button className="round-btn is-primary" onClick={back} aria-label="Back to map"><ChevronLeft size={18} /></button>
                <h1>{focusNode.name}</h1>
                <code>{focusNode.path}</code>
              </div>
            ) : (
              <div className="bar-left">
                <div className="seg" role="tablist" aria-label="Layout">
                  {(["clusters", "adaptive", "ontology"] as const).map((v) => (
                    <button key={v} role="tab" aria-selected={layout === v} onClick={() => setLayout(v)}
                      title={v === "adaptive" ? "Links act like springs. Drag nodes to rearrange." : undefined}>
                      {v === "clusters" ? "Network" : v === "adaptive" ? "Adaptive" : "Ontology"}
                    </button>
                  ))}
                </div>
                <label className="field depth">
                  <span>Detail</span>
                  <select value={depthSetting} onChange={(e) => setDepthSetting(e.target.value === "auto" ? "auto" : (+e.target.value as Depth))}>
                    <option value="auto">Auto · {["", "services", "modules", "files"][autoDepth]}</option>
                    <option value={1}>Services</option>
                    <option value={2}>Modules</option>
                    <option value={3}>Files</option>
                  </select>
                </label>
              </div>
            )}
            <div className="filters">
              <label className="field">
                <span>Min. used by</span>
                <span className="range-row">
                  <b>{minUsedBy}</b>
                  <input type="range" min={0} max={5} value={minUsedBy} onChange={(e) => setMinUsedBy(+e.target.value)} aria-label="Hide files used by fewer than" />
                </span>
              </label>
              <label className="field">
                <span>Link type</span>
                <select value={typeFilter} onChange={(e) => setTypeFilter(e.target.value as LinkType | "all")}>
                  <option value="all">All</option>
                  <option value="imports">Imports</option>
                  <option value="calls">Calls</option>
                  <option value="http">HTTP</option>
                  <option value="event">Events</option>
                  <option value="db">Database</option>
                </select>
              </label>
            </div>
          </div>

          <Graph focus={!!focusId} layout={layout} depthSetting={depthSetting} t={t} focusId={focusId}
            scopeId={scopeId === WORKSPACE ? null : scopeId} highlight={highlight}
            typeFilter={typeFilter} minUsedBy={minUsedBy} onOpen={open} onSelect={select} onDepth={onDepth} />

          <Ask t={t} answer={answer} onAnswer={showAnswer} onOpen={select} />

          <div className="legend" aria-label="Legend">
            <span><i className="lg-added" />Added in last 30 days</span>
            <span><i className="lg-removed" />Removed in last 30 days</span>
          </div>
          {head && (
            <p className="head-commit">
              <span>HEAD at this date</span>
              <code>{head.sha}</code> {head.message}
            </p>
          )}
        </section>

        <Panel scopeId={focusId ?? scopeId} t={t} pinned={pinned} focused={!!focusId} onTogglePin={togglePin} onOpen={select} onFocus={open} />
      </main>

      <footer className="timeline">
        <div className="year">
          <span>Year</span>
          <b>{year}</b>
          <div className="year-step">
            <button onClick={() => setYear(year + 1)} disabled={year >= YEARS[YEARS.length - 1]} aria-label="Next year"><ChevronUp size={13} /></button>
            <button onClick={() => setYear(year - 1)} disabled={year <= YEARS[0]} aria-label="Previous year"><ChevronDown size={13} /></button>
          </div>
        </div>
        <div className="steps">
          <button className="round-btn is-ghost" disabled={!prevCommit} onClick={() => prevCommit && setT(dayMs(prevCommit.date))} aria-label="Previous commit" title={prevCommit?.message}><ArrowLeft size={18} /></button>
          <button className="round-btn is-ghost" disabled={!nextCommit} onClick={() => nextCommit && setT(dayMs(nextCommit.date))} aria-label="Next commit" title={nextCommit?.message}><ArrowRight size={18} /></button>
        </div>
        <div className="ruler">
          <div className="months" aria-hidden>{MONTHS.map((m) => <span key={m}>{m}</span>)}</div>
          <div className="ticks" aria-hidden>
            {Array.from({ length: 49 }, (_, i) => <i key={i} className={i % 4 === 0 ? "major" : ""} style={{ left: `${(i / 48) * 100}%` }} />)}
            {COMMITS.filter((c) => c.date.startsWith(String(year))).map((c) => (
              <b key={c.sha} className={dayMs(c.date) <= t ? "is-past" : ""} style={{ left: `${pct(dayMs(c.date))}%`, ["--c" as string]: serviceColor(c.repo) }} title={c.message} />
            ))}
          </div>
          <div className="bubble" style={{ left: `${pct(t)}%` }} aria-hidden>
            {new Date(t).toLocaleString("en-GB", { month: "short", day: "numeric", timeZone: "UTC" })}
          </div>
          <input
            type="range"
            className="scrub"
            min={yearStart(year)}
            max={yearEnd(year)}
            step={DAY}
            value={t}
            onChange={(e) => setT(Math.min(TODAY, +e.target.value))}
            aria-label="Date"
            aria-valuetext={new Date(t).toDateString()}
          />
        </div>
      </footer>
    </div>
  );
}

function TreeRow({ id, depth, t, scopeId, expanded, onToggle, onPick }: {
  id: string; depth: number; t: number; scopeId: string; expanded: Set<string>;
  onToggle: (id: string) => void; onPick: (id: string) => void;
}) {
  const n = byId.get(id)!;
  const kids = n.kind === "file" ? [] : children(id);
  const m = useMemo(() => metrics(id, t), [id, t]);
  const was = useMemo(() => metrics(id, t - 90 * DAY), [id, t]);
  const isOpen = expanded.has(id);
  return (
    <div className={`tree d${depth}`}>
      <div className={`row ${scopeId === id ? "is-selected" : ""}`} style={{ ["--c" as string]: serviceColor(n.service) }}>
        {kids.length ? (
          <button className="twist" aria-expanded={isOpen} aria-label={`${isOpen ? "Collapse" : "Expand"} ${n.name}`} onClick={() => onToggle(id)}>
            <ChevronRight size={14} />
          </button>
        ) : <span className="twist" />}
        <button className="row-main" onClick={() => onPick(id)} aria-current={scopeId === id || undefined}>
          <i className={n.kind === "file" ? "ring" : "dot"} aria-hidden />
          <span className="row-name">{n.name}</span>
          <span className="num" title="Active links">{m.links}</span>
          <Delta v={m.links - was.links} />
        </button>
      </div>
      {isOpen && kids.map((k) => (
        <TreeRow key={k.id} id={k.id} depth={depth + 1} t={t} scopeId={scopeId} expanded={expanded} onToggle={onToggle} onPick={onPick} />
      ))}
    </div>
  );
}
