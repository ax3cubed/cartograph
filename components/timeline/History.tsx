"use client";

// Board G10 on the index: a node's paths over time (…/nodes/{id}/history) and its repo's rewritten histories
// (…/timeline/markers), newest first. Paths and labels come from the repo: rendered as text only.
import { ArrowRight, Circle, History as HistoryIcon, RefreshCw } from "lucide-react";
import type { Lineage, TimelineMarker } from "@/lib/api";
import { useWsRead } from "@/lib/api/live";
import EmptyState from "@/components/shell/EmptyState";
import { fmtDate } from "@/lib/date";
import "./timeline.css";

export interface HistoryProps {
  ws: string;
  /** Node id, for example `n19`. */
  nodeId: string;
  kind: string;
  /** Commit dates by sha, to show when each step happened. */
  dateOf: (sha: string) => string | undefined;
  /** Called with a commit's sha when someone picks its date: the map moves the timeline there. */
  onJump?: (sha: string) => void;
}

/** Past this many rows the list draws a window of them (FE-10 scale). */
const WINDOW = 200;

export default function History({ ws, nodeId, kind, dateOf, onJump }: HistoryProps) {
  const lineage = useWsRead<Lineage[]>(ws, kind === "file" ? `nodes/${encodeURIComponent(nodeId)}/history` : null);
  const markers = useWsRead<TimelineMarker[]>(ws, "timeline/markers");
  const rewrites = (markers.data ?? []).filter((m) => m.kind === "rewrite");
  const steps = lineage.data ?? [];
  type Row = { seq: number; key: string; l?: Lineage; prev?: Lineage; m?: TimelineMarker };
  const rows: Row[] = [
    ...steps.map((l, i) => ({ seq: l.from.seq, key: `l${i}`, l, prev: steps[i - 1] })),
    ...rewrites.map((m, i) => ({ seq: m.at.seq, key: `m${i}`, m })),
  ].sort((a, b) => b.seq - a.seq);

  if (lineage.loading && !lineage.data) return <p className="lm-none" aria-busy="true">Loading the history…</p>;
  const moves = steps.filter((l) => l.via !== "add").length;
  if (moves === 0 && rewrites.length === 0) {
    const body = kind === "file"
      ? "This file has kept its path since it was created, and its repo's history hasn't been rewritten."
      : "This repo's history hasn't been rewritten. Select one of its files to see that file's renames.";
    return <EmptyState icon={HistoryIcon} title="No renames or rewrites" body={body} />;
  }
  const when = (sha: string) => {
    const d = dateOf(sha);
    const text = d ? fmtDate(d, "dmy") : sha.slice(0, 7);
    return onJump ? <button type="button" className="tl-jump" onClick={() => onJump(sha)} aria-label={`Show the map at ${text}`}>{text}</button> : text;
  };
  const created = steps[0];

  return (
    <section className="tl-hist" aria-label="Renames and rewrites">
      <div className="tl-hist-chips">
        {moves > 0 && <span className="tl-chip is-strong">{moves} earlier {moves === 1 ? "path" : "paths"}</span>}
        {created && dateOf(created.from.sha) && <span className="tl-chip">Since {fmtDate(dateOf(created.from.sha)!, "my")}</span>}
      </div>
      <div className="tl-hist-head"><h4>Renames and rewrites</h4><span>Newest first</span></div>
      <ol className="tl-ev-list">
        {rows.slice(0, WINDOW).map((row) => row.m ? (
          <li key={row.key} className="tl-ev is-rewrite">
            <RefreshCw size={18} aria-hidden />
            <span>
              <b>History rewritten.</b> {row.m.label}
              <div className="tl-meta"><span className="tl-sha">{row.m.at.sha.slice(0, 7)}</span> · {when(row.m.at.sha)}</div>
            </span>
          </li>
        ) : row.l ? (
          <li key={row.key} className="tl-ev">
            {row.l.via === "add" ? <Circle size={18} aria-hidden /> : <ArrowRight size={18} aria-hidden />}
            <span>
              {row.l.via === "add"
                ? <><b>Created</b> as <span className="tl-path">{row.l.path}</span></>
                : <><b>Moved</b> to <span className="tl-path">{row.l.path}</span>{row.prev && <> from <span className="tl-path">{row.prev.path}</span></>}</>}
              <div className="tl-meta">
                <span className="tl-sha">{row.l.from.sha.slice(0, 7)}</span> · {when(row.l.from.sha)}
                {row.l.via === "rename_exact" && <> · same content</>}
                {row.l.similarity != null && <> · {Math.round(row.l.similarity)}% same content</>}
              </div>
            </span>
          </li>
        ) : null)}
      </ol>
      {rows.length > WINDOW && <p className="lm-none">{rows.length - WINDOW} older rows not shown.</p>}
    </section>
  );
}
