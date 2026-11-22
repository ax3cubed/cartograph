"use client";

import { useMemo } from "react";
import Link from "next/link";
import { Plug } from "lucide-react";
import { getCommits, getContracts, getEntrypoints } from "@/lib/source";
import { LINKS, byId, commitBySha, serviceColor, SERVICES } from "@/lib/workspace";
import { chipText } from "@/lib/model";
import type { Entrypoint } from "@/lib/model";
import { AsideTop, Cell, Chip, KeyHint, NodeLink, Page, ViewSplit, fmtDate, mapHref, toneOf, useHashState, useRoving } from "./shared";
import "./entrypoints.css";

const KINDS: { kind: Entrypoint["kind"]; label: string; tag: string }[] = [
  { kind: "http", label: "HTTP", tag: "HTTP" }, { kind: "event", label: "Event consumers", tag: "EVENT" },
  { kind: "job", label: "Scheduled jobs", tag: "CRON" }, { kind: "cli", label: "CLI", tag: "CLI" },
];
const tagOf = (e: Entrypoint) => (e.kind === "http" ? e.label.split(" ")[0] : KINDS.find((k) => k.kind === e.kind)!.tag);
const routeOf = (e: Entrypoint) => (e.kind === "http" ? e.label.split(" ").slice(1).join(" ") : e.label);
const BACK = "/entrypoints";
const svcName = (id: string) => SERVICES.find((s) => s.id === id)?.name ?? id;

/** Files reachable from a node over links still alive, not counting the node itself. */
function reach(start: string): string[] {
  const seen = new Set([start]); const stack = [start];
  while (stack.length) { const id = stack.pop()!; for (const l of LINKS) if (l.from === id && !l.removed && !seen.has(l.to)) { seen.add(l.to); stack.push(l.to); } }
  seen.delete(start);
  return [...seen];
}
/** The newest commit that added or removed a link at this node. */
function lastChanged(node: string): string | undefined {
  const shas = LINKS.filter((l) => l.from === node || l.to === node).flatMap((l) => [l.added, l.removed]).filter(Boolean) as string[];
  return shas.map((s) => commitBySha.get(s)!.date).sort().at(-1);
}

export default function Entrypoints() {
  const all = useMemo(() => getEntrypoints(), []);
  const ops = useMemo(() => new Map(getContracts().flatMap((s) => s.operations.map((o) => [o.id, o] as const))), []);
  const h = useHashState();
  const kind = (KINDS.some((k) => k.kind === h.get("kind")) ? h.get("kind") : "all") as Entrypoint["kind"] | "all";
  const rows = kind === "all" ? all : all.filter((e) => e.kind === kind);
  const sel = rows.find((e) => e.id === h.get("row")) ?? rows[0];
  const kinds = KINDS.filter((k) => all.some((e) => e.kind === k.kind));
  const services = new Set(all.map((e) => e.service)).size;
  const rv = useRoving(rows.map((e) => e.id), sel?.id, (row) => h.set({ row }), (id) => mapHref(id, BACK, "trace"));

  return (
    <Page icon={Plug} when={all.length === 0} empty={{ title: "No adapters found yet", body: "Entrypoints come from the adapters that read your routes, events and jobs. Add a repository with a supported framework, then index again.", action: { label: "Open repositories", href: "/settings/repositories" } }}>
      <ViewSplit id="ep" title="Entrypoints: where requests and jobs start" asideLabel="Selected entrypoint"
        purpose={`${all.length} entrypoints across ${services} services. Pick one to see its consumers and everything it reaches.`}
        toolbar={
          <div role="group" aria-label="Entrypoint kind" className="view-chips">
            <button type="button" className="view-filter" aria-pressed={kind === "all"} onClick={() => h.set({ kind: undefined, row: undefined })}>All {all.length}</button>
            {kinds.map((k) => <button key={k.kind} type="button" className="view-filter" aria-pressed={kind === k.kind} onClick={() => h.set({ kind: k.kind, row: undefined })}>{k.label} {all.filter((e) => e.kind === k.kind).length}</button>)}
          </div>
        }
        aside={sel ? <Detail e={sel} op={ops.get(sel.id.replace(/\.consumer$/, ""))} /> : <p className="view-note">No entrypoints of this kind.</p>}>
        <div className="view-scroll">
          <div className="view-card" role="grid" aria-label="Entrypoints" ref={rv.ref} onKeyDown={rv.onKeyDown}>
            <div role="row" className="view-cols ep-cols view-head-row view-k"><span role="columnheader">Entrypoint</span><span role="columnheader">Service</span><span role="columnheader">Consumers</span><span role="columnheader">Files reached</span><span role="columnheader">Last changed</span></div>
            {rows.map((e) => {
              const when = lastChanged(e.node);
              return (
                <div key={e.id} className="view-cols ep-cols view-row" {...rv.row(e.id)}>
                  <Cell className="ep-label"><span className="view-mono view-method">{tagOf(e)}</span><NodeLink id={e.node} back={BACK} label={routeOf(e)} inRow /></Cell>
                  <Cell className="ep-svc"><i style={{ background: serviceColor(e.service) }} aria-hidden />{svcName(e.service)}</Cell>
                  <Cell className={e.consumers ? "" : "view-warn"}>{e.consumers || "0 · none"}</Cell>
                  <Cell>{reach(e.node).length}</Cell>
                  <Cell className="view-muted">{when ? fmtDate(when) : "No changes"}</Cell>
                </div>
              );
            })}
          </div>
        </div>
        <KeyHint t="traces the entrypoint on the map." />
      </ViewSplit>
    </Page>
  );
}

