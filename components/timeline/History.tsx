"use client";

import { ArrowRight, Circle, GitCommitHorizontal, History as HistoryIcon, RefreshCw } from "lucide-react";
import { byId, serviceColor } from "@/lib/workspace";
import { REWRITES, dayOf, fileEvents } from "@/lib/demo/history";
import EmptyState from "@/components/shell/EmptyState";
import "./timeline.css";

export interface HistoryProps {
  /** Node id, for example `billing/invoices/createInvoice.ts`. */
  nodeId: string;
  /** Called with the day (ms) when someone picks an event's date. The map can move the timeline there. */
  onJump?: (day: number) => void;
}

const date = (iso: string) => new Date(dayOf(iso)).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });
const since = (iso: string) => new Date(dayOf(iso)).toLocaleDateString("en-GB", { month: "short", year: "numeric", timeZone: "UTC" });

/** File history for a node: renames and commits, with a marker where history was rewritten. Newest first. */
export default function History({ nodeId, onJump }: HistoryProps) {
  const node = byId.get(nodeId);
  const events = fileEvents(nodeId);
  const rewrites = node ? REWRITES.filter((r) => r.repo === node.service) : [];
  const rows = [
    ...events.map((e) => ({ at: e.date, key: e.id, e })),
    ...rewrites.map((r) => ({ at: r.date, key: r.id, r })),
  ].sort((a, b) => (a.at < b.at ? 1 : -1));

  if (!node || rows.length === 0) {
    const body = node?.kind === "file"
      ? "This file has kept its path since it was created, and its repo's history hasn't been rewritten."
      : node?.kind === "service"
        ? "This service's repo history hasn't been rewritten. Select one of its files to see that file's renames."
        : "This module's repo history hasn't been rewritten. Select one of its files to see that file's renames.";
    return <EmptyState icon={HistoryIcon} title="No renames or rewrites" body={body} />;
  }
  const earlier = new Set(events.map((e) => e.from).filter(Boolean)).size;
  const created = events.find((e) => e.kind === "created");
  const when = (iso: string) => onJump
    ? <button type="button" className="tl-jump" onClick={() => onJump(dayOf(iso))} aria-label={`Show the map on ${date(iso)}`}>{date(iso)}</button>
    : date(iso);

  return (
    <section className="tl-hist" aria-label={`Renames and rewrites of ${node.name}`}>
      <div className="tl-hist-chips">
        {earlier > 0 && <span className="tl-chip is-strong">{earlier} earlier {earlier === 1 ? "path" : "paths"}</span>}
        {created && <span className="tl-chip">Since {since(created.date)}</span>}
      </div>
      <div className="tl-hist-head"><h4>Renames and rewrites</h4><span>Newest first</span></div>
      <ol className="tl-ev-list">
        {rows.map((row) => "r" in row && row.r ? (
          <li key={row.key} className="tl-ev is-rewrite">
            <RefreshCw size={18} aria-hidden />
            <span>
              <b>History rewritten</b> in <span style={{ color: `color-mix(in oklab, ${serviceColor(row.r.repo)}, black var(--hue-dim))` }}>{byId.get(row.r.repo)?.name}</span>.
              Sync replayed from merge base <span className="tl-sha">{row.r.mergeBase}</span>, and {row.r.linksRebuilt} links were rebuilt.
              <div className="tl-meta">{when(row.r.date)} · {row.r.kind}</div>
            </span>
          </li>
        ) : "e" in row && row.e ? (
          <li key={row.key} className={`tl-ev${row.e.kind === "change" ? " is-change" : ""}`}>
            {row.e.kind === "rename" ? <ArrowRight size={18} aria-hidden /> : row.e.kind === "created" ? <Circle size={18} aria-hidden /> : <GitCommitHorizontal size={18} aria-hidden />}
            <span>
              <b>{row.e.text}</b>
              {row.e.kind === "rename" && <> from <span className="tl-path">{row.e.from}</span></>}
              {row.e.kind === "created" && <> as <span className="tl-path">{row.e.from}</span></>}
              <div className="tl-meta">
                <span className="tl-sha">{row.e.sha}</span> · {row.e.author} · {when(row.e.date)}
                {row.e.similarity != null && <> · {Math.round(row.e.similarity * 100)}% same content</>}
                {row.e.note && <> · {row.e.note}</>}
              </div>
            </span>
          </li>
        ) : null)}
      </ol>
    </section>
  );
}
