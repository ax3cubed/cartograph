"use client";

import { useRef, useSyncExternalStore, type KeyboardEvent, type ReactNode } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Loader, type LucideIcon } from "lucide-react";
import { byId } from "@/lib/workspace";
import EmptyState from "@/components/shell/EmptyState";
import { levelOf, type ConfidenceLevel } from "@/lib/model";
import "./views.css";

const subscribe = (cb: () => void) => { window.addEventListener("hashchange", cb); return () => window.removeEventListener("hashchange", cb); };
/** The URL hash without "#": view state that must survive a reload lives here (decision A2). */
export const useHash = () => useSyncExternalStore(subscribe, () => decodeURIComponent(location.hash.slice(1)), () => "");

export type Tone = "ok" | "warn" | "bad" | "acc" | "";
export const Chip = ({ tone = "", children }: { tone?: Tone; children: ReactNode }) => <span className={`view-chip ${tone}`}>{children}</span>;
export const toneOf = (score: number): Tone => (levelOf(score) === "exact" ? "ok" : "");
export const levelLabel = (l: ConfidenceLevel) => l[0].toUpperCase() + l.slice(1);

export const fmtDate = (iso: string) => new Date(`${iso}T12:00:00Z`).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });

/** `#indexing` shows the first-index state on any view. */
export const IndexingEmpty = () => (
  <EmptyState icon={Loader} title="Building your first index" body="This view fills in as the index finishes. You can keep working. Progress is in the status at the top right." action={{ label: "See indexing settings", href: "/settings/indexing" }} />
);

/** `when` is the real empty condition; `#empty` shows the same state for review (decision I9). */
export function Page({ icon, empty, when, children }: { icon?: LucideIcon; empty: { title: string; body: string; action?: { label: string; href?: string; onClick?: () => void } }; when: boolean; children: ReactNode }) {
  const hash = useHash();
  if (hash === "indexing") return <IndexingEmpty />;
  if (when || hash === "empty") return <EmptyState icon={icon} {...empty} />;
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

/** A repo-relative path is the one path format in every view. */
export const pathOf = (id: string) => byId.get(id)?.path ?? id;
export const mapHref = (id: string, back: string, mode: "focus" | "trace" = "focus") => `/#${mode}=${encodeURIComponent(id)}&back=${back}`;

/** A node or path that opens the map. Inside a table row it is skipped by Tab, because the table is one tab stop. */
export function NodeLink({ id, back, label, inRow, mode }: { id: string; back: string; label?: string; inRow?: boolean; mode?: "focus" | "trace" }) {
  return <Link className="view-mono view-link view-cell" href={mapHref(id, back, mode)} tabIndex={inRow ? -1 : undefined}>{label ?? pathOf(id)}</Link>;
}

export const Cell = ({ className = "", children }: { className?: string; children: ReactNode }) => <span role="gridcell" className={`view-cell ${className}`}>{children}</span>;

const ASIDE = "view-aside";
/** One tab stop for a table: arrows move and select, Enter moves focus to the aside, `t` opens the row on the map. */
export function useRoving(ids: string[], sel: string | undefined, select: (id: string) => void, traceHref?: (id: string) => string | undefined) {
  const ref = useRef<HTMLDivElement>(null);
  const router = useRouter();
  const onKeyDown = (e: KeyboardEvent) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-rid]");
    if (!row || e.target !== row) return;
    const id = row.dataset.rid!; const i = ids.indexOf(id);
    const move = (n: number) => { const to = ids[Math.max(0, Math.min(ids.length - 1, n))]; if (!to) return; select(to); ref.current?.querySelector<HTMLElement>(`[data-rid="${CSS.escape(to)}"]`)?.focus(); };
    if (e.key === "ArrowDown") { e.preventDefault(); move(i + 1); }
    else if (e.key === "ArrowUp") { e.preventDefault(); move(i - 1); }
    else if (e.key === "Home") { e.preventDefault(); move(0); }
    else if (e.key === "End") { e.preventDefault(); move(ids.length - 1); }
    else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(id); document.getElementById(ASIDE)?.focus(); }
    else if (e.key === "t" && !e.metaKey && !e.ctrlKey) { const h = traceHref?.(id); if (h) { e.preventDefault(); router.push(h); } }
  };
  const row = (id: string) => ({ "data-rid": id, role: "row", tabIndex: id === sel ? 0 : -1, "aria-selected": id === sel, onClick: () => select(id) });
  return { ref, onKeyDown, row };
}

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
