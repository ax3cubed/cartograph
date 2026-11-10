"use client";

import { useState } from "react";
import { ArrowDownLeft, ArrowUpRight, Check, Copy, GitCommitHorizontal, Orbit, Star } from "lucide-react";
import { Parts } from "./Ask";
import { stories } from "@/lib/ask";
import {
  SERVICES, WORKSPACE, ancestry, byId, commitBySha, metrics, neighbours, ranking, series, serviceColor, type Metrics,
} from "@/lib/workspace";

const DAY = 86_400_000;
const fmtMonth = (t: number) => new Date(t).toLocaleString("en-US", { month: "short", timeZone: "UTC" });
const fmtDay = (iso: string) => new Date(iso + "T12:00:00Z").toLocaleString("en-GB", { day: "numeric", month: "short", year: "2-digit", timeZone: "UTC" });

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
  const W = 380, H = 170, L = 26, B = 22, T = 8, R = 16;
  const max = Math.max(4, ...points.map((p) => p.v));
  const top = Math.ceil(max / 4) * 4;
  const x = (i: number) => L + (i / (points.length - 1)) * (W - L - R);
  const y = (v: number) => T + (1 - v / top) * (H - T - B);
  const line = points.map((p, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(p.v).toFixed(1)}`).join(" ");
  const step = points.length > 8 ? 2 : 1;
  return (
    <svg className="chart" viewBox={`0 0 ${W} ${H}`} role="img" aria-label={`Trend: ${points.map((p) => p.v).join(", ")}`}>
      <defs>
        <linearGradient id="area" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="var(--chart)" stopOpacity=".75" />
          <stop offset="1" stopColor="var(--chart)" stopOpacity=".15" />
        </linearGradient>
      </defs>
      {[0, 1, 2, 3, 4].map((k) => {
        const v = (top / 4) * k;
        return (
          <g key={k}>
            <line className="grid" x1={L} x2={W} y1={y(v)} y2={y(v)} />
            <text className="axis" x={L - 8} y={y(v) + 3.5} textAnchor="end">{v}</text>
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

interface Props {
  scopeId: string;
  t: number;
  pinned: Set<string>;
  focused: boolean;
  onTogglePin: (id: string) => void;
  onOpen: (id: string) => void;
  onFocus: (id: string) => void;
}

export default function Panel({ scopeId, t, pinned, focused, onTogglePin, onOpen, onFocus }: Props) {
  const node = byId.get(scopeId)!;
  const isFile = node.kind === "file";
  const [months, setMonths] = useState(12);
  const [tabByKind, setTab] = useState<keyof Metrics | null>(null);
  const [copied, setCopied] = useState(false);
  const [allChanges, setAllChanges] = useState(false);

  const tabs: { key: keyof Metrics; label: string }[] = isFile
    ? [{ key: "fanIn", label: "Used by" }, { key: "fanOut", label: "Uses" }, { key: "churn", label: "Churn 90d" }]
    : [{ key: "links", label: "Links" }, { key: "cross", label: "Cross-service" }, { key: "churn", label: "Churn 90d" }];
  const tab = tabs.find((x) => x.key === tabByKind)?.key ?? tabs[0].key;

  const now = metrics(scopeId, t), before = metrics(scopeId, t - 90 * DAY);
  const changes = stories(scopeId === WORKSPACE ? null : scopeId, -Infinity, t);
  const rank = isFile ? [] : ranking(scopeId, t).slice(0, 8);
  const { dependsOn, usedBy } = neighbours(scopeId, t);
  const strongest = [...dependsOn, ...usedBy].sort((a, b) => b[1].length - a[1].length).slice(0, 5);
  const color = node.kind === "workspace" ? "var(--accent)" : serviceColor(node.service);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(node.path);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch { /* clipboard blocked: the path stays visible to select by hand */ }
  };

  return (
    <aside className="panel" aria-label={`Details for ${node.name}`} style={{ ["--c" as string]: color }}>
      <header className="panel-head">
        <div className="badge" aria-hidden>{node.lang ?? node.kind.slice(0, 2).toUpperCase()}</div>
        <div className="panel-title">
          <h2>{node.name}</h2>
          <p>{node.kind === "workspace" ? `${SERVICES.length} services` : node.kind[0].toUpperCase() + node.kind.slice(1)}{node.kind !== "service" && node.service ? ` · ${byId.get(node.service)!.name}` : ""}</p>
        </div>
        {node.kind !== "workspace" && (
          <div className="panel-actions">
            {!focused && (
              <button className="icon-btn" aria-label="Focus: show what it depends on and what uses it" title="Focus (or double-click the node)" onClick={() => onFocus(scopeId)}>
                <Orbit size={16} />
              </button>
            )}
            <button className="icon-btn" aria-pressed={pinned.has(scopeId)} aria-label={pinned.has(scopeId) ? "Unpin" : "Pin"} onClick={() => onTogglePin(scopeId)}>
              <Star size={16} fill={pinned.has(scopeId) ? "currentColor" : "none"} />
            </button>
            <button className="icon-btn" aria-label="Copy path" onClick={copy}>{copied ? <Check size={16} /> : <Copy size={16} />}</button>
          </div>
        )}
      </header>

      <code className="path">{node.path}</code>
      <p className="summary">{node.summary}</p>

      <div className="period">
        <span>Period</span>
        <select value={months} onChange={(e) => setMonths(+e.target.value)} aria-label="Chart period">
          <option value={6}>Last 6 months</option>
          <option value={12}>Last 12 months</option>
          <option value={18}>Last 18 months</option>
        </select>
      </div>

      <div className="kpis" role="tablist" aria-label="Metric">
        {tabs.map(({ key, label }) => (
          <button key={key} role="tab" aria-selected={tab === key} className="kpi" onClick={() => setTab(key)}>
            <span className="kpi-row"><b>{now[key]}</b><Delta v={now[key] - before[key]} /></span>
            <span className="kpi-label">{label}</span>
          </button>
        ))}
      </div>
      <AreaChart points={series(scopeId, t, tab, months)} />

      {node.kind !== "workspace" && (
        <div className="duo">
          <section>
            <h3>Lives in</h3>
            <ol className="crumbs">
              {ancestry(scopeId).map((a, i) => (
                <li key={a.id} style={{ paddingLeft: i * 10 }}>
                  <button className={a.id === scopeId ? "is-current" : ""} onClick={() => onOpen(a.id)}>{a.name}</button>
                </li>
              ))}
            </ol>
          </section>
          <section>
            <h3>Strongest links</h3>
            {strongest.length ? (
              <ol className="mini">
                {strongest.map(([id, ls], i) => {
                  const o = byId.get(id)!;
                  return (
                    <li key={id}>
                      <span className="idx">{i + 1}</span>
                      <span className="chip" style={{ ["--c" as string]: serviceColor(o.service) }} aria-hidden>{o.lang ?? "··"}</span>
                      <button onClick={() => onOpen(id)} title={o.path}>{o.name}</button>
                      <span className="num">{ls.length}</span>
                    </li>
                  );
                })}
              </ol>
            ) : <p className="empty">No links at this date.</p>}
          </section>
        </div>
      )}

      <section className="block">
        <div className="block-head">
          <h3>Relationship changes</h3>
          {changes.length > 5 && <button className="link-btn" onClick={() => setAllChanges(!allChanges)}>{allChanges ? "Show fewer" : `All ${changes.length}`}</button>}
        </div>
        {changes.length ? (
          <ol className="changes">
            {(allChanges ? changes : changes.slice(0, 5)).map(({ sha, parts }) => {
              const c = commitBySha.get(sha)!;
              return (
                <li key={sha}>
                  <GitCommitHorizontal size={15} className="ci" aria-hidden />
                  <div>
                    <p className="msg">{c.message}</p>
                    <p className="story"><Parts parts={parts} onOpen={onOpen} />.</p>
                    <p className="meta"><code>{c.sha}</code> · {c.author} · {fmtDay(c.date)}</p>
                  </div>
                </li>
              );
            })}
          </ol>
        ) : <p className="empty">No commits have touched these links before this date.</p>}
      </section>

      {!isFile && (
        <section className="block">
          <div className="block-head"><h3>Impact ranking{node.kind !== "workspace" && <> in <em>{node.name}</em></>}</h3></div>
          <table className="rank">
            <thead><tr><th>#</th><th>File</th><th>Used by</th><th>90d</th></tr></thead>
            <tbody>
              {rank.map((r, i) => (
                <tr key={r.node.id} onClick={() => onOpen(r.node.id)}>
                  <td className="idx">{i + 1}</td>
                  <td><button onClick={(e) => (e.stopPropagation(), onOpen(r.node.id))} title={r.node.path}>
                    <i style={{ ["--c" as string]: serviceColor(r.node.service) }} aria-hidden />{r.node.name}
                  </button></td>
                  <td className="num">{r.usedBy}</td>
                  <td><Delta v={r.delta} /></td>
                </tr>
              ))}
            </tbody>
          </table>
          {!rank.length && <p className="empty">No linked files at this date.</p>}
        </section>
      )}
    </aside>
  );
}
