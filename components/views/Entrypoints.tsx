"use client";

// Board A2 on the index: what starts work in a service (…/entrypoints), who calls it and what it reaches. The list is
// paged, filtered and sorted by the server, 50 rows at a time, and drawn virtualised, so a few thousand entrypoints
// cost no more than a few. Route strings, labels and client names are repo text: rendered as text, never as markup.
import { useEffect, useState } from "react";
import Link from "next/link";
import { Plug } from "lucide-react";
import type { Entrypoint, EntrypointCounts, EntrypointDetail, EntrypointKind } from "@/lib/api";
import { usePaged } from "@/lib/api/paged";
import { wsRoute, useWsRead } from "@/lib/api/live";
import { AsideTop, Cell, Chip, KeyHint, NodeLink, Page, ViewSplit, levelLabel, levelTone, mapHref, useHashState, useNames, useView } from "./shared";
import VGrid from "./VGrid";
import "./entrypoints.css";

const KINDS: { kind: EntrypointKind; label: string; tag: string }[] = [
  { kind: "http", label: "HTTP", tag: "HTTP" }, { kind: "event", label: "Event consumers", tag: "EVENT" }, { kind: "job", label: "Scheduled jobs", tag: "CRON" },
  { kind: "cli", label: "CLI", tag: "CLI" }, { kind: "export", label: "Package entry points", tag: "EXPORT" }, { kind: "ui", label: "Pages", tag: "PAGE" },
];
const kindOf = (k: EntrypointKind) => KINDS.find((x) => x.kind === k)!;
const tagOf = (e: Entrypoint) => (e.kind === "http" ? e.label.split(" ")[0] : kindOf(e.kind).tag);
const routeOf = (e: Entrypoint) => (e.kind === "http" ? e.label.split(" ").slice(1).join(" ") : e.label);
const BACK = "/entrypoints";
const SORTS = ["name", "consumers", "-consumers", "reach", "-reach"] as const;
const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

const HEAD = { id: "ep", title: "Entrypoints: where requests and jobs start", purpose: "Pick one to see its consumers and everything it reaches." };

export default function Entrypoints() {
  const { ws, indexing } = useView();
  const h = useHashState();
  const kind = KINDS.find((k) => k.kind === h.get("kind"))?.kind;
  const sort = SORTS.find((s) => s === h.get("sort")) ?? "name";
  const [text, setText] = useState(h.get("q") ?? "");
  const [q, setQ] = useState(text);
  useEffect(() => { const t = setTimeout(() => setQ(text.trim()), 250); return () => clearTimeout(t); }, [text]);

  const counts = useWsRead<EntrypointCounts>(ws, "entrypoints/counts");
  const list = usePaged<Entrypoint>(ws, wsRoute(ws, "entrypoints"), { kind, q, sort: sort === "name" ? undefined : sort });
  const names = useNames(ws);
  const sel = list.rows.find((e) => e.id === h.get("row")) ?? list.rows[0];
  const total = counts.data?.total ?? 0;
  const filtered = !!kind || !!q;
  const flip = (field: "consumers" | "reach") => h.set({ sort: sort === field ? `-${field}` : field, row: undefined });
  const sortOf = (field: string) => (sort === field ? "ascending" : sort === `-${field}` ? "descending" : "none");
  const mark = (field: string) => (sort === field ? " ▲" : sort === `-${field}` ? " ▼" : "");

  return (
    <Page head={HEAD} icon={Plug} indexing={indexing} when={!!counts.data && total === 0}
      empty={{ title: "No adapters found yet", body: "Entrypoints come from the adapters that read your routes, events and jobs. None of the indexed code matched one. Add a repository with a supported framework, then index again.", action: { label: "Open repositories", href: "/settings/repositories" } }}>
      <ViewSplit id="ep" title="Entrypoints: where requests and jobs start" asideLabel="Selected entrypoint"
        purpose={counts.data ? `${plural(total, "entrypoint")} across ${plural(counts.data.services, "service")}. Pick one to see its consumers and everything it reaches.` : "Pick one to see its consumers and everything it reaches."}
        toolbar={<>
          <div role="group" aria-label="Entrypoint kind" className="view-chips">
            <button type="button" className="view-filter" aria-pressed={!kind} onClick={() => h.set({ kind: undefined, row: undefined })}>All {total}</button>
            {(counts.data?.kinds ?? []).map((k) => <button key={k.kind} type="button" className="view-filter" aria-pressed={kind === k.kind} onClick={() => h.set({ kind: k.kind, row: undefined })}>{kindOf(k.kind).label} {k.count}</button>)}
          </div>
          <label className="view-select">Filter
            <input type="search" className="view-search" value={text} placeholder="Route or name" onChange={(e) => { setText(e.target.value); h.set({ q: e.target.value.trim() || undefined, row: undefined }); }} />
          </label>
        </>}
        aside={sel ? <Detail ws={ws} e={sel} names={names} /> : <p className="view-note">{list.loading ? "Loading…" : "No entrypoints match."}</p>}>
        <VGrid rows={list.rows} rowId={(e) => e.id} label="Entrypoints" cols="ep-cols" sel={sel?.id} select={(row) => h.set({ row })}
          hasMore={list.hasMore} more={list.more} loading={list.loading} traceHref={(id) => { const e = list.rows.find((x) => x.id === id); return e && mapHref(e.node, BACK, "trace"); }}
          empty={<p className="view-sub">{filtered ? "No entrypoint matches this filter. Clear it to see all of them." : "No entrypoints."}</p>}
          head={<>
            <span role="columnheader">Entrypoint</span><span role="columnheader">Service</span>
            <span role="columnheader" aria-sort={sortOf("consumers")}><button type="button" className="view-sort" onClick={() => flip("consumers")}>Consumers{mark("consumers")}</button></span>
            <span role="columnheader" aria-sort={sortOf("reach")}><button type="button" className="view-sort" onClick={() => flip("reach")}>Files reached{mark("reach")}</button></span>
          </>}
          render={(e) => (
            <>
              <Cell className="ep-label"><span className="view-mono view-method">{tagOf(e)}</span><NodeLink id={e.node} back={BACK} label={routeOf(e)} inRow /></Cell>
              <Cell className="ep-svc">{e.service ? names(e.service) : ""}</Cell>
              <Cell className={e.consumers ? "" : "view-warn"}>{e.consumers || "0 · none"}</Cell>
              <Cell>{e.reach}</Cell>
            </>
          )} />
        <KeyHint t="traces the entrypoint on the map." />
      </ViewSplit>
    </Page>
  );
}

