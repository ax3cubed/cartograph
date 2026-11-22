"use client";

import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import Link from "next/link";
import { ArrowDownLeft, ArrowUpRight, Check, ChevronLeft, Copy, GitCommitHorizontal, MessageSquare, Orbit, Star } from "lucide-react";
import { Parts } from "./Ask";
import { stories } from "@/lib/ask";
import {
  SERVICES, WORKSPACE, ancestry, byId, commitBySha, descendants, metrics, ranking, series, type Metrics,
} from "@/lib/workspace";
import type { Correction } from "@/lib/model";
import { symbolById, symbolsIn } from "@/lib/demo/symbols";
import History from "./timeline/History";
import LinkInspector from "./map/LinkInspector";
import { symbolSides } from "./map/SymbolView";
import { serviceHue } from "./map/domains";
import { neighboursOf, type EffLink } from "./map/links";
import "./map/map.css";

const DAY = 86_400_000;
const fmtMonth = (t: number) => new Date(t).toLocaleString("en-US", { month: "short", timeZone: "UTC" });
const fmtDay = (iso: string) => new Date(iso + "T12:00:00Z").toLocaleString("en-GB", { day: "numeric", month: "short", year: "2-digit", timeZone: "UTC" });
const KIND: Record<string, string> = { workspace: "Workspace", service: "Service", module: "Module", file: "File" };

export function Delta({ v, unit = "" }: { v: number; unit?: string }) {
  if (v === 0) return <span className="delta is-flat">±0{unit}</span>;
  const Up = v > 0;
  return (
    <span className={`delta ${Up ? "is-up" : "is-down"}`}>
      {Up ? <ArrowUpRight size={13} strokeWidth={2.2} aria-hidden /> : <ArrowDownLeft size={13} strokeWidth={2.2} aria-hidden />}
      {Up ? "+" : "−"}{Math.abs(v)}{unit}
    </span>
  );
}

