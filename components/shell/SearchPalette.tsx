"use client";

import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useRouter } from "next/navigation";
import { usePath } from "./usePath";
import { ArrowRight, Clock, Search } from "lucide-react";
import type { Node, SearchResult } from "@/lib/api";
import { load } from "@/lib/api/cache";
import { nodeRoute, useLoaded, useSearch, useStructure, useWorkspaces } from "@/lib/api/live";
import "./searchPalette.css";

// FE-10: …/search on the index, grouped by kind, with board G05's "did you mean" when nothing matched.
type Kind = SearchResult["kind"];
const GROUPS: { kind: Kind; label: string }[] = [
  { kind: "service", label: "Services" }, { kind: "module", label: "Modules" }, { kind: "file", label: "Files" },
  { kind: "symbol", label: "Symbols" }, { kind: "concept", label: "Concepts" }, { kind: "operation", label: "Operations" },
];
const PER_GROUP = 12; // the All scope shows the top rows of each kind; a kind scope shows them all
const KEY = "cartograph.recent-search"; // ids only (FE-10): names are read back from the index
const loadRecent = (): string[] => { try { const v = JSON.parse(localStorage.getItem(KEY) ?? "[]"); return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []; } catch { return []; } };

const Hit = ({ text, q }: { text: string; q: string }) => {
  const i = q ? text.toLowerCase().indexOf(q.toLowerCase()) : -1;
  return i < 0 ? <>{text}</> : <>{text.slice(0, i)}<mark>{text.slice(i, i + q.length)}</mark>{text.slice(i + q.length)}</>;
};

/**
 * Board G05: grouped results with kind scopes, keyboard navigation and recent picks.
 * Open with open={true}; the shell opens it on Cmd/Ctrl+K. onPick gets a node id (or concept / operation id).
 */
