"use client";

import { useMemo } from "react";
import Link from "next/link";
import { FileCode2 } from "lucide-react";
import { getContracts } from "@/lib/source";
import { commitBySha } from "@/lib/workspace";
import { VERSIONS } from "@/lib/demo/views-extra";
import { AsideTop, Cell, Chip, KeyHint, NodeLink, Page, ViewSplit, fmtDate, mapHref, useHashState, useRoving } from "./shared";
import { STATUS, specRisk, specRows, type Row } from "./risk";
import "./contracts.css";

const BACK = "/contracts";
const riskText = (r: ReturnType<typeof specRisk>) => [r.mismatch && `${r.mismatch} mismatch`, r.breaking && "1 breaking change", r.undocumented && `${r.undocumented} undocumented`, r.specOnly && `${r.specOnly} spec-only`].filter(Boolean).join(", ") || "in sync";

export default function Contracts() {
  const specs = useMemo(() => getContracts(), []);
  const h = useHashState();
  // Open on the spec with the worst status unless the URL names one.
  const worst = useMemo(() => [...specs].sort((a, b) => specRisk(a).rank - specRisk(b).rank)[0], [specs]);
  const want = h.get("spec") ?? h.bare;
  const all = want === "all";
  const spec = all ? undefined : specs.find((s) => s.id === want) ?? worst;
  const rows: Row[] = useMemo(() => (all ? specs.flatMap(specRows).sort((a, b) => STATUS[a.status].rank - STATUS[b.status].rank) : spec ? specRows(spec) : []), [all, spec, specs]);
  const sel = rows.find((r) => r.key === h.get("row")) ?? rows[0];
  const count = (s: Row["status"]) => rows.filter((r) => r.status === s).length;
  const selSpec = specs.find((s) => s.id === sel?.spec);
  const versions = selSpec ? VERSIONS[selSpec.id] ?? [] : [];
  const rv = useRoving(rows.map((r) => r.key), sel?.key, (row) => h.set({ row }), (k) => { const r = rows.find((x) => x.key === k); return r?.handler ? mapHref(r.handler, BACK) : undefined; });

  return (
    <Page icon={FileCode2} when={specs.length === 0} empty={{ title: "No specs found yet", body: "This view lines up each spec operation with the handler that serves it. Add an OpenAPI or event schema file to a repository, then index again.", action: { label: "Open contract settings", href: "/settings/contracts" } }}>
      <ViewSplit id="ct" title="Contracts: where the spec and the code disagree" purpose="Each operation in the spec beside the handler that serves it and the callers that use it. The riskiest rows come first." asideLabel="Selected operation"
        toolbar={<>
          <label className="view-select">Spec
            <select value={all ? "all" : spec?.id} onChange={(e) => h.set({ spec: e.target.value, row: undefined })}>
              <option value="all">All specs</option>
              {[...specs].sort((a, b) => specRisk(a).rank - specRisk(b).rank).map((s) => <option key={s.id} value={s.id}>{s.file} · {riskText(specRisk(s))}</option>)}
            </select>
          </label>
          <div className="view-chips" aria-live="polite">
            <Chip tone="ok">{count("sync")} in sync</Chip>
            {count("spec-only") > 0 && <Chip tone="warn">{count("spec-only")} spec-only</Chip>}
            {count("undocumented") > 0 && <Chip tone="warn">{count("undocumented")} undocumented {count("undocumented") === 1 ? "route" : "routes"}</Chip>}
            {count("mismatch") > 0 && <Chip tone="bad">{count("mismatch")} schema mismatch</Chip>}
          </div>
        </>}
        aside={<>
          {sel ? (
            <>
              <AsideTop map={sel.handler} back={BACK} />
              <section className="view-slab" aria-labelledby="ct-sel" aria-live="polite">
                <span className="view-k">Selected operation</span>
                <h2 id="ct-sel" className="view-mono ct-sel">{sel.method} {sel.path}</h2>
                <Chip tone={STATUS[sel.status].tone}>{STATUS[sel.status].label}</Chip>
                {sel.handler ? <span className="ct-p">Handler <NodeLink id={sel.handler} back={BACK} /></span> : <span className="ct-p view-warn">No handler serves this operation.</span>}
                {sel.status === "sync" && <p className="ct-p">The spec and the handler agree.</p>}
                {sel.status === "spec-only" && <p className="ct-p">The spec lists this operation, but no handler serves it yet.</p>}
                {sel.status === "undocumented" && <p className="ct-p">A handler serves this route, but no spec lists it.</p>}
                {sel.diff && <><p className="ct-p">The spec and the handler disagree about the response of <span className="view-mono">{sel.name}</span>.</p><p className="ct-p ct-diff">{sel.diff.spec}<br />{sel.diff.handler}</p></>}
                {sel.callers.length > 0 && <span className="view-k">Callers · {sel.callers.length}</span>}
                {sel.callers.map((c) => <div key={c.node} className="ct-caller"><NodeLink id={c.node} back={BACK} /><Chip tone={c.tone}>{c.chip}</Chip></div>)}
              </section>
            </>
          ) : <p className="view-note">Select an operation to see its handler and callers.</p>}
          {selSpec && <h2 className="ct-vh">Versions of {selSpec.file}</h2>}
          <ol className="ct-vers">
            {versions.map((v) => (
              <li key={v.v}>
                <b>v{v.v} · {fmtDate(commitBySha.get(v.sha)!.date)} <Link className="view-mono view-link ct-sha" href={`/impact#${v.sha}`}>{v.sha}</Link></b>
                {v.breaking ? (
                  <>
                    <Chip tone="bad">1 breaking change</Chip>
                    <span className="ct-p">{v.breaking.what} {v.breaking.callers.length} {v.breaking.callers.length === 1 ? "caller still reads" : "callers still read"} it:</span>
                    <span className="ct-callers">{v.breaking.callers.map((c) => <NodeLink key={c} id={c} back={BACK} />)}</span>
                  </>
                ) : <span className="ct-p">{v.summary}</span>}
              </li>
            ))}
          </ol>
        </>}>
        <div className="view-scroll">
          <div className="view-card" role="grid" aria-label="Operations" ref={rv.ref} onKeyDown={rv.onKeyDown}>
            <div role="row" className="view-cols ct-cols view-head-row view-k"><span role="columnheader">Method</span><span role="columnheader">Path · operation</span><span role="columnheader">Handler</span><span role="columnheader">Callers</span><span role="columnheader">Status</span></div>
            {rows.map((r) => (
              <div key={r.key} className="view-cols ct-cols view-row" {...rv.row(r.key)}>
                <Cell className="view-mono view-method">{r.method}</Cell>
                <Cell className="ct-path"><span className="view-mono view-cell">{r.path}</span><span className="view-muted view-cell">{all ? `${r.name} · ${r.spec}` : r.name}</span></Cell>
                <Cell>{r.handler ? <NodeLink id={r.handler} back={BACK} inRow /> : <span className="view-warn">missing</span>}</Cell>
                <Cell className="view-mono">{r.callers.length}</Cell>
                <Cell><Chip tone={STATUS[r.status].tone}>{STATUS[r.status].label}</Chip></Cell>
              </div>
            ))}
          </div>
        </div>
        <KeyHint t="shows the handler on the map." />
      </ViewSplit>
    </Page>
  );
}