function Detail({ e, op }: { e: Entrypoint; op?: ReturnType<typeof getContracts>[number]["operations"][number] }) {
  const callers = (op?.callers ?? []).filter((c) => e.kind !== "event" || c.node === e.node);
  const reached = reach(e.node);
  const bySvc = new Map<string, number>();
  for (const id of reached) { const s = byId.get(id)!.service; bySvc.set(s, (bySvc.get(s) ?? 0) + 1); }
  return (
    <>
      <AsideTop map={e.node} back={BACK}><Link className="btn view-btn" href={mapHref(e.id, BACK, "trace")}>Trace on the map</Link></AsideTop>
      <div className="ep-title">
        <Chip tone="acc">{KINDS.find((k) => k.kind === e.kind)!.label}{op ? " · in contract" : ""}</Chip>
        <h2 className="view-mono">{e.label}</h2>
        <span className="view-sub">{svcName(e.service)} · {e.kind === "event" ? "consumer" : "handler"} <NodeLink id={e.node} back={BACK} /></span>
      </div>
      <div>
        <span className="view-k">Consumers · {callers.length} known</span>
        {callers.length === 0 && <p className="view-note ep-none">No consumers found. Callers outside the workspace aren&apos;t visible. Declare them as clients in the override file to count them.</p>}
        {callers.map((c) => (
          <div key={c.node} className="ep-con">
            <span className="ep-ic" aria-hidden>{svcName(byId.get(c.node)!.service).slice(0, 2).toUpperCase()}</span>
            <span className="ep-con-t"><b>{byId.get(c.node)!.name}</b><NodeLink id={c.node} back={BACK} /><span className="view-muted">{c.confidence.evidence[0]?.note}</span></span>
            <Chip tone={toneOf(c.confidence.score)}>{chipText(c.confidence)}</Chip>
          </div>
        ))}
      </div>
      <div className="view-slab">
        <span className="view-k">Reaches</span>
        <b>{reached.length} {reached.length === 1 ? "file" : "files"} across {bySvc.size} {bySvc.size === 1 ? "service" : "services"}</b>
        <span className="view-sub">{[...bySvc].map(([s, n]) => `${svcName(s)} ${n}`).join(" · ") || "Nothing downstream."}</span>
      </div>
    </>
  );
}
