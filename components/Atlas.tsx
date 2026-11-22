"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeftRight, ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight, Copy, Flag, ListFilter, LogIn, LogOut, MessageSquare, Orbit, PanelRight, Route,
  SearchX, SlidersHorizontal, Star, StarOff, TriangleAlert, X,
} from "lucide-react";
import Link from "next/link";
import Graph, { type Depth, type Layout } from "./Graph";
import { AskDocked } from "./Ask";
import AskFloat from "./ask/Float";
import { askStore, findTurn } from "./ask/store";
import type { Answer } from "@/lib/ask";
import Panel, { Delta } from "./Panel";
import AppShell, { ASK_EVENT, VIEW_TABS } from "./shell/AppShell";
import AddedHint from "./shell/AddedHint";
import Sidebar, { showSidebar } from "./sidebar/Sidebar";
import { useRoving } from "./sidebar/useRoving";
import Timeline from "./timeline/Timeline";
import EmptyState from "./shell/EmptyState";
import Legend from "./map/Legend";
import CorrectionDialog from "./map/CorrectionDialog";
import NodeMenu, { isMenuKey, longPress, onMenuEvent, type MenuItem, type MenuReq, type MenuTarget } from "./map/NodeMenu";
import { toast } from "./shell/toast";
import { serviceHue } from "./map/domains";
import { applyCorrections, between, neighboursOf, reach, score, traceStart, visibleIds, type EffLink } from "./map/links";
import { HIDDEN_BELOW, type Correction, type LinkType } from "@/lib/model";
import { getCorrections, getEntrypoints } from "@/lib/source";
import { loadKeymap } from "@/lib/settings";
import { symbolById, symbolsIn } from "@/lib/demo/symbols";
import {
  COMMITS, NODES, WORKSPACE, SERVICES, byId, children, commitBySha, dayMs, metrics,
} from "@/lib/workspace";
import "./map/map.css";

const serviceColor = serviceHue;
const RANKS = { workspace: 0, service: 1, module: 2, file: 3 };
const fmtDay = (ms: number) => new Date(ms).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });

const DAY = 86_400_000;
const RAIL_KEY = "cartograph:rail";
/** Label for a `back=` route: the view tab's name, else the last path segment. */
const backLabel = (route: string) => VIEW_TABS.find((v) => v.href === route)?.label ?? (route.split("/").filter(Boolean).pop() ?? "map").replace(/^./, (c) => c.toUpperCase());
const TODAY = dayMs("2026-11-07"); // ponytail: pinned "now" for the mock; use Date.now() with real data

