"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { ChevronLeft, ChevronRight, Copy, GitCommit } from "lucide-react";
import { getCommitImpact, getCommits, getEntrypoints } from "@/lib/source";
import { SERVICES, byId } from "@/lib/workspace";
import type { CommitImpact as Impact, Link as Edge } from "@/lib/model";
import { AsideTop, Cell, Chip, KeyHint, NodeLink, Page, ViewSplit, fmtDate, mapHref, useHashState, useRoving } from "./shared";
import "./commitImpact.css";

const TYPE: Record<Edge["type"], string> = { imports: "Import", calls: "Call", http: "HTTP", event: "Event", db: "Table read" };
const BACK = "/impact";
const name = (id: string) => byId.get(id)?.name ?? id;
const cross = (l: Edge) => byId.get(l.from)?.service !== byId.get(l.to)?.service;
const svc = (id: string) => SERVICES.find((s) => s.id === id)?.name ?? id;

function summarize(i: Impact) {
  const touched = [...new Set([...i.addedLinks, ...i.removedLinks].map((l) => l.from))];
  const reach = new Set([...touched, ...i.affected]);
  const eps = getEntrypoints().filter((e) => reach.has(e.node));
  const files = touched.map((f) => {
    const a = i.addedLinks.filter((l) => l.from === f).length, r = i.removedLinks.filter((l) => l.from === f).length;
    return { id: f, a, r };
  });
  return { eps, files, cross: [...i.addedLinks, ...i.removedLinks].filter(cross).length };
}

/** The same Markdown `cartograph impact` prints (see docs/cli-and-mcp.html). */
function toMarkdown(i: Impact, s: ReturnType<typeof summarize>) {
  const link = (l: Edge, verb: string) => `- ${verb} \`${name(l.from)}\` -> \`${name(l.to)}\` (${TYPE[l.type]}${cross(l) ? ", cross-service" : ""})`;
  return [
    `## Commit impact: ${i.commit.sha} ${i.commit.message}`, "",
    `${svc(i.commit.repo)} · ${i.commit.author} · ${fmtDate(i.commit.date)}`, "",
    `${i.addedLinks.length} links added, ${i.removedLinks.length} removed, ${s.cross} cross-service, ${s.eps.length} entrypoints affected.`, "",
    "### Links", ...(i.addedLinks.length + i.removedLinks.length ? [...i.addedLinks.map((l) => link(l, "Added")), ...i.removedLinks.map((l) => link(l, "Removed"))] : ["- None"]), "",
    "### Entrypoints affected", ...(s.eps.length ? s.eps.map((e) => `- \`${e.label}\``) : ["- None"]), "",
    "### Files touched", ...(s.files.length ? s.files.map((f) => `- \`${byId.get(f.id)?.path}\` (${f.a} added, ${f.r} removed)`) : ["- None"]),
  ].join("\n");
}

