"use client";

import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { usePath } from "./usePath";
import { ChevronDown, Search, Settings } from "lucide-react";
import AiMark from "@/components/brand/AiMark";
import { getWorkspaces } from "@/lib/source";
import WorkspaceSwitcher from "./WorkspaceSwitcher";
import IndexStatus from "./IndexStatus";
import SearchPalette from "./SearchPalette";
import Activity from "./Activity";
import IndexOverlay, { IndexBand } from "./IndexOverlay";
import Toasts from "./Toasts";
import { useIndexState } from "./useIndexState";
import { useLive, useWorkspaces } from "@/lib/api/live";
import "./shell.css";

export const VIEW_TABS = [
  { href: "/", label: "Map" },
  { href: "/entrypoints", label: "Entrypoints" },
  { href: "/contracts", label: "Contracts" },
  { href: "/concepts", label: "Concepts" },
  { href: "/coverage", label: "Coverage" },
  { href: "/impact", label: "Impact" },
] as const;

/** Window event the Ask trigger fires; the map listens for it and focuses the Ask bar. */
export const ASK_EVENT = "cartograph:ask";

/**
 * The top bar (decision I9): brand, workspace switcher, index status, view tabs, search, Ask, activity, settings.
 * `onPick` receives what the search palette picked (a node id on the map). `extra` is a slot before Ask (the map's pins).
 */
export default function AppShell({ onPick, extra }: { onPick?: (id: string, kind: string) => void; extra?: ReactNode }) {
  const raw = usePath();
  const path = raw.length > 1 ? raw.replace(/\/$/, "") : raw;
  const router = useRouter();
  const [searching, setSearching] = useState(false);
  const idx = useIndexState();
  const active = path;
  const live = useLive();
  const listed = useWorkspaces().data;

  // first run (D2): no workspace yet, so the welcome screen. With the binary, the registry decides.
  useEffect(() => { if (live ? listed?.length === 0 : !getWorkspaces().length) router.replace("/welcome"); }, [router, live, listed]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") { e.preventDefault(); setSearching(true); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  // off the map, a pick opens the matching view; node picks open the map focused on that node (the map reads #focus=)
  const pick = onPick ?? ((id: string, kind: string) => router.push(kind === "concept" ? "/concepts" : kind === "operation" ? "/contracts" : `/#focus=${encodeURIComponent(id)}`));
  const ask = () => {
    if (path === "/") window.dispatchEvent(new Event(ASK_EVENT));
    else router.push("/#ask");
  };

  return (
    <>
    <header className="top sh-top">
      <Link href="/" className="brand" aria-label="Cartograph, back to the map">
        <svg viewBox="0 0 48 48" aria-hidden><path className="ring" d="M35.31 12.69A16 16 0 1 0 35.31 35.31" /><path className="spoke" d="M24 24 35.31 12.69" /><circle cx="24" cy="24" r="5.2" /><circle cx="35.31" cy="12.69" r="4.4" /></svg>
        <span>cartograph</span>
      </Link>
      <WorkspaceSwitcher />
      <nav className="sh-tabs" aria-label="Views">
        {VIEW_TABS.map((t) => <Link key={t.href} href={t.href} aria-current={active === t.href ? "page" : undefined}>{t.label}</Link>)}
      </nav>
      {/* phones: the tabs become one select so the header never scrolls the page */}
      <label className="sh-tabsel">
        <span className="sh-sr">View</span>
        <select value={VIEW_TABS.some((t) => t.href === active) ? active : ""} onChange={(e) => router.push(e.target.value)}>
          {!VIEW_TABS.some((t) => t.href === active) && <option value="" disabled>Settings</option>}
          {VIEW_TABS.map((t) => <option key={t.href} value={t.href}>{t.label}</option>)}
        </select>
        <ChevronDown size={14} aria-hidden />
      </label>
      <button type="button" className="sh-search" onClick={() => setSearching(true)} aria-label="Search the workspace" aria-keyshortcuts="Control+K Meta+K">
        <Search size={16} aria-hidden /><span>Search services, files, concepts or operations</span><kbd>⌘K</kbd>
      </button>
      <div className="sh-right">
        <IndexStatus freshness={idx.freshness} loading={idx.loading} offline={idx.offline} />
        {extra}
        <button type="button" className="sh-ask" onClick={ask} aria-label="Ask" aria-keyshortcuts="/" aria-current={path === "/ask" ? "page" : undefined}><AiMark size={16} style={{ "--aimark-accent": "currentColor" } as CSSProperties} /><span>Ask</span></button>
        <Activity freshness={idx.freshness} />
        <Link href="/settings/connections" className="sh-icon" aria-label="Settings" aria-current={path.startsWith("/settings") ? "page" : undefined}><Settings size={17} aria-hidden /></Link>
      </div>
      <IndexOverlay state={idx.state} why={idx.why} onClose={idx.clear} />
      <Toasts />
      <SearchPalette open={searching} onClose={() => setSearching(false)} onPick={pick} />
    </header>
    {/* STATES slot: a slim row in normal flow right after the header; renders nothing unless the index is behind */}
    <IndexBand state={idx.state} freshness={idx.freshness} onClose={idx.clear} />
    </>
  );
}

/** Frame for every route except the map: the top bar, then the page below it. */
export function ShellFrame({ children }: { children: ReactNode }) {
  return <div className="sh-frame"><AppShell /><div className="sh-body">{children}</div></div>;
}