export default function Atlas() {
  const [layout, setLayout] = useState<Layout>("clusters");
  const [depthSetting, setDepthSetting] = useState<Depth | "auto">("auto");
  const [autoDepth, setAutoDepth] = useState<Depth>(2);
  const onDepth = useCallback((d: Depth) => setAutoDepth(d), []);
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [focusId, setFocusId] = useState<string | null>(null);
  const [scopeId, setScopeId] = useState<string>(WORKSPACE);
  const [t, setT] = useState(TODAY);
  const [typeFilter, setTypeFilter] = useState<LinkType | "all">("all");
  const [minUsedBy, setMinUsedBy] = useState(0);
  const [expanded, setExpanded] = useState<Set<string>>(new Set(["billing"]));
  const [pinned, setPinned] = useState<Set<string>>(new Set());
  const [compare, setCompare] = useState<{ a: number; b: number } | null>(null);
  const [corrections, setCorrections] = useState<Correction[]>(getCorrections);
  const [showWeak, setShowWeak] = useState(false);
  const [edge, setEdge] = useState<{ a: string; b: string } | null>(null);
  const [dlg, setDlg] = useState<{ link: EffLink | null; from?: string } | null>(null);
  const [symbolId, setSymbolId] = useState<string | null>(null);
  const [trace, setTrace] = useState<string | null>(null);
  const [treeQ, setTreeQ] = useState("");
  const [saved, setSaved] = useState("");
  const [railOpen, setRailOpen] = useState(true);
  const [backRoute, setBackRoute] = useState<string | null>(null);
  const [askQ, setAskQState] = useState<string | undefined>(undefined);
  // Clear, then set on the next frame: Ask re-arms on the empty value, so the same question can be handed over twice.
  const setAskQ = useCallback((q: string | undefined) => { setAskQState(undefined); if (q) requestAnimationFrame(() => setAskQState(q)); }, []);
  const [unread, setUnread] = useState(false);
  const [statusSlot, setStatusSlot] = useState<HTMLElement | null>(null);
  const [menu, setMenu] = useState<MenuReq | null>(null);
  const filtersRef = useRef<HTMLDetailsElement>(null);
  const treeRef = useRef<HTMLDivElement>(null);
  const treeRove = useRoving(treeRef, ".row-main", '.row-main[aria-selected="true"]');

  const eff = useMemo(() => applyCorrections(corrections), [corrections]);
  const links = useMemo(() => (showWeak ? eff.links : eff.links.filter((l) => score(l) >= HIDDEN_BELOW)), [eff, showWeak]);
  const weakCount = eff.links.filter((l) => score(l) < HIDDEN_BELOW).length;

  useEffect(() => {
    if (window.matchMedia("(max-width: 1180px)").matches) setExpanded(new Set()); // tree starts collapsed on narrow screens
    try {
      const r = localStorage.getItem(RAIL_KEY);
      setRailOpen(r === null ? !window.matchMedia("(max-width: 900px)").matches : r === "1");
    } catch { /* storage unavailable */ }
    try { setPinned(new Set(JSON.parse(localStorage.getItem("cartograph:pins") ?? "[]"))); } catch { /* storage unavailable */ }
  }, []);
  const toggleRail = () => setRailOpen((o) => {
    try { localStorage.setItem(RAIL_KEY, o ? "0" : "1"); } catch { /* storage unavailable */ }
    return !o;
  });
  // The hash is &-separated key=value pairs: #focus=<id>, #trace=<id>, #commit=<sha>, each with an optional &back=<route>
  // (the view that sent us here), #ask or #ask=<question> (opens Ask in the sidebar). #added= belongs to AddedHint.
  const fromHash = () => {
    const p = new URLSearchParams(location.hash.slice(1));
    const b = p.get("back");
    setBackRoute(b && b.startsWith("/") && !b.startsWith("//") && b !== "/" ? b : null);
    if (p.has("ask")) {
      const q = p.get("ask")?.trim();
      if (q) setAskQ(q);
      window.dispatchEvent(new Event(ASK_EVENT));
    }
    const an = p.get("answer"); // an answer from the /ask page: light its evidence here
    const hit = an ? findTurn(an) : null;
    if (hit) { askStore.openThread(hit.thread.id); showAnswer(hit.turn.a); }
    const f = p.get("focus");
    if (f && byId.has(f)) open(f);
    const tr = p.get("trace");
    if (tr) { setTrace(traceStart(tr).length ? tr : null); setFocusId(null); setAnswer(null); }
    const c = p.get("commit");
    const cm = c ? commitBySha.get(c) : undefined;
    if (cm) setT(dayMs(cm.date));
  };
  useEffect(() => {
    const ask = () => setUnread(false); // the sidebar opens Ask and focuses the composer itself
    window.addEventListener(ASK_EVENT, ask);
    window.addEventListener("hashchange", fromHash);
    fromHash();
    return () => { window.removeEventListener(ASK_EVENT, ask); window.removeEventListener("hashchange", fromHash); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const clearTrace = () => { setTrace(null); history.replaceState(null, "", location.pathname + location.search); };
  const togglePin = (id: string) => {
    const next = new Set(pinned);
    next.has(id) ? next.delete(id) : next.add(id);
    setPinned(next);
    try { localStorage.setItem("cartograph:pins", JSON.stringify([...next])); } catch { /* storage unavailable */ }
    return next.has(id);
  };

  // click = select (panel + emphasis), double-click / Focus button = open the ego view
  const select = (id: string) => {
    setEdge(null);
    if (focusId) return open(id);
    setScopeId(scopeId === id ? WORKSPACE : id);
  };
  const open = (id: string) => {
    setScopeId(id);
    setEdge(null);
    setSymbolId(null);
    if (id === WORKSPACE) return back();
    setFocusId(id);
  };
  const back = () => setFocusId(null);
  const showAnswer = (a: Answer | null) => {
    setAnswer(a);
    const s = askStore.get();
    if (a && !document.querySelector('.sb.is-open[data-tab="ask"]') && !(s.mode === "floating" && s.float === "open")) setUnread(true);
    if (a) setFocusId(null); // answers are shown on the map, where the evidence lives
  };
  const traced = useMemo(() => (trace ? reach(traceStart(trace), eff.links, t) : null), [trace, eff, t]);
  const highlight = useMemo(() => (answer && answer.nodes.length ? { nodes: new Set(answer.nodes), links: new Set(answer.links) } : traced), [answer, traced]);
  const traceName = trace ? (getEntrypoints().find((e) => e.id === trace)?.label ?? byId.get(trace)?.name ?? trace) : "";

  const depth = depthSetting === "auto" ? autoDepth : depthSetting;
  const hi = compare ? Math.max(compare.a, compare.b) : t;
  const nodeCount = (min: number, kind: LinkType | "all") => {
    const v = visibleIds(eff.links, depth, hi, kind, min, showWeak);
    return NODES.filter((n) => v.has(n.id) && RANKS[n.kind] <= depth && n.kind !== "workspace").length;
  };
  const filtersOn = (minUsedBy > 0 ? 1 : 0) + (typeFilter !== "all" ? 1 : 0);
  const total = NODES.filter((n) => n.kind !== "workspace" && RANKS[n.kind] <= depth).length;
  const shownCount = filtersOn ? nodeCount(minUsedBy, typeFilter) : total;
  const mapEmpty = !focusId && !highlight && filtersOn > 0 && shownCount === 0;
  const resetFilters = () => { setMinUsedBy(0); setTypeFilter("all"); };
  const lowerTo = (() => { for (let n = minUsedBy - 1; n >= 0; n--) if (nodeCount(n, typeFilter) > 0) return n; return null; })();

  const focusNode0 = focusId ? byId.get(focusId) : null;
  const canSymbols = !!focusNode0 && symbolsIn(focusNode0.id).length > 0;
  const showSymbol = (id: string) => {
    setSymbolId(id); setEdge(null);
    const f = symbolById(id)!.file;
    setFocusId(f); setScopeId(f);
  };
  const onCorrect = (link: EffLink | null, from?: string) => setDlg({ link, from });

  // keys from Settings > Keyboard; arrows, 1 2 3, + − and [ ] are fixed
  const keys = useRef(loadKeymap());
  useEffect(() => { keys.current = loadKeymap(); }, [dlg]);
  const act = useRef<Record<string, () => void>>({});
  act.current = {
    pin: () => scopeId !== WORKSPACE && togglePin(scopeId),
    now: () => setT(TODAY),
    ask: () => window.dispatchEvent(new Event(ASK_EVENT)),
    link: () => {
      const { dependsOn, usedBy } = neighboursOf(links, scopeId, t);
      const first = [...dependsOn.keys()][0] ?? [...usedBy.keys()][0];
      if (first) setEdge(dependsOn.has(first) ? { a: scopeId, b: first } : { a: first, b: scopeId });
    },
    back: () => (symbolId ? setSymbolId(null) : edge ? setEdge(null) : focusId ? back() : trace ? clearTrace() : undefined),
  };
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (dlg || e.metaKey || e.ctrlKey || e.altKey || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) return;
      if (e.key === "[") { e.preventDefault(); toggleRail(); return; }
      const k = e.key.toUpperCase();
      if (k === "1" || k === "2" || k === "3") { if (!focusId) setLayout((["clusters", "adaptive", "ontology"] as const)[+k - 1]); return; }
      const hit = keys.current.find((x) => !x.set && x.keys.length === 1 && x.keys[0].toUpperCase() === (e.key === "Escape" ? "ESC" : k));
      const fn = hit && act.current[hit.id];
      if (fn) { if (hit.id === "ask") e.preventDefault(); fn(); }
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, [dlg, focusId]);

  const head = [...COMMITS].reverse().find((c) => dayMs(c.date) <= t);

  const focusNode = focusId ? byId.get(focusId)! : null;

  // a citation opens the node in Details with a way back to the answer; any other selection drops that crumb
  const [fromAnswer, setFromAnswer] = useState(false);
  const citing = useRef(false);
  const cite = (id: string) => {
    citing.current = true;
    setEdge(null); setSymbolId(null); setScopeId(id);
    if (focusId) setFocusId(id);
    setFromAnswer(true);
    showSidebar("details");
  };
  const askAbout = (id: string) => {
    if (id !== scopeId && id !== focusId) setScopeId(id);
    window.dispatchEvent(new Event(ASK_EVENT));
  };
  const context = focusId ?? (scopeId !== WORKSPACE ? scopeId : undefined);

  // ---- context menu (right-click, ContextMenu key, Shift+F10, touch long-press) on nodes, tree rows and links ----
  const openMenu = (target: MenuTarget, at: { x: number; y: number } | null, from: Element | null) => {
    // right-click selects like a left click, but never toggles off and never opens focus view
    if (target.type === "node" && !focusId && target.id !== scopeId) { setEdge(null); setSymbolId(null); setScopeId(target.id); }
    setMenu({ target, at, from });
  };
  const showDetails = (id: string) => {
    setEdge(null); setSymbolId(null); setScopeId(id);
    if (focusId && focusId !== id) setFocusId(id);
    showSidebar("details");
  };
  const inspectLink = (a: string, b: string) => { setSymbolId(null); setEdge({ a, b }); setScopeId(a); showSidebar("details"); };
  const copyPath = async (path: string) => {
    let ok = false;
    try { await navigator.clipboard.writeText(path); ok = true; } catch {
      // clipboard API blocked (no permission, not a secure context): fall back to a hidden textarea
      const ta = Object.assign(document.createElement("textarea"), { value: path, readOnly: true });
      ta.style.cssText = "position:fixed;opacity:0;pointer-events:none";
      document.body.append(ta);
      ta.select();
      try { ok = document.execCommand("copy"); } catch { /* nothing left to try */ }
      ta.remove();
    }
    toast(ok ? `Copied ${path}` : "Couldn't copy the path. It's shown in Details to select by hand.");
  };
  const menuItems = (tg: MenuTarget): { title: string; sub: string; items: MenuItem[] } => {
    if (tg.type === "link") {
      const A = byId.get(tg.a)!, B = byId.get(tg.b)!;
      const bundle = between(links, tg.a, tg.b, t);
      return {
        title: `${A.name} → ${B.name}`,
        sub: bundle.length === 1 ? "1 link" : `${bundle.length} links`,
        items: [
          { label: "Inspect link", icon: ArrowLeftRight, run: () => inspectLink(tg.a, tg.b) },
          { label: "Mark link as wrong", icon: Flag, run: () => {
            if (bundle.length === 1) return setDlg({ link: bundle[0] });
            inspectLink(tg.a, tg.b);
            toast(`This line holds ${bundle.length} links. Pick one in Details, then mark it as wrong.`);
          } },
          { label: `Go to source · ${A.name}`, icon: LogOut, run: () => showDetails(tg.a) },
          { label: `Go to target · ${B.name}`, icon: LogIn, run: () => showDetails(tg.b) },
        ],
      };
    }
    const n = byId.get(tg.id)!;
    const ws = n.kind === "workspace";
    const isPinned = pinned.has(n.id);
    return {
      title: n.name,
      sub: n.kind === "service" ? "Service" : n.kind === "module" ? "Module" : n.kind === "file" ? "File" : "Workspace",
      items: [
        { label: isPinned ? "Unpin" : "Pin", icon: isPinned ? StarOff : Star, hint: "P", key: "p", hidden: ws,
          run: () => toast(togglePin(n.id) ? `Pinned ${n.name}` : `Unpinned ${n.name}`) },
        { label: "Open focus view", icon: Orbit, hidden: ws || focusId === n.id, run: () => open(n.id) },
        { label: "Show details", icon: PanelRight, run: () => showDetails(n.id) },
        { label: "Ask about this", icon: MessageSquare, run: () => askAbout(n.id) },
        { label: "What breaks if this changes", icon: TriangleAlert, hidden: ws, run: () => {
          askAbout(n.id);
          setAskQ(`/impact @${n.name} `);
        } },
        { label: "Trace on the map", icon: Route, hidden: !traceStart(n.id).length, run: () => { location.hash = `trace=${encodeURIComponent(n.id)}`; } },
        { label: "Copy path", icon: Copy, hidden: ws, run: () => void copyPath(n.path) },
      ],
    };
  };
  const menuView = menu ? menuItems(menu.target) : null;
  useEffect(() => {
    if (citing.current) citing.current = false;
    else setFromAnswer(false);
  }, [scopeId, focusId, edge, symbolId]);
  // the Filters popover closes on an outside click or Esc
  useEffect(() => {
    const on = (e: PointerEvent | KeyboardEvent) => {
      const d = filtersRef.current;
      if (!d?.open) return;
      if (e instanceof KeyboardEvent ? e.key === "Escape" : !d.contains(e.target as Node)) {
        d.open = false;
        if (e instanceof KeyboardEvent) d.querySelector("summary")?.focus();
      }
    };
    window.addEventListener("pointerdown", on);
    window.addEventListener("keydown", on);
    return () => { window.removeEventListener("pointerdown", on); window.removeEventListener("keydown", on); };
  }, []);

  const treeKey = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const el = e.target as HTMLElement;
    if (!el.matches(".row-main")) return;
    const id = el.dataset.id!;
    if (isMenuKey(e)) { e.preventDefault(); return openMenu({ type: "node", id }, null, el); }
    const hasKids = el.hasAttribute("aria-expanded"), isOpen = el.getAttribute("aria-expanded") === "true";
    const toggle = () => setExpanded((x) => { const n = new Set(x); n.has(id) ? n.delete(id) : n.add(id); return n; });
    if (e.key === "ArrowRight" && hasKids) { e.preventDefault(); return isOpen ? treeRove.move(el, 1) : toggle(); }
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      if (hasKids && isOpen) return toggle();
      const parent = byId.get(id)?.parent;
      const pe = parent ? treeRef.current?.querySelector(`.row-main[data-id="${CSS.escape(parent)}"]`) : null;
      if (pe) treeRove.move(pe, 0);
      return;
    }
    treeRove.onKey(e, { ArrowDown: 1, ArrowUp: -1 });
  };

  return (
    <div className="app">
      <a className="skip-link" href="#map">Skip to map</a>
      <AppShell onPick={(id) => open(id)} extra={
        <details className="pins">
          <summary aria-label={`${pinned.size} pinned`}><Star size={15} fill="var(--accent)" stroke="var(--accent)" aria-hidden /> {pinned.size} pinned</summary>
          <div className="pins-menu">
            {pinned.size ? [...pinned].filter((id) => byId.has(id)).map((id) => (
              <div key={id} className="map-pin-row">
                <button type="button" title={byId.get(id)!.path} onClick={(e) => { open(id); (e.currentTarget.closest("details") as HTMLDetailsElement).open = false; }}>
                  <i style={{ ["--c" as string]: serviceColor(byId.get(id)!.service) }} aria-hidden />{byId.get(id)!.name}
                </button>
                <button type="button" className="map-unpin" aria-label={`Unpin ${byId.get(id)!.name}`} title="Unpin"
                  onClick={() => { togglePin(id); toast(`Unpinned ${byId.get(id)!.name}`); }}>
                  <X size={14} aria-hidden />
                </button>
              </div>
            )) : <p>Right-click a node and choose Pin, or select it and press P.</p>}
          </div>
        </details>
      } />
      <AddedHint />

      <main className="stage">
        <nav className={`rail ${railOpen ? "is-open" : "is-collapsed"}`} aria-label="Workspace tree">
          <div className="rail-head">
            {railOpen && (
              <label className="map-filter">
                <ListFilter size={14} aria-hidden />
                <input value={treeQ} onChange={(e) => setTreeQ(e.target.value)} placeholder={`Filter ${SERVICES.length} services`} aria-label={`Filter ${SERVICES.length} services`} />
              </label>
            )}
            <button type="button" className="rail-toggle" onClick={toggleRail} aria-expanded={railOpen} aria-keyshortcuts="["
              aria-label={railOpen ? "Collapse the tree" : "Expand the tree"} title={railOpen ? "Collapse the tree ([)" : "Expand the tree ([)"}>
              {railOpen ? <ChevronsLeft size={16} aria-hidden /> : <ChevronsRight size={16} aria-hidden />}
            </button>
          </div>
          {railOpen && (
            <div className="rail-tree" role="tree" aria-label="Services" ref={treeRef} onKeyDown={treeKey} onFocus={treeRove.onFocus}
              onContextMenu={(e) => {
                const el = (e.target as Element).closest<HTMLElement>(".row-main");
                if (el) onMenuEvent(e, (at) => openMenu({ type: "node", id: el.dataset.id! }, at, el));
              }}
              onPointerDown={(e) => {
                const el = (e.target as Element).closest<HTMLElement>(".row-main");
                if (el) longPress(e, (at) => openMenu({ type: "node", id: el.dataset.id! }, at, el));
              }}>
              {SERVICES.map((s) => (
                <TreeRow key={s.id} id={s.id} depth={0} t={t} scopeId={scopeId} expanded={expanded} pinned={pinned} q={treeQ.trim().toLowerCase()}
                  onToggle={(id) => { const n = new Set(expanded); n.has(id) ? n.delete(id) : n.add(id); setExpanded(n); }}
                  onPick={select} />
              ))}
            </div>
          )}
        </nav>

        <section className="canvas" id="map" tabIndex={-1} aria-label="Map">
          <div className="canvas-bar">
            {focusNode ? (
              <div className="focus-title">
                <button className="round-btn is-primary" onClick={back} aria-label="Back to map"><ChevronLeft size={18} /></button>
                <h1>{symbolId ? `${symbolById(symbolId)!.name}${symbolById(symbolId)!.kind === "function" ? "()" : ""}` : focusNode.name}</h1>
                <code>{focusNode.path}</code>
                {canSymbols && (
                  <label className="field depth">
                    <span>Detail</span>
                    <select value={symbolId ? "symbols" : "files"} onChange={(e) => (e.target.value === "symbols" ? showSymbol(symbolsIn(focusNode.id)[0].id) : setSymbolId(null))}>
                      <option value="files">Files</option>
                      <option value="symbols">Symbols</option>
                    </select>
                  </label>
                )}
              </div>
            ) : (
              <div className="bar-left">
                <div className="seg" role="tablist" aria-label="Layout" onKeyDown={(e) => {
                  const L = ["clusters", "adaptive", "ontology"] as const;
                  const by = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
                  if (!by) return;
                  e.preventDefault();
                  const i = (L.indexOf(layout) + by + 3) % 3;
                  setLayout(L[i]);
                  (e.currentTarget.children[i] as HTMLElement).focus();
                }}>
                  {(["clusters", "adaptive", "ontology"] as const).map((v) => (
                    <button key={v} role="tab" aria-selected={layout === v} tabIndex={layout === v ? 0 : -1} onClick={() => setLayout(v)}
                      title={v === "adaptive" ? "Links act like springs. Drag nodes to rearrange. Adaptive also sets the detail level from zoom." : undefined}>
                      {v === "clusters" ? "Network" : v === "adaptive" ? "Adaptive" : "Ontology"}
                    </button>
                  ))}
                </div>
                <label className="field depth">
                  <span>Detail</span>
                  <select value={depthSetting} onChange={(e) => setDepthSetting(e.target.value === "auto" ? "auto" : (+e.target.value as Depth))}>
                    <option value="auto">Auto · {["", "services", "modules", "files"][autoDepth]}</option>
                    <option value={1}>Services</option>
                    <option value={2}>Modules</option>
                    <option value={3}>Files</option>
                  </select>
                </label>
              </div>
            )}
            <details className={`map-filters ${filtersOn ? "is-on" : ""}`} ref={filtersRef}>
              <summary><SlidersHorizontal size={15} aria-hidden />Filters{filtersOn > 0 && <b>{filtersOn}</b>}</summary>
              <div className="map-filters-pop">
                <label className={`field ${minUsedBy > 0 ? "on" : ""}`}>
                  <span>Used by at least</span>
                  <span className="range-row">
                    <b>{minUsedBy}</b>
                    <input type="range" min={0} max={5} value={minUsedBy} onChange={(e) => setMinUsedBy(+e.target.value)} aria-label="Hide nodes used by fewer than this many others" />
                  </span>
                </label>
                <label className={`field ${typeFilter !== "all" ? "on" : ""}`}>
                  <span>Link kind</span>
                  <select value={typeFilter} onChange={(e) => setTypeFilter(e.target.value as LinkType | "all")}>
                    <option value="all">All</option>
                    <option value="imports">Imports</option>
                    <option value="calls">Calls</option>
                    <option value="http">HTTP</option>
                    <option value="event">Events</option>
                    <option value="db">Database</option>
                  </select>
                </label>
                {filtersOn > 0 && <button type="button" className="map-reset" onClick={resetFilters}>Reset {filtersOn} {filtersOn === 1 ? "filter" : "filters"}</button>}
              </div>
            </details>
            <p className="map-line">
              {head && (
                <span className="map-line-head" title={`${head.sha} ${head.message}`}>
                  <span>HEAD at this date</span> <code>{head.sha}</code> <span className="map-line-msg">{head.message}</span>
                </span>
              )}
              <span className="map-line-zoom" ref={setStatusSlot} />
              <span className="map-line-hint"><span className="is-mouse">Right-click a node for more</span><span className="is-touch">Long-press a node for more</span></span>
            </p>
          </div>

          <div className="map-stage">
            <Graph focus={!!focusId} layout={layout} depthSetting={depthSetting} t={t} focusId={focusId}
              scopeId={scopeId === WORKSPACE ? null : scopeId} highlight={highlight}
              typeFilter={typeFilter} minUsedBy={minUsedBy} onOpen={open} onSelect={select} onDepth={onDepth}
              links={eff.links} showWeak={showWeak} compare={compare} edgeSel={edge} symbolId={symbolId}
              onEdge={(e) => { setEdge(e); setScopeId(e.a); }} onSymbol={showSymbol} statusSlot={statusSlot}
              pinned={pinned} onMenu={openMenu} />

            {mapEmpty && (
              <div className="map-empty">
                <EmptyState icon={SearchX} title="No nodes match these filters"
                  body={`No node is used by ${minUsedBy || "any"}${minUsedBy ? " or more others" : " others"}${typeFilter !== "all" ? ` over ${typeFilter} links` : ""} on ${fmtDay(hi)}.`}
                  action={{ label: `Reset ${filtersOn} ${filtersOn === 1 ? "filter" : "filters"}`, onClick: resetFilters }} />
                <div className="map-fixes">
                  {lowerTo !== null && minUsedBy > 0 && (
                    <div className="map-fix"><span>Lower <b>Used by at least</b> to {lowerTo} <span>· {nodeCount(lowerTo, typeFilter)} nodes</span></span>
                      <button aria-label={`Set Used by at least to ${lowerTo}`} onClick={() => setMinUsedBy(lowerTo)}>Set to {lowerTo}</button></div>
                  )}
                  {typeFilter !== "all" && (
                    <div className="map-fix"><span>Show any <b>link kind</b> <span>· {nodeCount(minUsedBy, "all")} nodes</span></span>
                      <button aria-label="Show all link kinds" onClick={() => setTypeFilter("all")}>Show all kinds</button></div>
                  )}
                </div>
              </div>
            )}

            <div className="map-status">
              {backRoute && (
                <Link className="map-pill is-back" href={backRoute}><ChevronLeft size={14} aria-hidden />Back to {backLabel(backRoute)}</Link>
              )}
              {trace && <p className="map-pill is-trace">Tracing from {traceName} <button onClick={clearTrace}>Clear trace</button></p>}
              {compare && <p className="map-pill">Comparing {fmtDay(Math.min(compare.a, compare.b))} with {fmtDay(Math.max(compare.a, compare.b))} <button onClick={() => setCompare(null)}>Exit compare</button></p>}
              {filtersOn > 0 && !focusId && <p className="map-pill" role="status">{shownCount} of {total} nodes shown <button onClick={resetFilters}>Reset filters</button></p>}
            </div>
            <p className="map-sr" role="status">{saved}</p>

            {!focusId && <Legend compare={!!compare} weak={showWeak} weakCount={weakCount} onWeak={setShowWeak} />}
          </div>
        </section>

        <Sidebar askUnread={unread}
          details={
            <Panel scopeId={focusId ?? scopeId} t={t} pinned={pinned} focused={!!focusId} onTogglePin={togglePin} onOpen={select} onFocus={open}
              links={links} corrections={corrections} edge={edge} symbolId={symbolId} onEdge={setEdge} onCorrect={onCorrect}
              onUndo={(id) => { setCorrections((c) => c.filter((x) => x.id !== id)); setSaved("Correction put back."); }}
              onSymbol={(id) => (id ? showSymbol(id) : setSymbolId(null))} onJump={setT}
              onAskAbout={askAbout} onBackToAnswer={fromAnswer ? () => window.dispatchEvent(new Event(ASK_EVENT)) : undefined} />
          }
          ask={
            <AskDocked t={t} answer={answer} onAnswer={showAnswer} onOpen={select}
              onCitation={cite} context={context} initialQuestion={askQ} />
          } />
      </main>

      {dlg && <CorrectionDialog link={dlg.link} from={dlg.from} onClose={() => setDlg(null)}
        onSave={(c) => { setCorrections((x) => [c, ...x]); setEdge(null); setSaved("Correction saved. The link shows an Overridden marker on the map."); }} />}
      <AskFloat onMap t={t} answer={answer} onAnswer={showAnswer} onOpen={select}
        onCitation={cite} context={context} initialQuestion={askQ} />
      {menu && menuView && (
        <NodeMenu key={JSON.stringify(menu.target) + (menu.at ? `${menu.at.x},${menu.at.y}` : "")} req={menu}
          title={menuView.title} sub={menuView.sub} items={menuView.items} onClose={() => setMenu(null)} />
      )}
      <Timeline t={t} today={TODAY} onChange={setT} compare={compare} onCompare={setCompare} />
    </div>
  );
}

