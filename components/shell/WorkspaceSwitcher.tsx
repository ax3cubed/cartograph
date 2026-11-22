"use client";

import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import Link from "next/link";
import { ChevronDown, FolderPlus, FileDown, X } from "lucide-react";
import { getCurrentWorkspace, getWorkspaces } from "@/lib/source";
import { usePopover } from "./usePopover";
import { toast } from "./toast";
import { rememberOrigin } from "./origin";
import "./workspaceSwitcher.css";

const KEY = "cartograph.recent-indexes";
interface Recent { name: string; file: string; at: number }
const load = (): Recent[] => { try { return JSON.parse(localStorage.getItem(KEY) ?? "[]"); } catch { return []; } };
const save = (r: Recent[]) => { try { localStorage.setItem(KEY, JSON.stringify(r)); } catch { /* storage can be blocked */ } };
const initials = (n: string) => n.replace(/\.cartograph$/, "").split(/[-_ ]/).map((p) => p[0]).join("").slice(0, 2).toUpperCase();

/** Decision I9 / board G01: the workspace menu. Recent indexes, open an index file, add a workspace, the demo workspace. */
export default function WorkspaceSwitcher() {
  const ws = getCurrentWorkspace();
  const { open, setOpen, ref } = usePopover();
  const [recents, setRecents] = useState<Recent[]>([]);
  const menu = useRef<HTMLDivElement>(null);
  const file = useRef<HTMLInputElement>(null);
  useEffect(() => { setRecents(load()); }, []);
  useEffect(() => { if (open) menu.current?.querySelector<HTMLElement>("[role=menuitem]")?.focus(); }, [open]);

  const nav = (e: KeyboardEvent) => {
    const items = [...(menu.current?.querySelectorAll<HTMLElement>("[role=menuitem]") ?? [])];
    const i = items.indexOf(document.activeElement as HTMLElement);
    const to = e.key === "ArrowDown" ? (i + 1) % items.length : e.key === "ArrowUp" ? (i - 1 + items.length) % items.length : e.key === "Home" ? 0 : e.key === "End" ? items.length - 1 : -1;
    if (to >= 0) { e.preventDefault(); items[to].focus(); }
  };
  const drop = (r: Recent) => { const next = recents.filter((x) => x.file !== r.file); setRecents(next); save(next); };
  const picked = (f?: File) => {
    if (!f) return;
    setOpen(false);
    // nothing is added to Recent: an entry the app can't open would be a dead end
    toast(f.name.endsWith(".cartograph")
      ? `Can't open ${f.name} yet. This version of Cartograph can't read index files. The northwind demo is available.`
      : `${f.name} isn't an index. Index files end in .cartograph.`);
  };

  return (
    <div className="sh-pop-wrap sh-ws-wrap" ref={ref}>
      <button type="button" className="sh-ws" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span className="sh-ws-text"><span>Workspace</span><b>{ws ? `${ws.name} · ${ws.repos} repos` : "No workspace"}</b></span>
        {ws?.demo && <span className="sh-chip sh-chip-demo">Demo data</span>}
        <ChevronDown size={14} aria-hidden className={open ? "sh-flip" : undefined} />
      </button>
      {open && (
        <div className="sh-pop sh-wsm" role="menu" aria-label="Workspaces" ref={menu} onKeyDown={nav}>
          <span className="sh-k">Recent indexes</span>
          {recents.length === 0 && <p className="sh-pop-note">None yet. Indexes you open will be listed here.</p>}
          {recents.map((r) => (
            <div className="sh-wsm-row" key={r.file}>
              <button type="button" role="menuitem" className="sh-wsm-item" onClick={() => { setOpen(false); toast(`${r.name} can't be opened in this build. Try the northwind demo.`); }}>
                <span className="sh-wsm-badge">{initials(r.name)}</span>
                <span className="sh-wsm-t"><b>{r.name}</b><small>Opened from disk</small></span>
              </button>
              <button type="button" className="sh-wsm-rm" aria-label={`Remove ${r.name} from recent indexes`} title="Remove from recent indexes. The index file stays on disk." onClick={() => drop(r)}><X size={15} aria-hidden /></button>
            </div>
          ))}
          {recents.length > 0 && <p className="sh-pop-note">Removing an index only clears it from this list. The file stays on disk.</p>}
          <input ref={file} type="file" accept=".cartograph" hidden onChange={(e) => { picked(e.target.files?.[0]); e.target.value = ""; }} />
          <button type="button" role="menuitem" className="sh-wsm-act" onClick={() => file.current?.click()}><FileDown size={17} aria-hidden />Open index file…</button>
          <Link href="/welcome/add" role="menuitem" className="sh-wsm-act" onClick={() => { rememberOrigin(); setOpen(false); }}><FolderPlus size={17} aria-hidden />Add workspace…</Link>
          <hr />
          <span className="sh-k">Demo workspace</span>
          {getWorkspaces().filter((w) => w.demo).map((w) => (
            <Link key={w.id} href="/" role="menuitem" className="sh-wsm-item" aria-current={w.id === ws?.id ? "true" : undefined} onClick={() => setOpen(false)}>
              <span className="sh-wsm-badge is-demo">{initials(w.name)}</span>
              <span className="sh-wsm-t"><b>{w.name}</b><small>{w.repos} repos · built into the app</small></span>
              <span className="sh-chip sh-chip-demo">Demo data</span>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