function Detail({ ws, e, names }: { ws: string | undefined; e: Entrypoint; names: (id: string) => string }) {
  const d = useWsRead<EntrypointDetail>(ws, `entrypoints/${encodeURIComponent(e.id)}`).data;
  const detail = d && d.entrypoint.id === e.id ? d : undefined;
  const consumers = detail?.consumers ?? [];
  const reach = detail?.reach;
  return (
    <>
      <AsideTop map={e.node} back={BACK}><Link className="btn view-btn" href={mapHref(e.node, BACK, "trace")}>Trace on the map</Link></AsideTop>
      <div className="ep-title">
        <Chip tone="acc">{kindOf(e.kind).label}</Chip>
        <h2 className="view-mono">{e.label}</h2>
        <span className="view-sub">{e.service ? `${names(e.service)} · ` : ""}{e.kind === "event" ? "consumer" : "starts in"} <NodeLink id={e.node} back={BACK} label={names(e.node)} /></span>
      </div>
      <div>
        <span className="view-k">Consumers · {detail ? `${consumers.length} known` : e.consumers}</span>
        {detail && consumers.length === 0 && <p className="view-note ep-none">No consumers found. Callers outside the workspace aren&apos;t visible. Declare them as clients in cartograph.yaml to count them.</p>}
        {consumers.map((c, i) => (
          <div key={c.link ?? c.name ?? i} className="ep-con">
            <span className="ep-ic" aria-hidden>{(c.node ? names(c.node) : c.name ?? "").slice(0, 2).toUpperCase()}</span>
            <span className="ep-con-t">
              {c.node ? <NodeLink id={c.node} back={BACK} label={names(c.node)} /> : <b>{c.name}</b>}
              {!c.node && <span className="view-muted">Declared client</span>}
            </span>
            <span className="view-chips">{c.overridden && <Chip tone="acc">Overridden</Chip>}{c.node && <Chip tone={levelTone(c.level)}>{levelLabel(c.level)}</Chip>}</span>
          </div>
        ))}
      </div>
      <div className="view-slab">
        <span className="view-k">Reaches</span>
        {reach ? (
          <>
            <b>{plural(reach.nodes, "file")} across {plural(reach.services.length, "service")}{reach.truncated ? " (the walk hit a cap)" : ""}</b>
            <span className="view-sub">{reach.services.map((s) => `${names(s.service)} ${s.nodes}`).join(" · ") || "Nothing downstream."}</span>
          </>
        ) : <span className="view-sub">{e.reach} files</span>}
      </div>
    </>
  );
}
