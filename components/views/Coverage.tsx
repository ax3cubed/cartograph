"use client";

// Board A4 on the index: the six coverage rows (…/coverage, read from the summary the indexer wrote, never counted
// per request) and the references behind each gap (…/coverage/{row}/unresolved, 50 at a time, virtualised: a row can
// have tens of thousands). The raw reference and the reason are repo text: rendered as text.
import Link from "next/link";
import { Gauge } from "lucide-react";
import type { CoverageRow, Unresolved } from "@/lib/api";
import { usePaged } from "@/lib/api/paged";
import { useWsRead, wsRoute } from "@/lib/api/live";
import { AsideTop, Cell, Chip, NodeLink, Page, ViewSplit, useHashState, useView } from "./shared";
import { specRisk } from "./risk";
import { useContracts } from "./useContracts";
import VGrid from "./VGrid";
import "./coverage.css";
import { fmtNum } from "@/lib/date";

const BACK = "/coverage";
/** Each fix button is named for the place it opens. */
const FIX: Record<string, { label: string; href: string }> = {
  files: { label: "Open indexing settings", href: "/settings/indexing" },
  imports: { label: "Open indexing settings", href: "/settings/indexing" },
  calls: { label: "Open call matching", href: "/settings/contracts" },
  handlers: { label: "Open contracts", href: "/contracts" },
  routes: { label: "Open contracts", href: "/contracts" },
  consumers: { label: "Open entrypoints", href: "/entrypoints" },
};
const pctOf = (d: number, t: number) => (t ? (d / t) * 100 : 100);
const fmt = (p: number) => `${+p.toFixed(1)}%`;
const sentence = (s: string) => { const t = s.replace(/^\d+\s+/, ""); return t[0].toUpperCase() + t.slice(1); };
/** The rows the gap list holds: the unresolved references, which `gaps` counts by reason. */
const openOf = (r: CoverageRow) => r.gaps.reduce((n, g) => n + g.rows, 0);
const missed = (r: CoverageRow) => r.total > 0 && r.target != null && pctOf(r.done, r.total) < r.target;

const HEAD = { id: "cov", title: "Coverage: what the map can and can't see", purpose: "Unresolved references are kept, so each number traces back to a file and line." };

