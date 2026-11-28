"use client";

// Board A3 on the index: each operation of a spec beside the handler that serves it and the callers that use it
// (…/contracts), and each spec's versions with what they broke (…/contracts/{id}/versions). Operation summaries, paths
// and change text come from the repo: rendered as text, never as markup.
import { useMemo } from "react";
import Link from "next/link";
import { FileCode2 } from "lucide-react";
import type { Contract } from "@/lib/api";
import { AsideTop, Cell, Chip, KeyHint, NodeLink, Page, ViewSplit, fmtDate, levelLabel, levelTone, mapHref, useHashState, useNames, useView } from "./shared";
import { STATUS, riskText, specRisk, specRows, type Row } from "./risk";
import { useContracts } from "./useContracts";
import VGrid from "./VGrid";
import "./contracts.css";

const BACK = "/contracts";

const HEAD = { id: "ct", title: "Contracts: where the spec and the code disagree", purpose: "Each operation in the spec beside the handler that serves it and the callers that use it." };

export default function Contracts() {
  const { ws, indexing } = useView();
  const { specs, versions: vers, loading, error } = useContracts(ws);
  const names = useNames(ws);
  const h = useHashState();
  const risk = (s: Contract) => specRisk(s, vers?.[s.id]);
  // Open on the spec with the worst status unless the URL names one.
  const worst = useMemo(() => [...specs].sort((a, b) => specRisk(a, vers?.[a.id]).rank - specRisk(b, vers?.[b.id]).rank)[0], [specs, vers]);
  const want = h.get("spec") ?? h.bare;
  const all = want === "all";
  const spec = all ? undefined : specs.find((s) => s.id === want) ?? worst;
  const rows: Row[] = useMemo(() => (all ? specs.flatMap(specRows).sort((a, b) => STATUS[a.status].rank - STATUS[b.status].rank) : spec ? specRows(spec) : []), [all, spec, specs]);
  const sel = rows.find((r) => r.key === h.get("row")) ?? rows[0];
  const count = (s: Row["status"]) => rows.filter((r) => r.status === s).length;
  const selSpec = specs.find((s) => s.id === sel?.spec);
  const history = [...(vers?.[selSpec?.id ?? ""] ?? [])].reverse();

  return (
    <Page head={HEAD} icon={FileCode2} indexing={indexing} when={!loading && !error && specs.length === 0}
      empty={{ title: "No specs found yet", body: "This view lines up each spec operation with the handler that serves it. Add an OpenAPI or event schema file to a repository, then index again.", action: { label: "Open contract settings", href: "/settings/contracts" } }}>
      <ViewSplit id="ct" title="Contracts: where the spec and the code disagree" purpose="Each operation in the spec beside the handler that serves it and the callers that use it. The riskiest rows come first." asideLabel="Selected operation"
        toolbar={<>
          <label className="view-select">Spec
            <select value={all ? "all" : spec?.id ?? ""} onChange={(e) => h.set({ spec: e.target.value, row: undefined })}>
              <option value="all">All specs</option>
              {[...specs].sort((a, b) => risk(a).rank - risk(b).rank).map((s) => <option key={s.id} value={s.id}>{s.file} · {riskText(risk(s))}</option>)}
            </select>
          </label>
          <div className="view-chips" aria-live="polite">
            <Chip tone="ok">{count("sync")} in sync</Chip>
            {count("spec-only") > 0 && <Chip tone="warn">{count("spec-only")} spec-only</Chip>}
            {count("undocumented") > 0 && <Chip tone="warn">{count("undocumented")} undocumented {count("undocumented") === 1 ? "route" : "routes"}</Chip>}
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
                {sel.summary && <p className="ct-p">{sel.summary}</p>}
                {sel.handler ? <span className="ct-p">Handler <NodeLink id={sel.handler} back={BACK} label={names(sel.handler)} /></span> : <span className="ct-p view-warn">No handler serves this operation.</span>}
                {sel.status === "sync" && <p className="ct-p">A parsed route serves this operation.</p>}
                {sel.status === "spec-only" && <p className="ct-p">The spec lists this operation, but no handler serves it yet.</p>}
                {sel.status === "undocumented" && <p className="ct-p">A handler serves this route, but no spec lists it.</p>}
                {sel.callers.length > 0 && <span className="view-k">Callers · {sel.callers.length}</span>}
                {sel.callers.map((c) => <div key={c.node} className="ct-caller"><NodeLink id={c.node} back={BACK} label={names(c.node)} /><Chip tone={levelTone(c.confidence.level)}>{levelLabel(c.confidence.level)} · {c.confidence.score.toFixed(2)}</Chip></div>)}
              </section>
            </>
          ) : <p className="view-note">Select an operation to see its handler and callers.</p>}
          {selSpec && <h2 className="ct-vh">Versions of {selSpec.file}</h2>}
          <ol className="ct-vers">
            {history.map((v) => (
              <li key={v.n}>
                <b>v{v.n} · {fmtDate(v.commit.date)} <Link className="view-mono view-link ct-sha" href={`/impact#${v.commit.sha}`}>{v.commit.sha.slice(0, 7)}</Link></b>
                {v.breaking_count > 0 ? (
                  <>
                    <Chip tone="bad">{v.breaking_count} breaking {v.breaking_count === 1 ? "change" : "changes"}</Chip>
                    {v.breaking.slice(0, 5).map((b) => (
                      <span key={`${b.kind}${b.subject}`} className="ct-p">{b.summary}{b.callers.length > 0 && ` ${b.callers.length} ${b.callers.length === 1 ? "caller still reads" : "callers still read"} it:`}
                        <span className="ct-callers">{b.callers.map((c) => <NodeLink key={c.node} id={c.node} back={BACK} label={names(c.node)} />)}</span></span>
                    ))}
                  </>
                ) : <span className="ct-p">{v.summary || (v.n === 1 ? "First version found in history." : "No breaking changes.")}</span>}
              </li>
            ))}
          </ol>
        </>}>
        <VGrid rows={rows} rowId={(r) => r.key} label="Operations" cols="ct-cols" sel={sel?.key} select={(row) => h.set({ row })} hasMore={false} more={() => {}} loading={loading}
          traceHref={(k) => { const r = rows.find((x) => x.key === k); return r?.handler ? mapHref(r.handler, BACK) : undefined; }}
          head={<><span role="columnheader">Method</span><span role="columnheader">Path · operation</span><span role="columnheader">Handler</span><span role="columnheader">Callers</span><span role="columnheader">Status</span></>}
          render={(r) => (
            <>
              <Cell className="view-mono view-method">{r.method}</Cell>
              <Cell className="ct-path"><span className="view-mono view-cell">{r.path}</span><span className="view-muted view-cell">{all ? `${r.name} · ${r.spec}` : r.name}</span></Cell>
              <Cell>{r.handler ? <NodeLink id={r.handler} back={BACK} label={names(r.handler)} inRow /> : <span className="view-warn">missing</span>}</Cell>
              <Cell className="view-mono">{r.callers.length}</Cell>
              <Cell><Chip tone={STATUS[r.status].tone}>{STATUS[r.status].label}</Chip></Cell>
            </>
          )} />
        <KeyHint t="shows the handler on the map." />
      </ViewSplit>
    </Page>
  );
}
