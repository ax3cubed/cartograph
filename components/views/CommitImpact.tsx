"use client";

// Board A5 on the index: what one commit changed in the graph (…/commits/{sha}/impact) and what depends on it.
// Previous and Next step the merged first-parent timeline (I9). Messages, authors and paths are repo text.
import { useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import { ChevronLeft, ChevronRight, Copy, GitCommit } from "lucide-react";
import type { CommitImpact as Impact } from "@/lib/api";
import { getText } from "@/lib/api/client";
import { useCommits, useWsRead, wsRoute } from "@/lib/api/live";
import { AsideTop, Cell, Chip, KeyHint, NodeLink, Page, ViewSplit, fmtDate, useHashState, useNames, useView } from "./shared";
import VGrid from "./VGrid";
import "./commitImpact.css";

const TYPE: Record<Impact["added_links"][number]["kind"], string> = { imports: "Import", calls: "Call", http: "HTTP", event: "Event", db: "Table read" };
const BACK = "/impact";

const HEAD = { id: "ci", title: "Impact: what a commit changed and who it affects", purpose: "The links a commit added and removed, and what depends on them." };

export default function CommitImpact() {
  const { ws, live, indexing } = useView();
  const h = useHashState();
  const commits = useCommits(ws);
  const list = commits.data ?? [];
  const latest = list.at(-1)?.sha ?? "";
  const asked = h.get("sha") ?? (h.bare && h.bare !== "empty" && h.bare !== "indexing" ? h.bare : "");
  const sha = asked || latest;
  const r = useWsRead<Impact>(ws, sha ? `commits/${encodeURIComponent(sha)}/impact` : null);
  const impact = r.data && (r.data.commit.sha.startsWith(sha) || sha.startsWith(r.data.commit.sha)) ? r.data : undefined;
  // names: the files now; a node gone by now shows its id
  const name = useNames(ws);
  const files = useMemo(() => {
    if (!impact) return [];
    const touched = [...new Set([...impact.added_links, ...impact.removed_links].map((l) => l.from))];
    return touched.map((f) => ({ id: f, a: impact.added_links.filter((l) => l.from === f).length, r: impact.removed_links.filter((l) => l.from === f).length }));
  }, [impact]);
  const [md, setMd] = useState("");
  const [msg, setMsg] = useState("");
  const [fallback, setFallback] = useState(false);
  const area = useRef<HTMLTextAreaElement>(null);
  useEffect(() => { setMsg(""); setFallback(false); setMd(""); }, [sha]);
  useEffect(() => { if (fallback) area.current?.select(); }, [fallback]);

  // The text is the server's, the one `cartograph impact` prints: the app doesn't write its own.
  const copy = async () => {
    if (!ws || !impact) return;
    let text = "";
    try { text = await getText(wsRoute(ws, `commits/${encodeURIComponent(impact.commit.sha)}/impact/markdown`)!); }
    catch { setMsg("The server couldn't write the Markdown. Try again."); return; }
    setMd(text);
    try { await navigator.clipboard.writeText(text); setFallback(false); setMsg("Copied as Markdown."); }
    catch { setFallback(true); setMsg("Clipboard access was refused. The Markdown is selected below. Press Ctrl+C or Cmd+C to copy it."); }
  };
  const go = (to?: string | null) => { if (to) h.set({ sha: to, row: undefined }); };
  const added = impact?.added_links ?? [], removed = impact?.removed_links ?? [];
  const links = [...added.map((l) => [l, true] as const), ...removed.map((l) => [l, false] as const)].map(([l, add]) => ({ id: `${l.id}>${add}`, l, add }));
  const sel = links.find((x) => x.id === h.get("row")) ?? links[0];
  const loading = live && (commits.loading || (!!sha && r.loading));
  const empty = list.length || loading
      ? { title: "That commit isn't in the index", body: `No commit ${sha.slice(0, 12)} was found in this workspace. Pick one from the list, or go to the latest commit.`, action: { label: "Go to latest commit", onClick: () => h.set({ sha: latest, row: undefined }) } }
      : { title: "No history to show yet", body: "Impact compares what each commit changed in the graph. It appears once the index has read your git history.", action: { label: "Open indexing settings", href: "/settings/indexing" } };

  if (loading && !impact) return <p className="view-note" aria-busy="true">Loading the commit…</p>;
  return (
    <Page head={HEAD} icon={GitCommit} indexing={indexing} when={!impact} empty={empty}>
      {impact && (
        <ViewSplit id="ci" title="Impact: what a commit changed and who it affects" asideLabel="Selected link"
          purpose={`${impact.commit.message}. ${impact.commit.repo} · ${impact.commit.author} · ${fmtDate(impact.commit.date)}.`}
          toolbar={<>
            <label className="view-select">Commit
              <select className="ci-pick" value={impact.commit.sha} onChange={(e) => h.set({ sha: e.target.value, row: undefined })}>
                {[...list].reverse().map((c) => <option key={c.sha} value={c.sha}>{fmtDate(c.date)} · {c.sha.slice(0, 7)} · {c.message.slice(0, 120)}</option>)}
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
                  <span className="view-k">{sel.add ? "Added" : "Removed"} · {TYPE[sel.l.kind]}</span>
                  <NodeLink id={sel.l.from} back={BACK} label={name(sel.l.from)} />
                  <span className="view-muted" aria-hidden>depends on</span>
                  <NodeLink id={sel.l.to} back={BACK} label={name(sel.l.to)} />
                </div>
              </>
            ) : <p className="view-note">This commit did not add or remove any links.</p>}
            <h2>Affected · {impact.affected.length}</h2>
            {impact.affected.length === 0 ? <p className="view-note">Nothing depends on the code this commit touched.</p> : (
              <div className="view-list">{impact.affected.slice(0, 200).map((a) => <div key={a} className="ci-lk"><NodeLink id={a} back={BACK} label={name(a)} /></div>)}</div>
            )}
            <p className="view-note">These depend on the files whose links changed, directly or through another file.</p>
            <h2>Files touched · {files.length}</h2>
            {files.length === 0 ? <p className="view-note">No files with changed links.</p> : (
              <div className="view-list">{files.map((f) => <div key={f.id} className="ci-lk"><NodeLink id={f.id} back={BACK} label={name(f.id)} /><span className="view-muted">{[f.a && `${f.a} ${f.a === 1 ? "link" : "links"} added`, f.r && `${f.r} ${f.r === 1 ? "link" : "links"} removed`].filter(Boolean).join(", ")}</span></div>)}</div>
            )}
          </>}>
          <div className="ci-summary" role="status">
            <b>Summary</b>
            <Chip tone="ok">{added.length} {added.length === 1 ? "link" : "links"} added</Chip>
            <Chip tone="bad">{removed.length} {removed.length === 1 ? "link" : "links"} removed</Chip>
            <Chip tone="warn">{impact.affected.length} affected</Chip>
            {msg && <span className="view-sub ci-msg" aria-live="polite">{msg}</span>}
          </div>
          {fallback && <textarea ref={area} className="ci-md view-mono" readOnly value={md} aria-label="Impact as Markdown" rows={10} onFocus={(e) => e.currentTarget.select()} />}
          {links.length === 0 ? <p className="view-note">This commit did not add or remove any links.</p> : (
            <>
              <VGrid rows={links} rowId={(x) => x.id} label="Links changed" cols="ci-cols" sel={sel?.id} select={(row) => h.set({ row })} hasMore={false} more={() => {}} loading={false}
                traceHref={(id) => { const x = links.find((y) => y.id === id); return x ? `/#focus=${encodeURIComponent(x.l.from)}&back=${BACK}` : undefined; }}
                head={<><span role="columnheader">Change</span><span role="columnheader">Link</span><span role="columnheader">Type</span></>}
                render={({ l, add }) => (
                  <>
                    <Cell className={add ? "ci-add" : "ci-rem"}>{add ? "Added" : "Removed"}</Cell>
                    <Cell><NodeLink id={l.from} back={BACK} label={name(l.from)} inRow /> → <NodeLink id={l.to} back={BACK} label={name(l.to)} inRow /></Cell>
                    <Cell><Chip>{TYPE[l.kind]}</Chip></Cell>
                  </>
                )} />
              <KeyHint t="shows the file on the map." />
            </>
          )}
        </ViewSplit>
      )}
    </Page>
  );
}