export default function SearchPalette({ open, onClose, onPick }: { open: boolean; onClose: () => void; onPick?: (id: string, kind: string) => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const router = useRouter();
  const path = usePath();
  const [q, setQ] = useState("");
  const [scope, setScope] = useState<"all" | Kind>("all");
  const [cur, setCur] = useState(0);
  const [recentIds, setRecent] = useState<string[]>([]);
  const ws = useWorkspaces().current?.id;
  const recent = useLoaded(ws, open && recentIds.length ? `recent:${recentIds.join(",")}` : null, async () => {
    const got = await Promise.all(recentIds.map((id) => load<{ node: Node }>(ws!, nodeRoute(ws, id)!).then((d) => d.node, () => null)));
    return got.filter((n): n is Node => !!n).map((n): SearchResult => ({ id: n.id, kind: n.kind === "workspace" ? "service" : (n.kind as Kind), name: n.name, path: n.path, service: n.service }));
  }).data ?? [];
  const svc = useStructure(open ? ws : undefined).data?.services;
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) { setQ(""); setScope("all"); setCur(0); setRecent(loadRecent()); d.showModal(); } else if (!open && d.open) d.close();
  }, [open]);

  const term = q.trim();
  const found = useSearch(ws, q);
  const all = useMemo(() => found.data?.results ?? [], [found.data]);
  const counts = useMemo(() => Object.fromEntries(GROUPS.map((g) => [g.kind, all.filter((h) => h.kind === g.kind).length])) as Record<Kind, number>, [all]);
  // flat list in display order: this is what the arrow keys walk
  const rows = useMemo(() => {
    if (!term) return recent.map((r) => ({ ...r, group: "Recent" }));
    return GROUPS.filter((g) => scope === "all" || scope === g.kind).flatMap((g) =>
      all.filter((h) => h.kind === g.kind).slice(0, scope === "all" ? PER_GROUP : 50).map((h) => ({ ...h, group: g.label })));
  }, [term, all, scope, recent]);
  const hidden = term && scope === "all" ? all.length - rows.length : 0;
  useEffect(() => setCur(0), [q, scope]);
  useEffect(() => { document.getElementById(`sp-${cur}`)?.scrollIntoView({ block: "nearest" }); }, [cur]);

  const pick = (h: SearchResult) => {
    try { localStorage.setItem(KEY, JSON.stringify([h.id, ...loadRecent().filter((r) => r !== h.id)].slice(0, 5))); } catch { /* storage can be blocked */ }
    onPick?.(h.id, h.kind);
    onClose();
  };
  /** Hands the question to the map's Ask as /#ask=<question>. On the map itself only the hash changes. */
  const askAbout = () => {
    const to = `/#ask=${encodeURIComponent(`Where is ${term}?`)}`;
    onClose();
    if (path === "/") location.hash = to.slice(2);
    else router.push(to);
  };
  const tabs = [{ kind: "all" as const, label: "All", n: all.length }, ...GROUPS.filter((g) => counts[g.kind]).map((g) => ({ kind: g.kind, label: g.label, n: counts[g.kind] }))];
  const key = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") { e.preventDefault(); setCur((c) => Math.min(c + 1, rows.length - 1)); }
    else if (e.key === "ArrowUp") { e.preventDefault(); setCur((c) => Math.max(c - 1, 0)); }
    else if (e.key === "Enter" && rows[cur]) { e.preventDefault(); pick(rows[cur]); }
    else if (e.key === "Tab" && term) { // Tab moves to the next result kind
      e.preventDefault();
      const i = tabs.findIndex((t) => t.kind === scope);
      setScope(tabs[(i + (e.shiftKey ? -1 : 1) + tabs.length) % tabs.length].kind);
    }
  };
  const near = term && !rows.length && !found.loading ? found.data?.near ?? undefined : undefined;
  const hue = (h: SearchResult) => { const k = svc?.findIndex((x) => x.id === h.service) ?? -1; return k >= 0 && k < 6 ? `var(--hue-${k + 1})` : "var(--steel)"; };

  let lastGroup = "";
  return (
    <dialog ref={ref} className="sh-palette" aria-label="Search the workspace" onClose={onClose} onClick={(e) => e.target === ref.current && onClose()}>
      <div className="sh-sp-inp">
        <Search size={18} aria-hidden />
        <input autoFocus value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={key} placeholder="Search services, files, concepts or operations" aria-label="Search"
          role="combobox" aria-expanded="true" aria-controls="sp-list" aria-activedescendant={rows.length ? `sp-${cur}` : undefined} autoComplete="off" spellCheck={false} />
        {term && <span className="sh-sp-n" aria-live="polite">{found.loading && !found.data ? "Searching…" : `${all.length} ${all.length === 1 ? "match" : "matches"}`}</span>}
      </div>
      {term && all.length > 0 && (
        <div className="sh-sp-tabs" role="tablist" aria-label="Result kind">
          {tabs.map((t) => <button key={t.kind} type="button" role="tab" tabIndex={-1} aria-selected={scope === t.kind} onClick={() => setScope(t.kind)}>{t.label} {t.n}</button>)}
        </div>
      )}
      {term && !rows.length && !found.loading ? (
        <div className="sh-sp-none">
          <b>No match for “{term}”</b>
          <span>Search covers names and paths in the open index, not what's inside files. Ask can look further.</span>
          {near && <><span className="sh-k">Did you mean</span><button type="button" className="sh-sp-row" onClick={() => pick(near)}><Dot h={near} c={hue(near)} /><span className="sh-sp-t"><span>{near.name}</span><span className="sh-sp-p">{near.path}</span></span><span /></button></>}
          <button type="button" className="sh-sp-ask" onClick={askAbout}><ArrowRight size={16} aria-hidden />Ask “Where is {term}?”</button>
        </div>
      ) : (
        <ul id="sp-list" role="listbox" aria-label={term ? "Results" : "Recent"} className="sh-sp-list">
          {!term && !rows.length && <li className="sh-sp-empty" role="presentation">Type to search services, modules, files, concepts and operations. Your recent picks show here.</li>}
          {rows.map((h, i) => {
            const head = h.group !== lastGroup ? (lastGroup = h.group) : null;
            return (
              <li key={h.id} role="presentation">
                {head && <div className="sh-k sh-sp-grp">{!term && <Clock size={12} aria-hidden />}{head}</div>}
                <div id={`sp-${i}`} role="option" aria-selected={i === cur} className="sh-sp-row" onMouseMove={() => setCur(i)} onClick={() => pick(h)}>
                  <Dot h={h} c={hue(h)} />
                  <span className="sh-sp-t"><span><Hit text={h.name} q={term} /></span><span className="sh-sp-p">{h.path}</span></span>
                  <span className="sh-sp-kind">{h.kind}</span>
                </div>
              </li>
            );
          })}
          {hidden > 0 && <li className="sh-sp-more" role="presentation">{hidden} more in the kind tabs above</li>}
        </ul>
      )}
      <div className="sh-sp-keys" aria-label="Keyboard shortcuts">
        <span><kbd>↑↓</kbd>move</span><span><kbd>↵</kbd>open</span><span><kbd>Tab</kbd>next result kind</span><span><kbd>esc</kbd>close</span>
      </div>
    </dialog>
  );
}

/** The service's hue, as on the map; files get a ring. */
const Dot = ({ h, c }: { h: SearchResult; c: string }) => {
  return <span className={`sh-sp-dot ${h.kind === "file" ? "is-ring" : ""}`} style={{ "--c": c } as React.CSSProperties} aria-hidden />;
};
