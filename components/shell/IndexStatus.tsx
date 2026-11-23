"use client";

import { Check, Copy, RefreshCw } from "lucide-react";
import { getFreshness, getRepos } from "@/lib/source";
import type { Freshness } from "@/lib/model";
import { useLive, useRepos, useWorkspaces } from "@/lib/api/live";
import { RequestError, send } from "@/lib/api/client";
import { usePopover } from "./usePopover";
import { ago } from "./ago";
import { toast } from "./toast";
import "./indexStatus.css";

export const copy = (text: string) =>
  navigator.clipboard?.writeText(text).then(() => toast("Copied to the clipboard"), () => toast("Couldn't copy. Select the command and copy it by hand."));

/** The commits-behind count as `cartograph status` prints it: "1000+" when a repo's count stopped at its cap. */
export const behindText = (f: Freshness) => `${f.behind}${f.behindCapped ? "+" : ""}`;

/** Per state: chip class, chip text, popover title, popover body. */
function describe(f: Freshness, offline: boolean) {
  const built = f.builtAt ? `Built ${ago(f.builtAt)} · index v${f.indexVersion}` : "";
  if (offline) return { cls: "sh-chip-down", text: "Cartograph isn't running", title: "Cartograph isn't running", body: "The app can't reach the local server. Start it again; this tab reconnects by itself while the server keeps its session.", cmd: "cartograph open", built: "" };
  switch (f.state) {
    // A registered workspace is repaired by `cartograph sync`; `cartograph index` builds a separate single-repo index.
    case "failed": return { cls: "sh-chip-down", text: f.error?.what ?? "Index failed", title: "The index can't be read", body: f.error ? `${f.error.what}. ${f.error.action}` : "Sync again; it resumes from the last batch.", cmd: "cartograph sync", built };
    case "behind": return { cls: "sh-chip-warn", text: `${behindText(f)} commits behind HEAD`, title: `${behindText(f)} commits behind HEAD`, body: "Links added or removed since the last build aren't on the map. Search and Ask still use the older index until you update it.", cmd: "cartograph sync", built };
    case "syncing": return { cls: "sh-chip-warn", text: `Syncing ${Math.round((f.progress ?? 0) * 100)}%`, title: "Catching up", body: "A sync is running. The map stays usable and updates when it finishes.", built };
    case "rewritten": return { cls: "sh-chip-warn", text: "History was rewritten", title: "History was rewritten", body: "A rebase or force-push changed commits the index had read. Sync to replay from where the histories meet, so the timeline matches git.", cmd: "cartograph sync", built };
    case "needs-migrate": return { cls: "sh-chip-warn", text: "Index needs an upgrade", title: "The index needs an upgrade", body: "It was built by an older version. Upgrading keeps your corrections.", cmd: "cartograph migrate", built };
    case "none": return { cls: "", text: "Not indexed", title: "Not indexed yet", body: "Build the first index to see the map.", cmd: "cartograph sync", built: "" };
    default: return { cls: "", text: `v${f.indexVersion} · ${ago(f.builtAt)}`, title: "The index is current", body: "Every repository is at its indexed commit.", built };
  }
}

/** Header chip for index state (board G02). The chip opens a popover with the details for that state. */
export default function IndexStatus({ freshness = getFreshness(), loading = false, offline = false }: { freshness?: Freshness; loading?: boolean; offline?: boolean }) {
  const { open, setOpen, ref } = usePopover();
  const live = useLive();
  const cur = useWorkspaces().current;
  const liveRepos = useRepos(live ? cur?.id : undefined).data;
  if (loading) return <span className="sh-chip" aria-live="polite">Checking index…</span>;
  const d = describe(freshness, offline);
  // one source: per-repo counts come from the Freshness itself when it has them, so the list can't disagree with the chip
  const dirty = new Set((live ? liveRepos ?? [] : getRepos()).filter((r) => r.dirty).map((r) => r.name));
  const repos = (freshness.repos ?? getRepos().map((r) => ({ name: r.name, behind: r.behind, capped: false }))).map((r) => ({ ...r, dirty: dirty.has(r.name) }));
  return (
    <div className="sh-pop-wrap sh-is" ref={ref}>
      <button type="button" className={`sh-chip sh-chip-btn ${d.cls}`} aria-haspopup="dialog" aria-expanded={open} aria-live="polite" onClick={() => setOpen(!open)}>{d.text}</button>
      {open && (
        <section className="sh-pop sh-is-pop" role="dialog" aria-label="Index status">
          <b>{d.title}</b>
          <p className="sh-pop-note">{d.body}</p>
          {live && cur && (freshness.state === "behind" || freshness.state === "rewritten") && !offline && (
            // SRV-13: one sync per workspace; a second click joins the running job.
            <button type="button" className="btn" onClick={() => send("POST", `/api/v1/w/${encodeURIComponent(cur.id)}/jobs`, { kind: "sync", repo: null }).then(() => setOpen(false), (e) => toast(e instanceof RequestError ? e.what : "Couldn't start the sync."))}><RefreshCw size={13} aria-hidden />Sync now</button>
          )}
          {d.cmd && (
            <div className="sh-is-cmd"><code><span aria-hidden>$ </span>{d.cmd}</code><button type="button" className="btn sh-is-copy" aria-label="Copy command" onClick={() => copy(d.cmd!)}><Copy size={13} aria-hidden />Copy</button></div>
          )}
          {freshness.state === "syncing" && (
            <div role="progressbar" aria-label="Sync progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round((freshness.progress ?? 0) * 100)} className="sh-is-bar"><span style={{ width: `${(freshness.progress ?? 0) * 100}%` }} /></div>
          )}
          {freshness.state !== "none" && freshness.state !== "failed" && !offline && (
            <ul className="sh-is-repos">
              {repos.map((r) => (
                <li key={r.name}>
                  <span>{r.name}</span>
                  <span className="sh-is-r">{r.behind > 0 ? <em>{r.behind}{r.capped ? "+" : ""} behind</em> : <><Check size={13} aria-hidden />Up to date</>}{r.dirty && <small>uncommitted changes</small>}</span>
                </li>
              ))}
            </ul>
          )}
          {d.built && <small className="sh-pop-note">{d.built}</small>}
        </section>
      )}
    </div>
  );
}