function AreaChart({ points }: { points: { t: number; v: number }[] }) {
  const W = 380, H = 170, L = 30, B = 24, T = 8, R = 16;
  const max = Math.max(4, ...points.map((p) => p.v));
  const top = Math.ceil(max / 4) * 4;
  const x = (i: number) => L + (i / (points.length - 1)) * (W - L - R);
  const y = (v: number) => T + (1 - v / top) * (H - T - B);
  const line = points.map((p, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(p.v).toFixed(1)}`).join(" ");
  const step = points.length > 8 ? 3 : 1;
  return (
    <svg className="chart" viewBox={`0 0 ${W} ${H}`} role="img" aria-label={`Trend: ${points.map((p) => p.v).join(", ")}`}>
      <defs>
        <linearGradient id="area" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="var(--chart)" stopOpacity=".75" />
          <stop offset="1" stopColor="var(--chart)" stopOpacity=".15" />
        </linearGradient>
      </defs>
      {[0, 2, 4].map((k) => {
        const v = (top / 4) * k;
        return (
          <g key={k}>
            <line className="grid" x1={L} x2={W} y1={y(v)} y2={y(v)} />
            <text className="axis" x={L - 8} y={y(v) + 4} textAnchor="end">{v}</text>
          </g>
        );
      })}
      <path d={`${line} L${x(points.length - 1)},${y(0)} L${x(0)},${y(0)} Z`} fill="url(#area)" />
      <path d={line} className="chart-line" />
      {points.map((p, i) => <circle key={i} className="chart-dot" cx={x(i)} cy={y(p.v)} r="3" />)}
      {points.map((p, i) => (i % step === (points.length - 1) % step ? (
        <text key={"m" + i} className="axis" x={x(i)} y={H - 5} textAnchor="middle">{fmtMonth(p.t)}</text>
      ) : null))}
    </svg>
  );
}

function Sparkline({ points }: { points: { t: number; v: number }[] }) {
  const W = 120, H = 28;
  const max = Math.max(1, ...points.map((p) => p.v)), min = Math.min(...points.map((p) => p.v));
  const span = Math.max(1, max - min);
  const d = points.map((p, i) => `${i ? "L" : "M"}${((i / (points.length - 1)) * W).toFixed(1)},${(H - 2 - ((p.v - min) / span) * (H - 4)).toFixed(1)}`).join(" ");
  return (
    <svg className="panel-spark" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" aria-hidden>
      <path d={d} />
    </svg>
  );
}

interface Props {
  scopeId: string;
  t: number;
  pinned: Set<string>;
  focused: boolean;
  onTogglePin: (id: string) => void;
  onOpen: (id: string) => void;
  onFocus: (id: string) => void;
  links: EffLink[];
  corrections: Correction[];
  edge: { a: string; b: string } | null;
  symbolId: string | null;
  onEdge: (e: { a: string; b: string } | null) => void;
  onCorrect: (link: EffLink | null, from?: string) => void;
  onUndo: (id: string) => void;
  onSymbol: (id: string | null) => void;
  onJump: (day: number) => void;
  /** "Ask about this" in the header. Hidden when absent. */
  onAskAbout?: (id: string) => void;
  /** Set after a citation in Ask opened this node: shows a "Back to answer" crumb. */
  onBackToAnswer?: () => void;
}

function Crumb({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button type="button" className="panel-back" onClick={onClick}>
      <ChevronLeft size={14} aria-hidden /> {label}
    </button>
  );
}

function SymbolPanel({ id, onOpen }: { id: string; onOpen: (id: string) => void }) {
  const sym = symbolById(id)!;
  const f = byId.get(sym.file)!;
  const { calls, callers } = symbolSides(id);
  const n = (g: { syms: unknown[] }[]) => g.reduce((a, x) => a + x.syms.length, 0);
  return (
    <section className="panel-body panel-sym" aria-label={`Details for ${sym.name}`}>
      <header className="panel-head">
        <div className="badge" aria-hidden>{sym.kind === "function" ? "fn" : "T"}</div>
        <div className="panel-title">
          <h2 className="is-code">{sym.name}{sym.kind === "function" ? "()" : ""}</h2>
          <p className="is-code">{f.name}:{sym.lines[0]}–{sym.lines[1]}</p>
        </div>
      </header>
      <pre className="map-code">{sym.sig}</pre>
      <div className="map-model">
        <span className="map-chip">Written by a model</span>
        <p>{sym.summary}</p>
      </div>
      <div className="kpis">
        <div className="kpi"><span className="kpi-row"><b>{n(callers)}</b></span><span className="kpi-label">Called by</span></div>
        <div className="kpi"><span className="kpi-row"><b>{n(calls)}</b></span><span className="kpi-label">Calls</span></div>
        <div className="kpi"><span className="kpi-row"><b>{sym.lines[1] - sym.lines[0] + 1}</b></span><span className="kpi-label">Lines</span></div>
      </div>
      <div className="map-chips"><span className="map-chip is-exact">From the parser</span></div>
      <p className="summary">Symbols show only in focus view, because the map stops at files. Calls and callers are grouped by file, so a function with 200 callers still fits.</p>
      <button type="button" className="link-btn" onClick={() => onOpen(sym.file)}>Show {f.name} on the map</button>
    </section>
  );
}

export default function Panel(p: Props) {
  const scroller = useRef<HTMLElement>(null);
  const node = byId.get(p.scopeId)!;
  const sym = p.symbolId ? symbolById(p.symbolId) : undefined;
  // A new selection or inspector starts at the top, not wherever the last one was scrolled to.
  useEffect(() => { scroller.current?.scrollTo({ top: 0 }); }, [p.scopeId, p.symbolId, p.edge?.a, p.edge?.b]);

  const back = `Back to ${node.name}`;
  return (
    <aside ref={scroller} className="panel" aria-label="Details">
      {p.onBackToAnswer && <div className="panel-crumbs"><Crumb label="Back to answer" onClick={p.onBackToAnswer} /></div>}
      {sym ? (
        <><div className="panel-crumbs"><Crumb label={back} onClick={() => p.onSymbol(null)} /></div><SymbolPanel id={sym.id} onOpen={p.onOpen} /></>
      ) : p.edge ? (
        <><div className="panel-crumbs"><Crumb label={back} onClick={() => p.onEdge(null)} /></div>
          <LinkInspector a={p.edge.a} b={p.edge.b} links={p.links} t={p.t} onClose={() => p.onEdge(null)} onOpen={p.onOpen} onCorrect={(l) => p.onCorrect(l)} /></>
      ) : <NodePanel {...p} />}
    </aside>
  );
}

const SECTIONS = [
  { key: "overview", label: "Overview" },
  { key: "connections", label: "Connections" },
  { key: "changes", label: "Changes" },
  { key: "corrections", label: "Corrections" },
] as const;

function NodePanel({ scopeId, t, pinned, focused, onTogglePin, onOpen, onFocus, links, corrections, onCorrect, onUndo, onSymbol, onJump, onAskAbout }: Props) {
  const uid = useId();
  const node = byId.get(scopeId)!;
  const isFile = node.kind === "file";
  const isWs = node.kind === "workspace";
  const [months, setMonths] = useState(12);
  const [trend, setTrend] = useState(false);
  const [tabByKind, setTab] = useState<keyof Metrics | null>(null);
  const [copied, setCopied] = useState(false);
  const [allChanges, setAllChanges] = useState(false);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const tabs: { key: keyof Metrics; label: string }[] = isFile
    ? [{ key: "fanIn", label: "Used by" }, { key: "fanOut", label: "Uses" }, { key: "churn", label: "Churn 90d" }]
    : [{ key: "links", label: "Links" }, { key: "cross", label: "Cross-service" }, { key: "churn", label: "Churn 90d" }];
  const tabIdx = Math.max(0, tabs.findIndex((x) => x.key === tabByKind));
  const tab = tabs[tabIdx].key;

  const now = metrics(scopeId, t), before = metrics(scopeId, t - 90 * DAY);
  const changes = stories(isWs ? null : scopeId, -Infinity, t);
  const rank = isFile ? [] : ranking(scopeId, t).slice(0, 8);
  const { dependsOn, usedBy } = neighboursOf(links, scopeId, t);
  const scope = descendants(scopeId);
  const mine = corrections.filter((c) => scope.has(c.target.split(">")[0]) || (c.to && scope.has(c.to)));
  const syms = symbolsIn(scopeId);
  const byCount = (m: Map<string, EffLink[]>) => [...m].sort((a, b) => b[1].length - a[1].length);
  const users = byCount(usedBy), deps = byCount(dependsOn);
  const strongest = [...deps, ...users].sort((a, b) => b[1].length - a[1].length).slice(0, 5);
  const color = isWs ? "var(--accent)" : serviceHue(node.service);
  const sid = (k: string) => `${uid}-${k}`;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(node.path);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch { /* clipboard blocked: the path stays visible to select by hand */ }
  };

  const jump = (k: string) => {
    const el = document.getElementById(sid(k)) as HTMLDetailsElement | null;
    if (!el) return;
    el.open = true;
    el.scrollIntoView({ block: "start", behavior: "smooth" });
    el.querySelector("summary")?.focus({ preventScroll: true });
  };

  // Tabs pattern: one tab stop, arrows move and select.
  const onTabKey = (e: KeyboardEvent) => {
    const move = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
    const to = e.key === "Home" ? 0 : e.key === "End" ? tabs.length - 1 : move !== undefined ? (tabIdx + move + tabs.length) % tabs.length : -1;
    if (to < 0) return;
    e.preventDefault();
    setTab(tabs[to].key);
    tabRefs.current[to]?.focus();
  };

  const nameList = (rows: [string, EffLink[]][], empty: string) => rows.length ? (
    <ol className="mini">
      {rows.map(([id, ls], i) => {
        const o = byId.get(id)!;
        return (
          <li key={id}>
            <span className="idx">{i + 1}</span>
            <span className="chip" style={{ ["--c" as string]: serviceHue(o.service) }} aria-hidden>{o.lang ?? "··"}</span>
            <button type="button" onClick={() => onOpen(id)} title={o.path}>{o.name}</button>
            <span className="num" aria-label={`${ls.length} ${ls.length === 1 ? "link" : "links"}`}>{ls.length}</span>
          </li>
        );
      })}
    </ol>
  ) : <p className="empty">{empty}</p>;

  const count: Record<string, number | null> = {
    overview: null,
    connections: users.length + deps.length,
    changes: changes.length,
    corrections: mine.length,
  };

  return (
    <div className="panel-node" style={{ ["--c" as string]: color }}>
      <div className="panel-sticky">
        <header className="panel-head">
          <div className="badge" aria-hidden>{node.lang ?? node.kind.slice(0, 2).toUpperCase()}</div>
          <div className="panel-title">
            <h2>{node.name}</h2>
            <p>
              <span className="panel-kind">{KIND[node.kind]}</span>
              {isWs ? ` · ${SERVICES.length} services` : node.kind === "module" || isFile ? ` in ${byId.get(node.service)!.name}` : ""}
            </p>
          </div>
          <div className="panel-actions">
            {!isWs && !focused && (
              <button type="button" className="icon-btn" aria-label="Focus on this node" title="Focus: show only what it depends on and what uses it (or double-click the node)" onClick={() => onFocus(scopeId)}>
                <Orbit size={16} aria-hidden />
              </button>
            )}
            {!isWs && (
              <button type="button" className="icon-btn" aria-pressed={pinned.has(scopeId)} aria-label="Pin" title={pinned.has(scopeId) ? "Unpin" : "Pin"} onClick={() => onTogglePin(scopeId)}>
                <Star size={16} fill={pinned.has(scopeId) ? "currentColor" : "none"} aria-hidden />
              </button>
            )}
            {!isWs && (
              <button type="button" className="icon-btn" aria-label={copied ? "Path copied" : "Copy path"} title="Copy path" onClick={copy}>
                {copied ? <Check size={16} aria-hidden /> : <Copy size={16} aria-hidden />}
              </button>
            )}
            {onAskAbout && (
              <button type="button" className="icon-btn" aria-label={`Ask about ${node.name}`} title="Ask about this" onClick={() => onAskAbout(scopeId)}>
                <MessageSquare size={16} aria-hidden />
              </button>
            )}
          </div>
        </header>
        <nav className="panel-jump" aria-label="Sections">
          {SECTIONS.map((s) => (
            <button key={s.key} type="button" onClick={() => jump(s.key)}>
              {s.label}{count[s.key] ? <span className="panel-n">{count[s.key]}</span> : null}
            </button>
          ))}
        </nav>
      </div>

      <div className="panel-body">
        <details id={sid("overview")} className="panel-sec" open>
          <summary><h3>Overview</h3></summary>
          {!isWs && (
            <nav aria-label="Location">
              <ol className="panel-loc">
                {ancestry(scopeId).map((a) => (
                  <li key={a.id}>
                    {a.id === scopeId
                      ? <span aria-current="location">{a.name}</span>
                      : <button type="button" onClick={() => onOpen(a.id)}>{a.name}</button>}
                  </li>
                ))}
              </ol>
            </nav>
          )}
          <code className="path">{node.path}</code>
          <p className="summary">{node.summary}</p>

          <div className="kpis" role="tablist" aria-label="Metric" onKeyDown={onTabKey}>
            {tabs.map(({ key, label }, i) => (
              <button
                key={key} type="button" role="tab" id={sid(`tab-${key}`)} aria-controls={sid("metric")}
                aria-selected={tab === key} tabIndex={tab === key ? 0 : -1} className="kpi"
                ref={(el) => { tabRefs.current[i] = el; }} onClick={() => setTab(key)}
              >
                <span className="kpi-row"><b>{now[key]}</b><Delta v={now[key] - before[key]} /></span>
                <span className="kpi-label">{label}</span>
              </button>
            ))}
          </div>
          <div role="tabpanel" id={sid("metric")} aria-labelledby={sid(`tab-${tab}`)} className="panel-metric">
            {trend ? (
              <>
                <div className="period">
                  <label htmlFor={sid("period")}>Period</label>
                  <select id={sid("period")} value={months} onChange={(e) => setMonths(+e.target.value)}>
                    <option value={6}>Last 6 months</option>
                    <option value={12}>Last 12 months</option>
                    <option value={18}>Last 18 months</option>
                  </select>
                </div>
                <AreaChart points={series(scopeId, t, tab, months)} />
              </>
            ) : (
              <div className="panel-sparkrow">
                <Sparkline points={series(scopeId, t, tab, 12)} />
                <span>Last 12 months</span>
              </div>
            )}
            <button type="button" className="link-btn" aria-expanded={trend} onClick={() => setTrend(!trend)}>{trend ? "Hide trend" : "Show trend"}</button>
          </div>

          {syms.length > 0 && (
            <>
              <h4>Symbols</h4>
              <div className="map-sym-list">
                {syms.map((y) => (
                  <button type="button" key={y.id} onClick={() => onSymbol(y.id)}>
                    {y.name}{y.kind === "function" ? "()" : ""}<span className="idx map-sym-kind">{y.kind}</span>
                  </button>
                ))}
              </div>
            </>
          )}
        </details>

        <details id={sid("connections")} className="panel-sec" open>
          <summary><h3>Connections</h3>{count.connections ? <span className="panel-n">{count.connections}</span> : null}</summary>
          <p className="panel-lead">What breaks if you change {isWs ? "the workspace" : node.name}, and what it needs.</p>
          {isFile ? (
            <>
              <h4>Used by · {users.length}</h4>
              {nameList(users.slice(0, 8), "Nothing uses this file at this date, so a change here breaks nothing else.")}
            </>
          ) : (
            <>
              <h4>Impact ranking{!isWs && <> in <em>{node.name}</em></>}</h4>
              {rank.length ? (
                <table className="rank">
                  <thead><tr><th scope="col">#</th><th scope="col">File</th><th scope="col">Used by</th><th scope="col">90d</th></tr></thead>
                  <tbody>
                    {rank.map((r, i) => (
                      <tr key={r.node.id}>
                        <td className="idx">{i + 1}</td>
                        <td><button type="button" onClick={() => onOpen(r.node.id)} title={r.node.path}>
                          <i style={{ ["--c" as string]: serviceHue(r.node.service) }} aria-hidden />{r.node.name}
                        </button></td>
                        <td className="num">{r.usedBy}</td>
                        <td><Delta v={r.delta} /></td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              ) : <p className="empty">No linked files at this date. Move the timeline later to see links.</p>}
            </>
          )}
          {!isWs && (
            <>
              <h4>Strongest links</h4>
              {nameList(strongest, "No links at this date.")}
              {!isFile && (
                <dl className="panel-counts">
                  <div><dt>Depends on</dt><dd>{deps.length}</dd></div>
                  <div><dt>Used by</dt><dd>{users.length}</dd></div>
                </dl>
              )}
            </>
          )}
        </details>

        <details id={sid("changes")} className="panel-sec" open>
          <summary><h3>Changes</h3>{changes.length ? <span className="panel-n">{changes.length}</span> : null}</summary>
          <h4>Relationship changes</h4>
          {changes.length ? (
            <>
              <ol className="changes">
                {(allChanges ? changes : changes.slice(0, 5)).map(({ sha, parts }) => {
                  const c = commitBySha.get(sha)!;
                  return (
                    <li key={sha}>
                      <GitCommitHorizontal size={15} className="ci" aria-hidden />
                      <div>
                        <p className="msg">{c.message}</p>
                        <p className="story"><Parts parts={parts} onOpen={onOpen} />.</p>
                        <p className="meta">
                          <Link href={`/impact#${c.sha}`} className="panel-sha" title="Open this commit's impact">{c.sha}</Link> · {c.author} · {fmtDay(c.date)}
                        </p>
                      </div>
                    </li>
                  );
                })}
              </ol>
              {changes.length > 5 && (
                <button type="button" className="link-btn" onClick={() => setAllChanges(!allChanges)}>
                  {allChanges ? "Show the latest 5" : `Show all ${changes.length}`}
                </button>
              )}
            </>
          ) : <p className="empty">No commit changed these links before this date. Move the timeline later to see changes.</p>}
          {!isWs && <History nodeId={scopeId} onJump={onJump} />}
        </details>

        <details id={sid("corrections")} className="panel-sec" open>
          <summary><h3>Corrections</h3>{mine.length ? <span className="panel-n">{mine.length}</span> : null}</summary>
          {mine.length ? (
            <ul className="map-corr">
              {mine.map((c) => (
                <li key={c.id}>
                  <div><code>{c.action}</code> {c.target.split(">").map((x) => byId.get(x)?.name ?? x).join(" → ")}{c.to ? ` → ${byId.get(c.to)?.name ?? c.to}` : ""}<p>{c.reason} · {c.by}</p></div>
                  <button type="button" className="link-btn" onClick={() => onUndo(c.id)}>Undo correction</button>
                </li>
              ))}
            </ul>
          ) : <p className="empty">No one has corrected links here. If the map is missing a link, add it here.</p>}
          <button type="button" className="btn panel-add" onClick={() => onCorrect(null, isFile ? scopeId : undefined)}>Add a missing link</button>
        </details>
      </div>
    </div>
  );
}
