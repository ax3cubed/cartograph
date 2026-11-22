"use client";

import { Check, Copy } from "lucide-react";
import { getFreshness, getRepos } from "@/lib/source";
import type { Freshness } from "@/lib/model";
import { usePopover } from "./usePopover";
import { ago } from "./ago";
import { toast } from "./toast";
import "./indexStatus.css";

export const copy = (text: string) =>
  navigator.clipboard?.writeText(text).then(() => toast("Copied to the clipboard"), () => toast("Couldn't copy. Select the command and copy it by hand."));

/** Per state: chip class, chip text, popover title, popover body. */
function describe(f: Freshness) {
  const built = `Built ${ago(f.builtAt)} · index v${f.indexVersion}`;
  switch (f.state) {
    case "failed": return { cls: "sh-chip-down", text: f.error?.what ?? "Index failed", title: "The index can't be read", body: f.error ? `${f.error.what}. ${f.error.action}` : "Rebuild the index.", cmd: "cartograph index --rebuild", built };
    case "behind": return { cls: "sh-chip-warn", text: `${f.behind} commits behind HEAD`, title: `${f.behind} commits behind HEAD`, body: "Links added or removed since the last build aren't on the map. Search and Ask still use the older index until you update it.", cmd: "cartograph sync", built };
    case "syncing": return { cls: "sh-chip-warn", text: `Syncing ${Math.round((f.progress ?? 0) * 100)}%`, title: "Catching up", body: "A sync is running. The map stays usable and updates when it finishes.", built };
    case "rewritten": return { cls: "sh-chip-warn", text: "History was rewritten", title: "History was rewritten", body: "A rebase or force-push changed commits the index had read. Rebuild so the timeline matches git.", cmd: "cartograph index --rebuild", built };
    case "needs-migrate": return { cls: "sh-chip-warn", text: "Index needs an upgrade", title: "The index needs an upgrade", body: "It was built by an older version. Upgrading keeps your corrections.", cmd: "cartograph migrate", built };
    case "none": return { cls: "", text: "Not indexed", title: "Not indexed yet", body: "Build the first index to see the map.", cmd: "cartograph index", built: "" };
    default: return { cls: "", text: `v${f.indexVersion} · ${ago(f.builtAt)}`, title: "The index is current", body: "Every repository is at its indexed commit.", built };
  }
}

/** Header chip for index state (board G02). The chip opens a popover with the details for that state. */
export default function IndexStatus({ freshness = getFreshness(), loading = false }: { freshness?: Freshness; loading?: boolean }) {
  const { open, setOpen, ref } = usePopover();
  if (loading) return <span className="sh-chip" aria-live="polite">Checking index…</span>;
  const d = describe(freshness);
  // one source: per-repo counts come from the Freshness itself when it has them, so the list can't disagree with the chip
  const dirty = new Set(getRepos().filter((r) => r.dirty).map((r) => r.name));
  const repos = (freshness.repos ?? getRepos().map((r) => ({ name: r.name, behind: r.behind }))).map((r) => ({ ...r, dirty: dirty.has(r.name) }));
  return (
    <div className="sh-pop-wrap sh-is" ref={ref}>
      <button type="button" className={`sh-chip sh-chip-btn ${d.cls}`} aria-haspopup="dialog" aria-expanded={open} aria-live="polite" onClick={() => setOpen(!open)}>{d.text}</button>
      {open && (
        <section className="sh-pop sh-is-pop" role="dialog" aria-label="Index status">
          <b>{d.title}</b>
          <p className="sh-pop-note">{d.body}</p>
          {d.cmd && (
            <div className="sh-is-cmd"><code><span aria-hidden>$ </span>{d.cmd}</code><button type="button" className="btn sh-is-copy" aria-label="Copy command" onClick={() => copy(d.cmd!)}><Copy size={13} aria-hidden />Copy</button></div>
          )}
          {freshness.state === "syncing" && (
            <div role="progressbar" aria-label="Sync progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round((freshness.progress ?? 0) * 100)} className="sh-is-bar"><span style={{ width: `${(freshness.progress ?? 0) * 100}%` }} /></div>
          )}
          {freshness.state !== "none" && freshness.state !== "failed" && (
            <ul className="sh-is-repos">
              {repos.map((r) => (
                <li key={r.name}>
                  <span>{r.name}</span>
                  <span className="sh-is-r">{r.behind > 0 ? <em>{r.behind} behind</em> : <><Check size={13} aria-hidden />Up to date</>}{r.dirty && <small>uncommitted changes</small>}</span>
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