export default function CommitImpact() {
  const h = useHashState();
  const commits = getCommits();
  const latest = commits.at(-1)?.sha ?? "";
  const sha = h.get("sha") ?? (h.bare && h.bare !== "empty" && h.bare !== "indexing" ? h.bare : latest);
  const impact = getCommitImpact(sha);
  const s = impact ? summarize(impact) : undefined;
  const md = impact && s ? toMarkdown(impact, s) : "";
  const [msg, setMsg] = useState("");
  const [fallback, setFallback] = useState(false);
  const area = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { setMsg(""); setFallback(false); }, [sha]);
  useEffect(() => { if (fallback) area.current?.select(); }, [fallback]);

  const copy = async () => {
    try { await navigator.clipboard.writeText(md); setFallback(false); setMsg("Copied as Markdown."); }
    catch { setFallback(true); setMsg("Clipboard access was refused. The Markdown is selected below. Press Ctrl+C or Cmd+C to copy it."); }
  };
  const go = (to?: string) => { if (to) h.set({ sha: to, row: undefined }); };
  const added = impact?.addedLinks ?? [], removed = impact?.removedLinks ?? [];
  const links = [...added.map((l) => [l, true] as const), ...removed.map((l) => [l, false] as const)].map(([l, add]) => ({ id: `${l.from}>${l.to}>${add}`, l, add }));
  const sel = links.find((x) => x.id === h.get("row")) ?? links[0];
  const rv = useRoving(links.map((x) => x.id), sel?.id, (row) => h.set({ row }), (id) => { const x = links.find((y) => y.id === id); return x ? mapHref(x.l.from, BACK) : undefined; });

  return (
    <Page icon={GitCommit} when={!impact} empty={commits.length ? { title: "That commit isn't in the index", body: `No commit ${sha} was found in this workspace. Pick one from the list, or go to the latest commit.`, action: { label: "Go to latest commit", onClick: () => h.set({ sha: latest, row: undefined }) } } : { title: "No history to show yet", body: "Impact compares what each commit changed in the graph. It appears once the index has read your git history.", action: { label: "Open indexing settings", href: "/settings/indexing" } }}>
      {impact && s && (
        <ViewSplit id="ci" title="Impact: what a commit changed and who it affects" asideLabel="Selected link"
          purpose={`${impact.commit.message}. ${svc(impact.commit.repo)} · ${impact.commit.author} · ${fmtDate(impact.commit.date)}.`}
          toolbar={<>
            <label className="view-select">Commit
              <select className="ci-pick" value={impact.commit.sha} onChange={(e) => h.set({ sha: e.target.value, row: undefined })}>
                {[...commits].reverse().map((c) => <option key={c.sha} value={c.sha}>{fmtDate(c.date)} · {c.sha} · {c.message}</option>)}
              </select>
            </label>
            <div className="ci-actions">
              <button type="button" className="btn view-btn" disabled={!impact.prev} onClick={() => go(impact.prev)}><ChevronLeft size={14} aria-hidden />Previous commit</button>
              <button type="button" className="btn view-btn" disabled={!impact.next} onClick={() => go(impact.next)}>Next commit<ChevronRight size={14} aria-hidden /></button>
              <button type="button" className="btn view-btn" onClick={copy}><Copy size={14} aria-hidden />Copy as Markdown</button>
              <Link className="btn view-btn" href={`/#commit=${impact.commit.sha}&back=${BACK}`}>Open map at this commit</Link>
            </div>
          </>}
          aside={<>
            {sel ? (
              <>
                <AsideTop map={sel.l.from} back={BACK} />
                <div className="view-slab" aria-live="polite">
                  <span className="view-k">{sel.add ? "Added" : "Removed"} · {TYPE[sel.l.type]}{cross(sel.l) ? " · cross-service" : ""}</span>
                  <NodeLink id={sel.l.from} back={BACK} />
                  <span className="view-muted" aria-hidden>depends on</span>
                  <NodeLink id={sel.l.to} back={BACK} />
                </div>
              </>
            ) : <p className="view-note">This commit did not add or remove any links.</p>}
            <h2>Entrypoints affected · {s.eps.length}</h2>
            {s.eps.length === 0 ? <p className="view-note">No entrypoint depends on the code this commit touched.</p> : (
              <div className="view-list">{s.eps.map((e) => <div key={e.id} className="ci-ep"><span className="view-mono view-method">{e.kind === "http" ? e.label.split(" ")[0] : e.kind.toUpperCase()}</span><NodeLink id={e.node} back={BACK} label={e.kind === "http" ? e.label.split(" ").slice(1).join(" ") : e.label} /></div>)}</div>
            )}
            <p className="view-note">These depend on the files whose links changed, directly or through another file.</p>
            <h2>Files touched · {s.files.length}</h2>
            {s.files.length === 0 ? <p className="view-note">No files with changed links.</p> : (
              <div className="view-list">{s.files.map((f) => <div key={f.id} className="ci-lk"><NodeLink id={f.id} back={BACK} /><span className="view-muted">{[f.a && `${f.a} ${f.a === 1 ? "link" : "links"} added`, f.r && `${f.r} ${f.r === 1 ? "link" : "links"} removed`].filter(Boolean).join(", ")}</span></div>)}</div>
            )}
          </>}>
          <div className="ci-summary" role="status">
            <b>Summary</b>
            <Chip tone="ok">{added.length} {added.length === 1 ? "link" : "links"} added</Chip>
            <Chip tone="bad">{removed.length} {removed.length === 1 ? "link" : "links"} removed</Chip>
            {s.cross > 0 && <Chip tone="acc">{s.cross} cross-service</Chip>}
            <Chip tone="warn">{s.eps.length} {s.eps.length === 1 ? "entrypoint" : "entrypoints"} affected</Chip>
            {msg && <span className="view-sub ci-msg" aria-live="polite">{msg}</span>}
          </div>
          {fallback && <textarea ref={area} className="ci-md view-mono" readOnly value={md} aria-label="Impact as Markdown" rows={10} onFocus={(e) => e.currentTarget.select()} />}
          {links.length === 0 ? <p className="view-note">This commit did not add or remove any links.</p> : (
            <>
              <div className="view-scroll">
                <div className="view-card" role="grid" aria-label="Links changed" ref={rv.ref} onKeyDown={rv.onKeyDown}>
                  <div role="row" className="view-cols ci-cols view-head-row view-k"><span role="columnheader">Change</span><span role="columnheader">Link</span><span role="columnheader">Type</span></div>
                  {links.map(({ id, l, add }) => (
                    <div key={id} className="view-cols ci-cols view-row" {...rv.row(id)}>
                      <Cell className={add ? "ci-add" : "ci-rem"}>{add ? "Added" : "Removed"}</Cell>
                      <Cell><NodeLink id={l.from} back={BACK} label={name(l.from)} inRow /> → <NodeLink id={l.to} back={BACK} label={name(l.to)} inRow /></Cell>
                      <Cell><Chip tone={cross(l) ? "acc" : ""}>{TYPE[l.type]}{cross(l) ? " · cross-service" : ""}</Chip></Cell>
                    </div>
                  ))}
                </div>
              </div>
              <KeyHint t="shows the file on the map." />
            </>
          )}
        </ViewSplit>
      )}
    </Page>
  );
}