const matchesQ = (id: string, q: string): boolean => !q || byId.get(id)!.name.toLowerCase().includes(q) || children(id).some((k) => matchesQ(k.id, q));

function TreeRow({ id, depth, t, scopeId, expanded, pinned, q, onToggle, onPick }: {
  id: string; depth: number; t: number; scopeId: string; expanded: Set<string>; pinned: Set<string>; q: string;
  onToggle: (id: string) => void; onPick: (id: string) => void;
}) {
  const n = byId.get(id)!;
  const kids = n.kind === "file" ? [] : children(id);
  const m = useMemo(() => metrics(id, t), [id, t]);
  const was = useMemo(() => metrics(id, t - 90 * DAY), [id, t]);
  const isOpen = expanded.has(id) || (!!q && kids.some((k) => matchesQ(k.id, q)));
  if (!matchesQ(id, q)) return null;
  return (
    <div className={`tree d${depth}`} role="none">
      <div className={`row ${scopeId === id ? "is-selected" : ""} ${pinned.has(id) ? "is-pinned" : ""}`} style={{ ["--c" as string]: serviceColor(n.service) }}>
        {kids.length ? (
          <button className="twist" tabIndex={-1} aria-hidden aria-label={`${isOpen ? "Collapse" : "Expand"} ${n.name}`} onClick={() => onToggle(id)}>
            <ChevronRight size={14} />
          </button>
        ) : <span className="twist" />}
        <button className="row-main" role="treeitem" tabIndex={-1} data-id={id} aria-level={depth + 1}
          aria-expanded={kids.length ? isOpen : undefined} aria-selected={scopeId === id} aria-haspopup="menu"
          onClick={() => onPick(id)}>
          <i className={n.kind === "file" ? "ring" : "dot"} aria-hidden />
          <span className="row-name">{n.name}</span>
          {pinned.has(id) && <><Star className="map-pinmark" size={12} aria-hidden /><span className="map-sr">, pinned</span></>}
          <span className="num" title="Active links">{m.links}</span>
          <Delta v={m.links - was.links} />
        </button>
      </div>
      {isOpen && kids.map((k) => (
        <TreeRow key={k.id} id={k.id} depth={depth + 1} t={t} scopeId={scopeId} expanded={expanded} pinned={pinned} q={q} onToggle={onToggle} onPick={onPick} />
      ))}
    </div>
  );
}
