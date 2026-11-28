"use client";

import { useMemo, useSyncExternalStore, type ReactNode } from "react";
import Link from "next/link";
import { Loader, type LucideIcon } from "lucide-react";
import EmptyState from "@/components/shell/EmptyState";
import type { Service } from "@/lib/api";
import { useFreshness, useGraph, useLive, useMode, useWorkspaces, useWsRead } from "@/lib/api/live";
import "./views.css";
import { fmtDate as fmtDay } from "@/lib/date";

const subscribe = (cb: () => void) => { window.addEventListener("hashchange", cb); return () => window.removeEventListener("hashchange", cb); };
/** The URL hash without "#": view state that must survive a reload lives here (decision A2). */
export const useHash = () => useSyncExternalStore(subscribe, () => decodeURIComponent(location.hash.slice(1)), () => "");

import type { Tone } from "./risk.ts";
export type { Tone };
export const Chip = ({ tone = "", children }: { tone?: Tone; children: ReactNode }) => <span className={`view-chip ${tone}`}>{children}</span>;
/** How sure a link is, as a tone: only an exact one is calm. */
export const levelTone = (l: string): Tone => (l === "exact" ? "ok" : "");
export const levelLabel = (l: string) => l[0].toUpperCase() + l.slice(1);

export const fmtDate = (iso: string) => fmtDay(iso, "dmy");

/** `#indexing` shows the first-index state on any view. */
export const IndexingEmpty = () => (
  <EmptyState icon={Loader} title="Building your first index" body="This view fills in as the index finishes. You can keep working. Progress is in the status at the top right." action={{ label: "See indexing settings", href: "/settings/indexing" }} />
);

/**
 * What every view needs first: the workspace it reads, and whether its first index is still running (O1). `ws` is
 * undefined without the binary (the demo build) or before a workspace is picked.
 */
export function useView() {
  const live = useLive();
  const ws = useWorkspaces().current?.id;
  const fresh = useFreshness(live ? ws : undefined).data;
  return { ws: live ? ws : undefined, live, indexing: !!fresh && (fresh.state === "none" || (fresh.state === "syncing" && fresh.builtAt === "")) };
}

/**
 * `when` is the real empty condition; `#empty` shows the same state for review (decision I9). The view's heading stays
 * above an empty state, so the page says where you are and what it will show.
 */
export function Page({ icon, head, empty, when, indexing, children }: { icon?: LucideIcon; head: { id: string; title: string; purpose: string }; empty?: { title: string; body: string; action?: { label: string; href?: string; onClick?: () => void } }; when?: boolean; indexing?: boolean; children: ReactNode }) {
  const hash = useHash();
  const mode = useMode();
  const instead = (node: ReactNode) => <main className="view-main" aria-labelledby={`${head.id}-h`}><ViewHead {...head} />{node}</main>;
  // Without the binary (pnpm dev, a static host) there is no index to read, and nothing stands in for one.
  if (mode === "demo") return instead(<EmptyState icon={icon} title="These views read your index" body="Nothing answered at this address, so there is no index to read. Open the app from the command line with cartograph open." />);
  if (hash === "indexing" || indexing) return instead(<IndexingEmpty />);
  if (empty && (when || hash === "empty")) return instead(<EmptyState icon={icon} {...empty} />);
  return <>{children}</>;
}

/** Hash state: `#key=value&key=value`. A bare `#value` (older links such as `/impact#<sha>`) is read as `bare`. */
export function useHashState() {
  const h = useHash();
  const q = h.includes("=");
  const p = new URLSearchParams(q ? h : "");
  const set = (patch: Record<string, string | undefined>) => {
    const raw = decodeURIComponent(location.hash.slice(1));
    const n = new URLSearchParams(raw.includes("=") ? raw : "");
    for (const [k, v] of Object.entries(patch)) { if (v === undefined) n.delete(k); else n.set(k, v); }
    history.replaceState(null, "", `${location.pathname}${location.search}#${n}`);
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  };
  return { bare: q ? "" : h, get: (k: string) => p.get(k) ?? undefined, set };
}

/** node id to name: the files from the workspace's graph (the map's own read, so it is already there), the services from …/services. */
export function useNames(ws: string | undefined) {
  const g = useGraph(ws, "").data;
  const services = useWsRead<Service[]>(ws, "services").data;
  return useMemo(() => {
    const m = new Map<string, string>();
    if (g) g.nodes.id.forEach((id, i) => m.set(id, g.nodes.name[i]));
    for (const s of services ?? []) m.set(s.id, s.name);
    return (id: string) => m.get(id) ?? id;
  }, [g, services]);
}
export const mapHref = (id: string, back: string, mode: "focus" | "trace" = "focus") => `/#${mode}=${encodeURIComponent(id)}&back=${back}`;

/** A node or path that opens the map. Inside a table row it is skipped by Tab, because the table is one tab stop. */
export function NodeLink({ id, back, label, inRow, mode }: { id: string; back: string; label: string; inRow?: boolean; mode?: "focus" | "trace" }) {
  return <Link className="view-mono view-link view-cell" href={mapHref(id, back, mode)} tabIndex={inRow ? -1 : undefined}>{label}</Link>;
}

export const Cell = ({ className = "", children }: { className?: string; children: ReactNode }) => <span role="gridcell" className={`view-cell ${className}`}>{children}</span>;

const ASIDE = "view-aside";

export const KeyHint = ({ t }: { t: string }) => <p className="view-note">Arrow keys move between rows. Enter opens the details. <kbd>t</kbd> {t}</p>;

export function ViewHead({ id, title, purpose, actions }: { id: string; title: string; purpose: string; actions?: ReactNode }) {
  return (
    <div className="view-head">
      <div><h1 id={`${id}-h`} className="view-h1">{title}</h1><span className="view-sub">{purpose}</span></div>
      {actions}
    </div>
  );
}

/** The one view layout: heading and purpose, a toolbar, a table, and a 400px aside. */
export function ViewSplit({ id, title, purpose, actions, toolbar, aside, asideLabel, children }: { id: string; title: string; purpose: string; actions?: ReactNode; toolbar?: ReactNode; aside: ReactNode; asideLabel: string; children: ReactNode }) {
  return (
    <div className="view-split">
      <section aria-labelledby={`${id}-h`}>
        <ViewHead id={id} title={title} purpose={purpose} actions={actions} />
        {toolbar && <div className="view-toolbar">{toolbar}</div>}
        {children}
      </section>
      <aside id={ASIDE} tabIndex={-1} className="view-aside" aria-label={asideLabel}>{aside}</aside>
    </div>
  );
}

/** Actions sit at the top of every aside, never pinned to the bottom. */
export function AsideTop({ map, back, mode = "focus", children }: { map?: string; back: string; mode?: "focus" | "trace"; children?: ReactNode }) {
  return (
    <div className="view-aside-top">
      {map && <Link className="btn view-btn primary" href={mapHref(map, back, mode)}>Show on map</Link>}
      {children}
    </div>
  );
}
