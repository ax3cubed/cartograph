"use client";

import Link from "next/link";
import { Gauge } from "lucide-react";
import { getContracts, getCoverage, getEntrypoints, getFreshness, getRepos } from "@/lib/source";
import { NODES } from "@/lib/workspace";
import { UNRESOLVED } from "@/lib/demo/views-extra";
import { AsideTop, Cell, Chip, KeyHint, NodeLink, Page, ViewSplit, mapHref, pathOf, useHashState, useRoving } from "./shared";
import { specRisk } from "./risk";
import "./coverage.css";

const BACK = "/coverage";
/** Each fix button is named for the place it opens. */
const FIX: Record<string, { label: string; href: string }> = {
  imports: { label: "Open indexing settings", href: "/settings/indexing" },
  calls: { label: "Open environments", href: "/settings/contracts" },
  handlers: { label: "Open contracts", href: "/contracts#gateway" },
  consumers: { label: "Open entrypoints", href: "/entrypoints" },
};
const pctOf = (d: number, t: number) => (t ? (d / t) * 100 : 100);
const fmt = (p: number) => `${+p.toFixed(1)}%`;
const sentence = (s: string) => { const t = s.replace(/^\d+\s+/, ""); return t[0].toUpperCase() + t.slice(1); };

interface Ref { at: string; raw: string; node?: string }
const nodeAt = (at: string) => NODES.find((n) => n.path === at.split(":")[0])?.id;
/** The references behind a gap. Each list is as long as the gap's count. */
function refs(id: string): Ref[] {
  if (id === "handlers") return getContracts().flatMap((s) => s.operations.filter((o) => !o.handler).map((o) => ({ at: s.file, raw: `${o.method} ${o.path}` })));
  if (id === "consumers") return getEntrypoints().filter((e) => e.consumers === 0).map((e) => ({ at: pathOf(e.node), raw: e.label, node: e.node }));
  return (UNRESOLVED[id] ?? []).map((u) => ({ ...u, node: nodeAt(u.at) }));
}

export default function Coverage() {
  const rows = getCoverage();
  const gaps = rows.filter((r) => r.gap && r.total > r.done).sort((a, b) => b.total - b.done - (a.total - a.done));
  const h = useHashState();
  const sel = gaps.find((g) => g.id === h.get("row")) ?? gaps[0];
  const below = rows.filter((r) => r.target !== undefined && pctOf(r.done, r.total) < r.target).length;
  const first = getFreshness().state === "none" || getRepos().length === 0;
  const risky = getContracts().map((s) => ({ s, r: specRisk(s) })).filter((x) => x.r.mismatch || x.r.breaking);
  const riskN = risky.reduce((n, x) => n + x.r.mismatch + x.r.breaking, 0);
  const list = sel ? refs(sel.id) : [];
  const n = sel ? sel.total - sel.done : 0;
  const first1 = list.find((r) => r.node)?.node;
  const rv = useRoving(gaps.map((g) => g.id), sel?.id, (row) => h.set({ row }), (id) => { const f = refs(id).find((r) => r.node)?.node; return f ? mapHref(f, BACK) : undefined; });

  return (
    <Page icon={Gauge} when={first} empty={{ title: "Building your first index", body: "Coverage shows what the map can and can't see once the first index finishes.", action: { label: "See indexing settings", href: "/settings/indexing" } }}>
      <ViewSplit id="cov" title="Coverage: what the map can and can't see" purpose="Unresolved references are kept, so each number traces back to a file and line." asideLabel="Unresolved references"
        actions={<span aria-live="polite"><Chip tone={below || riskN ? "warn" : "ok"}>{below ? `${below} below target` : "All targets met"}</Chip></span>}
        toolbar={
          <ul className="cov-mets">
            {rows.map((r) => {
              const p = pctOf(r.done, r.total);
              const miss = r.target !== undefined && p < r.target;
              return (
                <li key={r.id} className="cov-met">
                  <span className="cov-label">{r.label}</span>
                  <b>{fmt(p)}</b>
                  <div className="cov-bar" role="progressbar" aria-label={r.label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(p)}><i style={{ width: `${p}%` }} data-miss={miss || undefined} /></div>
                  <span className="view-muted cov-small">{r.done.toLocaleString()} of {r.total.toLocaleString()}{r.target !== undefined ? ` · target ${r.target}%` : ""}</span>
                  <span className={`cov-small cov-status ${r.target === undefined ? "view-muted" : miss ? "view-warn" : "cov-ok"}`}>{r.target === undefined ? "No target set" : miss ? "Below target" : "Meets target"}</span>
                </li>
              );
            })}
            <li className="cov-met">
              <span className="cov-label">Contract risks</span>
              <b>{riskN}</b>
              <span className="cov-small">{riskN ? "Schema mismatches and breaking changes" : "No mismatches or breaking changes"}</span>
              {risky.map(({ s, r }) => (
                <Link key={s.id} className="cov-small view-link" href={`/contracts#${s.id}`}>{s.file}: {[r.mismatch && `${r.mismatch} mismatch`, r.breaking && "1 breaking change"].filter(Boolean).join(", ")}</Link>
              ))}
            </li>
          </ul>
        }
        aside={sel ? (
          <>
            <AsideTop map={first1} back={BACK}><Link className="btn view-btn" href={FIX[sel.id]?.href ?? "/settings/indexing"}>{FIX[sel.id]?.label ?? "Open indexing settings"}</Link></AsideTop>
            <div className="cov-un-head"><h2>{n} · {sentence(sel.gap!.what)}</h2><Chip tone="bad">unresolved</Chip></div>
            <span className="view-sub">{sel.gap!.why}. Each one clears when you fix the cause and index again.</span>
            <ul className="cov-refs">
              <li className="cov-ref view-k"><span>Reference</span><span>Raw reference</span></li>
              {list.map((u) => <li key={u.at + u.raw} className="cov-ref">{u.node ? <NodeLink id={u.node} back={BACK} label={u.at} /> : <span className="view-mono view-cell">{u.at}</span>}<span className="view-mono view-cell">{u.raw}</span></li>)}
            </ul>
            {list.length < n && <p className="view-note">Showing {list.length} of {n}. The rest are in the index.</p>}
          </>
        ) : <p className="view-note">Nothing is unresolved. Every reference in the index points at something.</p>}>
        {gaps.length === 0 ? <p className="view-sub">Nothing is unresolved. Every reference in the index points at something.</p> : (
          <>
            <h2 className="view-k">Biggest gaps, by call sites or files affected</h2>
            <div className="view-scroll">
              <div className="view-card" role="grid" aria-label="Gaps" ref={rv.ref} onKeyDown={rv.onKeyDown}>
                {gaps.map((g) => (
                  <div key={g.id} className="cov-gap view-row" {...rv.row(g.id)}>
                    <Cell className="view-mono cov-n">{g.total - g.done}</Cell>
                    <Cell className="cov-what"><b>{sentence(g.gap!.what)}</b><span className="view-muted view-cell">{g.gap!.why}</span></Cell>
                    <Link className="btn view-btn" tabIndex={-1} href={FIX[g.id]?.href ?? "/settings/indexing"}>{FIX[g.id]?.label ?? "Open indexing settings"}</Link>
                  </div>
                ))}
              </div>
            </div>
            <KeyHint t="shows the first affected file on the map." />
          </>
        )}
      </ViewSplit>
    </Page>
  );
}