export default function Coverage() {
  const { ws, indexing } = useView();
  const res = useWsRead<CoverageRow[]>(ws, "coverage");
  const rows = res.data ?? [];
  const gaps = rows.filter((r) => openOf(r) > 0).sort((a, b) => openOf(b) - openOf(a));
  const h = useHashState();
  const sel = gaps.find((g) => g.id === h.get("row")) ?? gaps[0];
  const below = rows.filter(missed).length;
  const { specs, versions } = useContracts(ws);
  const risky = specs.map((s) => ({ s, r: specRisk(s, versions?.[s.id]) })).filter((x) => x.r.breaking || x.r.undocumented || x.r.specOnly);
  const riskN = risky.reduce((n, x) => n + x.r.breaking + x.r.undocumented + x.r.specOnly, 0);
  const list = usePaged<Unresolved>(ws, sel ? wsRoute(ws, `coverage/${encodeURIComponent(sel.id)}/unresolved`) : null);
  const first = list.rows.find((u) => u.file)?.file;
  const fix = sel ? FIX[sel.id] : undefined;

  return (
    <Page head={HEAD} icon={Gauge} indexing={indexing || (!!res.data && rows.length === 0)}>
      <ViewSplit id="cov" title="Coverage: what the map can and can't see" purpose="Unresolved references are kept, so each number traces back to a file and line." asideLabel="Unresolved references"
        actions={<span aria-live="polite"><Chip tone={below || riskN ? "warn" : "ok"}>{below ? `${below} below target` : "All targets met"}</Chip></span>}
        toolbar={
          <ul className="cov-mets">
            {rows.map((r) => {
              const p = pctOf(r.done, r.total);
              const miss = missed(r);
              const none = r.total === 0;
              return (
                <li key={r.id} className="cov-met">
                  <span className="cov-label">{r.label}</span>
                  <b>{none ? "—" : fmt(p)}</b>
                  <div className="cov-bar" role="progressbar" aria-label={r.label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={none ? undefined : Math.round(p)}><i style={{ width: `${none ? 0 : p}%` }} data-miss={miss || undefined} /></div>
                  <span className="view-muted cov-small">{fmtNum(r.done)} of {fmtNum(r.total)}{r.target != null ? ` · target ${r.target}%` : ""}</span>
                  <span className={`cov-small cov-status ${none || r.target == null ? "view-muted" : miss ? "view-warn" : "cov-ok"}`}>{none ? "Nothing to count yet" : r.target == null ? "No target set" : miss ? "Below target" : "Meets target"}</span>
                </li>
              );
            })}
            <li className="cov-met">
              <span className="cov-label">Contract risks</span>
              <b>{riskN}</b>
              <span className="cov-small">{riskN ? "Breaking changes, undocumented routes and spec-only operations" : "No drift between specs and code"}</span>
              {risky.map(({ s, r }) => (
                <Link key={s.id} className="cov-small view-link" href={`/contracts#${s.id}`}>{s.file}: {[r.breaking && `${r.breaking} breaking`, r.undocumented && `${r.undocumented} undocumented`, r.specOnly && `${r.specOnly} spec-only`].filter(Boolean).join(", ")}</Link>
              ))}
            </li>
          </ul>
        }
        aside={sel ? (
          <>
            <AsideTop map={first} back={BACK}><Link className="btn view-btn" href={fix?.href ?? "/settings/indexing"}>{fix?.label ?? "Open indexing settings"}</Link></AsideTop>
            <div className="cov-un-head"><h2>{fmtNum(openOf(sel))} · {sel.gap ? sentence(sel.gap.what) : sel.label}</h2><Chip tone="bad">unresolved</Chip></div>
            <span className="view-sub">{sel.gaps.map((g) => `${fmtNum(g.rows)} ${g.reason}`).slice(0, 3).join(" · ")}. Each one clears when you fix the cause and index again.</span>
            <VGrid rows={list.rows} rowId={(u) => `${u.path}:${u.line}:${u.text}`} label="Unresolved references" cols="cov-ref" rowHeight={52} maxHeight={420}
              sel={undefined} select={() => {}} hasMore={list.hasMore} more={list.more} loading={list.loading}
              empty={<p className="view-note">Nothing is unresolved here.</p>}
              head={<><span role="columnheader">Reference</span><span role="columnheader">Raw reference</span></>}
              render={(u) => (
                <>
                  <Cell>{u.file ? <NodeLink id={u.file} back={BACK} label={`${u.path}:${u.line}`} inRow /> : <span className="view-mono view-cell">{u.path}:{u.line}</span>}</Cell>
                  <Cell className="view-mono">{u.text}</Cell>
                </>
              )} />
          </>
        ) : <p className="view-note">Nothing is unresolved. Every reference in the index points at something.</p>}>
        {gaps.length === 0 ? <p className="view-sub">Nothing is unresolved. Every reference in the index points at something.</p> : (
          <>
            <h2 className="view-k">Biggest gaps, by call sites or files affected</h2>
            <VGrid rows={gaps} rowId={(g) => g.id} label="Gaps" cols="cov-gap" rowHeight={64} sel={sel?.id} select={(row) => h.set({ row })} hasMore={false} more={() => {}} loading={res.loading}
              head={<><span role="columnheader">Count</span><span role="columnheader">Gap</span><span role="columnheader">Fix</span></>}
              render={(g) => (
                <>
                  <Cell className="view-mono cov-n">{fmtNum(openOf(g))}</Cell>
                  <Cell className="cov-what"><b>{sentence(g.gap?.what ?? g.label)}</b><span className="view-muted view-cell">{g.gap?.why ?? g.gaps[0]?.reason}</span></Cell>
                  <span role="gridcell"><Link className="btn view-btn" tabIndex={-1} href={FIX[g.id]?.href ?? "/settings/indexing"}>{FIX[g.id]?.label ?? "Open indexing settings"}</Link></span>
                </>
              )} />
            <p className="view-note">Arrow keys move between rows. Enter opens the details.</p>
          </>
        )}
      </ViewSplit>
    </Page>
  );
}
