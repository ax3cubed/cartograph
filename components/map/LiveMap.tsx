"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode, type WheelEvent as ReactWheel } from "react";
import Link from "next/link";
import dynamic from "next/dynamic";
import { ChevronLeft, MonitorOff, X } from "lucide-react";
import type { GraphSlice, Reach } from "@/lib/api";
import { get } from "@/lib/api/client";
import { useCommits, useGraph, useMode, useStructure, useWorkspaces, wsRoute } from "@/lib/api/live";
import { endOfLocalDay, localDay } from "@/lib/api/at";
import { fmtDate, fmtNum } from "@/lib/date";
import AppShell, { ASK_EVENT, VIEW_TABS } from "../shell/AppShell";
import AddedHint from "../shell/AddedHint";
import { askStore, focusComposer } from "../ask/store";
import EmptyState from "../shell/EmptyState";
import Timeline, { DAY, type Tick } from "../timeline/Timeline";
import { nearest } from "../timeline/bins";
import TableView from "./TableView";
import CorrectionList from "./CorrectionList";
import Details from "./Details";
import { compare, hasLayout, layout as levelLayout, owners, placed, rollUp, type Compared, type DiffState, type Level } from "./levels";
import { degrees, linked, sayFile, sayFolder, shown, topFolder } from "./table";
import type { GLLevel } from "./WebGLMap";
import "./liveMap.css";

// sigma and graphology load with the WebGL map, not with every page
const WebGLMap = dynamic(() => import("./WebGLMap"), { ssr: false });

/** The map route on the index. Without the binary (a static host, `pnpm dev`) there is no index to draw. */
export default function MapRoute() {
  const mode = useMode();
  if (mode === "live") return <LiveMap />;
  if (mode === "checking") return null;
  return (
    <div className="sh-frame"><AppShell /><div className="sh-body">
      <EmptyState icon={MonitorOff} title="The map draws your index" body="Nothing answered at this address, so there is no index to draw. Open the app from the CLI with cartograph open." />
    </div></div>
  );
}

/** I1: the SVG path draws at most this many nodes plus links; above it the WebGL map (FE-08) takes over. */
const MAX_DRAWN = 2000;
const GOLDEN = Math.PI * (3 - Math.sqrt(5));
const STEP = 9; // spacing between files in a folder

type Pt = { x: number; y: number };
const hash = (s: string) => { let h = 2166136261; for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619); return h >>> 0; };
const n = (k: number, what: string) => `${fmtNum(k)} ${what}${k === 1 ? "" : "s"}`;
const hue = (i: number) => (i < 6 ? `var(--hue-${i + 1})` : "var(--steel)");
const token = (i: number) => (i >= 0 && i < 6 ? `--hue-${i + 1}` : "--steel");
const NOUN: Record<Level, { one: string; many: string }> = { service: { one: "service", many: "services" }, module: { one: "module", many: "modules" }, file: { one: "file", many: "files" } };

/** Stored positions (I2) are in the indexer's units, where a file's radius is 1; the SVG map spaces files wider. */
const STORED_SCALE = 6;

/**
 * Positions for the SVG map, from the workspace now so they hold still while the date moves (I2). A file sits where
 * the index placed it, and each top folder is a disc around its files. On an index with no layout rows yet, each top
 * folder is a disc of files (a sunflower, ordered by a hash of the id so a file keeps its rank as others come and
 * go), and the discs are packed on shelves, largest first. A file that isn't there now gets a seat by its hash.
 */
function svgLayout(s: GraphSlice) {
  const count = s.nodes.id.length;
  const folder = s.nodes.name.map(topFolder);
  const groups = [...new Set(folder)].sort();
  const pos = new Map<string, Pt>();
  const stored = hasLayout(s);
  const centres = new Map<string, Pt & { r: number }>();
  const members = new Map<string, number[]>(groups.map((g) => [g, []]));
  folder.forEach((g, i) => members.get(g)!.push(i));
  const bySize = [...groups].sort((a, b) => members.get(b)!.length - members.get(a)!.length || a.localeCompare(b));
  const radius = (g: string) => STEP * Math.sqrt(members.get(g)!.length + 1) + 24;
  const area = bySize.reduce((a, g) => a + (2 * radius(g)) ** 2, 0);
  const width = bySize.length ? Math.max(2 * radius(bySize[0]), Math.sqrt(area) * 1.3) : 0;
  let x = 0, y = 0, row = 0;
  const at0 = (i: number): Pt => ({ x: s.nodes.x[i] * STORED_SCALE, y: s.nodes.y[i] * STORED_SCALE });
  for (const g of bySize) {
    let c: Pt & { r: number };
    if (stored) {
      // the disc around the folder's placed files
      const ps = members.get(g)!.filter((i) => placed(s, i)).map(at0);
      const cx = ps.length ? ps.reduce((a, p) => a + p.x, 0) / ps.length : 0, cy = ps.length ? ps.reduce((a, p) => a + p.y, 0) / ps.length : 0;
      c = { x: cx, y: cy, r: Math.max(0, ...ps.map((p) => Math.hypot(p.x - cx, p.y - cy))) + 24 };
    } else {
      const r = radius(g);
      if (x > 0 && x + 2 * r > width) { x = 0; y += row; row = 0; }
      c = { x: x + r, y: y + r, r };
      x += 2 * r; row = Math.max(row, 2 * r);
    }
    centres.set(g, c);
    members.get(g)!.sort((a, b) => hash(s.nodes.id[a]) - hash(s.nodes.id[b])).forEach((i, k) => {
      const rr = STEP * Math.sqrt(k + 0.5), a = k * GOLDEN;
      pos.set(s.nodes.id[i], stored && placed(s, i) ? at0(i) : { x: c.x + rr * Math.cos(a), y: c.y + rr * Math.sin(a) });
    });
  }
  const all = [...pos.values()];
  const pad = 40;
  const xs = all.map((p) => p.x), ys = all.map((p) => p.y);
  const box = count ? { x: Math.min(...xs) - pad, y: Math.min(...ys) - pad, w: Math.max(...xs) - Math.min(...xs) + 2 * pad, h: Math.max(...ys) - Math.min(...ys) + 2 * pad } : { x: 0, y: 0, w: 100, h: 100 };
  const at = (id: string, name: string): Pt => {
    const p = pos.get(id);
    if (p) return p;
    const c = centres.get(topFolder(name)) ?? { x: box.x + box.w / 2, y: box.y + box.h / 2, r: 24 };
    const k = hash(id) % 64, rr = Math.min(c.r * 0.9, STEP * Math.sqrt(k + 0.5)), a = k * GOLDEN;
    return { x: c.x + rr * Math.cos(a), y: c.y + rr * Math.sin(a) };
  };
  return { at, groups, centres, box };
}

/** Hash state the views link with: #focus=<id>, #trace=<id>, #commit=<sha>, each with an optional &back=<route>. */
const readHash = () => new URLSearchParams(typeof location === "undefined" ? "" : location.hash.slice(1));
const backLabel = (route: string) => VIEW_TABS.find((t) => t.href === route)?.label ?? route.split("/").filter(Boolean).pop() ?? "map";

function LiveMap() {
  const { current, loading: wsLoading } = useWorkspaces();
  const ws = current?.id;
  const [day, setDay] = useState(""); // the date field: "" = now; else a local YYYY-MM-DD
  const [knob, setKnob] = useState<string | null>(null); // the timeline's commit, as it moves
  const [sha, setSha] = useState(""); // the commit the map is drawn at, a moment after the knob rests
  const at = sha || (day ? endOfLocalDay(day) : "");
  const [mode, setMode] = useState<"map" | "table">("map");
  const [filter, setFilter] = useState("");
  const [cmp, setCmp] = useState<{ a: number; b: number } | null>(null);
  const [side, setSide] = useState(false);
  const [glLevel, setGlLevel] = useState<Level>("service");
  const [lost, setLost] = useState(false);
  const [forceGL, setForceGL] = useState(false);
  const [back, setBack] = useState<string | null>(null);
  const [trace, setTrace] = useState<{ id: string; nodes: Set<string>; links: Set<string> } | null>(null);
  const [focusTick, setFocusTick] = useState(0);

  // a workspace past the SVG map's size starts fetching the WebGL map before its data arrives
  useEffect(() => { if ((current?.files ?? 0) > MAX_DRAWN / 2) void import("./WebGLMap").then((m) => m.loadLibs()); }, [current?.files]);
  const g = useGraph(ws, at);
  const s = g.data;
  const nowG = useGraph(ws, "");
  const structure = useStructure(ws).data;
  // the map's data is the slice and the structure it rolls up into (FE-08's gate is timed from here)
  useEffect(() => { if (s && structure && !performance.getEntriesByName("cartograph:map-data").length) performance.mark("cartograph:map-data"); }, [s, structure]);
  const commitList = useCommits(ws).data;
  const commits: Tick[] = useMemo(() => (commitList ?? []).map((c) => ({ sha: c.sha, ms: Date.parse(c.date), repo: c.repo, message: c.message })), [commitList]);
  const bySha = useMemo(() => new Map(commits.map((c) => [c.sha, c])), [commits]);
  const days = useMemo(() => Int32Array.from(commits, (c) => Math.floor(c.ms / DAY)), [commits]);
  const dateOf = useCallback((h: string) => { const c = bySha.get(h); return c ? fmtDate(c.ms, "dmy") : undefined; }, [bySha]);
  const [today] = useState(() => Date.now());

  // the knob moves at once; the map follows when it rests, so dragging never waits on a slice
  useEffect(() => {
    if (knob === null) return;
    const t = setTimeout(() => setSha(knob), 120);
    return () => clearTimeout(t);
  }, [knob]);
  const pickCommit = (h: string | null) => {
    setKnob(h ?? "");
    const c = h ? bySha.get(h) : undefined;
    setDay(c ? localDay(new Date(c.ms)) : "");
  };
  const t = knob ? bySha.get(knob)?.ms ?? today : knob === "" ? today : day ? Date.parse(endOfLocalDay(day)) : today;
  /** The commit at or before `ms`, as `at=`; past the last commit, now. */
  const atOf = (ms: number) => {
    const i = nearest(days, ms);
    if (i < 0 || ms >= today) return "";
    return commits[i].ms > ms + DAY ? (i > 0 ? commits[i - 1].sha : "") : commits[i].sha;
  };
  const bAt = cmp ? atOf(cmp.b) : null;
  const gB = useGraph(cmp ? ws : undefined, bAt ?? "");
  const sB = cmp ? gB.data : undefined;
  // compare runs earlier against later, whichever handle is which
  const bEarlier = cmp ? cmp.b < cmp.a : false;

  // the hash: #focus, #trace, #commit, #back, #renderer=webgl
  const pendingFocus = useRef<string | null>(null);
  useEffect(() => {
    const read = () => {
      const p = readHash();
      if (p.get("renderer") === "webgl") setForceGL(true);
      setBack(p.get("back"));
      const c = p.get("commit");
      if (c) { setSha(c); setKnob(c); }
      const f = p.get("focus") ?? p.get("trace");
      if (f) pendingFocus.current = f;
      if (p.get("trace")) void traceFrom(p.get("trace")!);
      setFocusTick((k) => k + 1);
    };
    // Ask on the map (#ask, #ask=<question>, the header's Ask): the floating chat
    const ask = () => { askStore.setMode("floating"); askStore.setFloat("open"); focusComposer(); };
    const askHash = () => { if (readHash().has("ask")) ask(); };
    read(); askHash();
    window.addEventListener("hashchange", read);
    window.addEventListener("hashchange", askHash);
    window.addEventListener(ASK_EVENT, ask);
    return () => { window.removeEventListener("hashchange", read); window.removeEventListener("hashchange", askHash); window.removeEventListener(ASK_EVENT, ask); };
    // traceFrom reads the workspace when it runs
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ---------- what is drawn ----------
  const size = s ? s.nodes.id.length + s.links.from.length : 0;
  // a workspace with more files than the SVG map draws is WebGL before its slice arrives, so sigma starts at once
  const gl = forceGL || size > MAX_DRAWN || (!s && (current?.files ?? 0) > MAX_DRAWN);
  const nowSlice = nowG.data;
  // an index with stored positions (I2) serves the service and module levels too; one without has them rolled up here
  const served = hasLayout(nowSlice);
  const lvWs = gl && served ? ws : undefined;
  const svcA = useGraph(lvWs, at, "service"), modA = useGraph(lvWs, at, "module");
  const svcB = useGraph(cmp ? lvWs : undefined, bAt ?? "", "service"), modB = useGraph(cmp ? lvWs : undefined, bAt ?? "", "module");
  const svcNow = useGraph(lvWs, "", "service"), modNow = useGraph(lvWs, "", "module");
  const L = useMemo(() => (!gl && nowSlice ? svgLayout(nowSlice) : null), [gl, nowSlice]);
  const cA = useMemo<Compared | null>(() => (cmp && s && sB ? (bEarlier ? compare(sB, s) : compare(s, sB)) : null), [cmp, s, sB, bEarlier]);
  /** The file-level slice the map, the table and the walk read: the date's, or both dates' union in compare. */
  const fileSlice = cA?.slice ?? s;
  const vis = useMemo(() => (fileSlice ? shown(fileSlice, filter) : { nodes: [], links: [] }), [fileSlice, filter]);
  const deg = useMemo(() => (fileSlice ? degrees(fileSlice) : { inn: [], out: [] }), [fileSlice]);

  // ---------- the WebGL map's levels (FE-08) ----------
  const nowServed = useMemo(() => (served && !svcNow.error && !modNow.error ? { service: svcNow.data, module: modNow.data } : undefined),
    [served, svcNow.data, svcNow.error, modNow.data, modNow.error]);
  const glL = useMemo(() => {
    if (!gl || !nowSlice || !structure) return null;
    if (nowServed && (!nowServed.service || !nowServed.module)) return null; // the index's discs are on their way
    return levelLayout(nowSlice, structure, undefined, nowServed);
  }, [gl, nowSlice, structure, nowServed]);
  const svcIndex = useMemo(() => new Map((structure?.services ?? []).map((x, k) => [x.id, k])), [structure]);
  const holderSvc = useMemo(() => new Map([...(structure?.services ?? []).map((x) => [x.id, x.id] as const), ...(structure?.modules ?? []).map((m) => [m.id, m.service] as const)]), [structure]);
  const glLevels = useMemo(() => {
    if (!gl || !structure || !s) return null;
    type R = { data?: GraphSlice; error?: unknown };
    // the index's levels when it serves them; rolled up here when it has no layout or the read failed
    const one = (x: GraphSlice, sv: R, md: R) => {
      const own = owners(x, structure);
      if (served && sv.data && md.data && !sv.error && !md.error) return { own, service: sv.data, module: md.data };
      return { own, service: rollUp(x, structure, "service", own), module: rollUp(x, structure, "module", own) };
    };
    const waiting = (r: R) => served && !r.data && !r.error;
    if (waiting(svcA) || waiting(modA) || (sB && (waiting(svcB) || waiting(modB)))) return null;
    const A = one(s, svcA, modA), B = sB ? one(sB, svcB, modB) : null;
    const pair = (lv: "service" | "module" | "file") => {
      const a = lv === "file" ? s : A[lv];
      if (!B || !sB) return { slice: a };
      const b = lv === "file" ? sB : B[lv];
      const c = bEarlier ? compare(b, a) : compare(a, b);
      return { slice: c.slice, diff: { node: c.node, link: c.link } };
    };
    const file = pair("file");
    const fOwn = owners(file.slice, structure);
    const svcOf = (k: number) => (k >= 0 ? structure.services[k].id : "");
    const out: Record<Level, GLLevel> = {
      service: { ...pair("service"), service: () => "" },
      module: { ...pair("module"), service: () => "" },
      file: { ...file, service: (i) => svcOf(fOwn.service[i]), holder: (i) => (fOwn.module[i] >= 0 ? structure.modules[fOwn.module[i]].id : svcOf(fOwn.service[i])) },
    };
    out.service.service = (i) => out.service.slice.nodes.id[i];
    out.module.service = (i) => holderSvc.get(out.module.slice.nodes.id[i]) ?? "";
    return out;
  }, [gl, structure, s, sB, bEarlier, holderSvc, served, svcA.data, svcA.error, modA.data, modA.error, svcB.data, svcB.error, modB.data, modB.error]); // eslint-disable-line react-hooks/exhaustive-deps
  const hueOf = useCallback((svc: string) => token(svcIndex.get(svc) ?? -1), [svcIndex]);
  // FE-12: the WebGL map draws what the table lists, under the same filter
  const glDrawn = useMemo(() => {
    const f = filter.trim().toLowerCase();
    if (!glLevels || !f) return glLevels;
    const out = {} as Record<Level, GLLevel>;
    for (const lv of ["service", "module", "file"] as Level[]) out[lv] = { ...glLevels[lv], hide: glLevels[lv].slice.nodes.name.map((x) => !x.toLowerCase().includes(f)) };
    return out;
  }, [glLevels, filter]);
  const levelSlice: GraphSlice | undefined = gl ? glLevels?.[glLevel].slice : fileSlice;
  const levelDiff = gl ? glLevels?.[glLevel].diff : cA ? { node: cA.node, link: cA.link } : undefined;
  const levelVis = useMemo(() => (gl && levelSlice ? shown(levelSlice, filter) : vis), [gl, levelSlice, filter, vis]);
  const levelDeg = useMemo(() => (gl && levelSlice ? degrees(levelSlice) : deg), [gl, levelSlice, deg]);

  // ---------- selection: a node id at a level, kept across dates ----------
  const [sel, setSelKey] = useState<{ level: Level; id: string } | null>(null);
  const panel = useRef<HTMLElement>(null);
  const back$ = useRef<HTMLElement | null>(null); // where Escape in the details panel returns focus
  const selLevel: Level = gl ? sel?.level ?? glLevel : "file";
  const selSlice = gl ? glLevels?.[selLevel].slice : fileSlice;
  const selIdx = sel && selSlice ? selSlice.nodes.id.indexOf(sel.id) : -1;
  const selI = selIdx >= 0 ? selIdx : null;
  const select = (i: number | null, level: Level = gl ? glLevel : "file") => {
    const sl = gl ? glLevels?.[level].slice : fileSlice;
    setSelKey(i === null || !sl ? null : { level, id: sl.nodes.id[i] });
  };
  const selectId = (id: string) => {
    if (!s) return;
    const lv: Level = holderSvc.has(id) ? (structure?.services.some((x) => x.id === id) ? "service" : "module") : "file";
    if (gl) { setSelKey({ level: lv, id }); setFocusTick((k) => k + 1); return; }
    if (lv === "file") { setSelKey({ level: "file", id }); return; }
    // the SVG map draws files: a service or module shows as its files, filtered by its path
    const h = [...(structure?.services ?? []), ...(structure?.modules ?? [])].find((x) => x.id === id);
    if (h) setFilter(h.path);
  };
  // a #focus= waits for the data it names
  useEffect(() => {
    const f = pendingFocus.current;
    if (!f || !s || !structure) return;
    pendingFocus.current = null;
    selectId(f);
    // selectId reads the current slice and structure
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [s, structure, focusTick]);
  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      const inPanel = panel.current?.contains(document.activeElement);
      setSelKey(null);
      if (inPanel) back$.current?.focus();
    };
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  });
  const open = (i: number, from: HTMLElement) => { select(i); back$.current = from; requestAnimationFrame(() => panel.current?.focus()); };

  async function traceFrom(id: string) {
    const w = ws ?? current?.id;
    if (!w) { pendingFocus.current = id; return; }
    try {
      const r = await get<Reach>(wsRoute(w, "reach")!, { node: id, direction: "out", at: at || "now" });
      setTrace({ id, nodes: new Set(r.data.nodes), links: new Set(r.data.links) });
    } catch { setTrace(null); }
  }
  const traceSet = useMemo(() => {
    if (!trace) return null;
    if (!gl || glLevel === "file" || !structure || !s) return trace.nodes;
    // a traced file lights the service or module that holds it
    const own = owners(s, structure), out = new Set<string>();
    s.nodes.id.forEach((id, i) => {
      if (!trace.nodes.has(id)) return;
      const k = glLevel === "module" && own.module[i] >= 0 ? structure.modules[own.module[i]].id : own.service[i] >= 0 ? structure.services[own.service[i]].id : "";
      if (k) out.add(k);
    });
    return out;
  }, [trace, gl, glLevel, structure, s]);

  const walk = useWalk(fileSlice, vis, deg, !gl && sel ? selI : null, (i) => select(i, "file"));
  /** Side by side: the second pane takes the first one's camera, so both move as one (I4). */
  const camHolder = useMemo(() => ({} as { cam?: unknown }), [side, cmp === null]); // eslint-disable-line react-hooks/exhaustive-deps

  // ---------- the SVG camera ----------
  const [view, setView] = useState<{ x: number; y: number; w: number; h: number } | null>(null);
  const drag = useRef<{ x: number; y: number } | null>(null);
  useEffect(() => { setView(null); }, [L?.box.w, L?.box.h]);
  const box = view ?? L?.box;
  const u = L ? Math.max(L.box.w, L.box.h) / 900 : 1; // one screen pixel, roughly, at the fitted zoom
  const onWheel = (e: ReactWheel<SVGSVGElement>) => {
    if (!box) return;
    const r = e.currentTarget.getBoundingClientRect();
    const k = Math.exp(e.deltaY * 0.0015);
    const px = box.x + ((e.clientX - r.left) / r.width) * box.w, py = box.y + ((e.clientY - r.top) / r.height) * box.h;
    setView({ x: px - (px - box.x) * k, y: py - (py - box.y) * k, w: box.w * k, h: box.h * k });
  };
  const onMove = (e: React.PointerEvent<SVGSVGElement>) => {
    if (!drag.current || !box) return;
    const r = e.currentTarget.getBoundingClientRect();
    const dx = ((e.clientX - drag.current.x) / r.width) * box.w, dy = ((e.clientY - drag.current.y) / r.height) * box.h;
    drag.current = { x: e.clientX, y: e.clientY };
    setView({ ...box, x: box.x - dx, y: box.y - dy });
  };
  const nameOf = useCallback((id: string) => {
    const i = fileSlice?.nodes.id.indexOf(id) ?? -1;
    if (i >= 0) return fileSlice!.nodes.name[i];
    return [...(structure?.services ?? []), ...(structure?.modules ?? [])].find((x) => x.id === id)?.name ?? id;
  }, [fileSlice, structure]);

  const svgPane = (sl: GraphSlice, diff: Compared | null, only: "a" | "b" | null, label: string) => {
    if (!L || !box) return null;
    const keep = (i: number) => !only || !diff || (only === "a" ? diff.node[i] !== "added" : diff.node[i] !== "removed");
    const keepL = (j: number) => !only || !diff || (only === "a" ? diff.link[j] !== "added" : diff.link[j] !== "removed");
    const st = (k: DiffState | undefined) => (k && k !== "unchanged" ? ` is-${k}` : k ? " is-same" : "");
    const nodes = vis.nodes.filter(keep);
    const folders = new Set(nodes.map((i) => topFolder(sl.nodes.name[i])));
    const lit = (id: string) => !trace || trace.nodes.has(id);
    return (
      <svg className="lm-svg" viewBox={`${box.x} ${box.y} ${box.w} ${box.h}`} tabIndex={0} role="application" aria-roledescription="map"
        aria-label={`${label}Map of ${nodes.length} files and ${vis.links.filter(keepL).length} import links`} aria-describedby="lm-walk-keys"
        onKeyDown={walk.onKeyDown} onFocus={walk.onFocus}
        onWheel={onWheel} onPointerDown={(e) => { drag.current = { x: e.clientX, y: e.clientY }; }} onPointerMove={onMove} onPointerUp={() => { drag.current = null; }} onPointerLeave={() => { drag.current = null; }}
        onClick={(e) => { if (e.target === e.currentTarget) setSelKey(null); }}>
        {[...L.centres].filter(([f]) => folders.has(f)).map(([f, c]) => (
          <g key={f}><circle className={walk.folder === f && selI === null ? "lm-group is-walk" : "lm-group"} cx={c.x} cy={c.y} r={c.r} /><text className="lm-glabel" x={c.x} y={c.y - c.r + 16 * u} fontSize={11 * u}>{f}</text></g>
        ))}
        <g className={selI === null && !trace ? undefined : "lm-dim"}>
          {vis.links.filter(keepL).map((j) => {
            const f = sl.links.from[j], a = L.at(sl.nodes.id[f], sl.nodes.name[f]), b = L.at(sl.nodes.id[sl.links.to[j]], sl.nodes.name[sl.links.to[j]]);
            const on = (selI !== null && (f === selI || sl.links.to[j] === selI)) || (!!trace && trace.links.has(sl.links.id[j]));
            return <line key={sl.links.id[j]} className={`lm-link${on ? " is-on" : ""}${only ? "" : st(diff?.link[j])}`} x1={a.x} y1={a.y} x2={b.x} y2={b.y} />;
          })}
        </g>
        {nodes.map((i) => {
          const p = L.at(sl.nodes.id[i], sl.nodes.name[i]);
          return (
            <circle key={sl.nodes.id[i]} className={`lm-node${i === selI ? " is-sel" : ""}${only ? "" : st(diff?.node[i])}${lit(sl.nodes.id[i]) ? "" : " is-faint"}`} cx={p.x} cy={p.y} r={(i === selI ? 6 : 4) * u}
              style={{ fill: hue(L.groups.indexOf(topFolder(sl.nodes.name[i]))) }} onClick={() => select(i, "file")}>
              <title>{sl.nodes.name[i]}</title>
            </circle>
          );
        })}
        {selI !== null && keep(selI) && (() => { const p = L.at(sl.nodes.id[selI], sl.nodes.name[selI]); return <text className="lm-label" x={p.x + 9 * u} y={p.y + 4 * u} fontSize={12 * u} strokeWidth={3 * u}>{sl.nodes.name[selI]}</text>; })()}
      </svg>
    );
  };

  const counts = levelSlice ? `${n(levelVis.nodes.length, gl ? NOUN[glLevel].one : "file")} · ${n(levelVis.links.length, gl && glLevel !== "file" ? "link" : "import link")}` : "";
  const earlier = cmp ? fmtDate(Math.min(cmp.a, cmp.b), "dmy") : "", later = cmp ? fmtDate(Math.max(cmp.a, cmp.b), "dmy") : "";
  let body: ReactNode;
  if (g.error) body = <div className="lm-msg" role="alert"><b>{g.error.what}</b><p>{g.error.why}. {g.error.try_}.</p></div>;
  else if (!current && !wsLoading) body = <div className="lm-msg"><b>No workspace yet</b><p>Index a repository with cartograph index, then reload.</p></div>;
  else if (!s && !gl) body = <div className="lm-msg" aria-busy="true">Loading the map…</div>;
  else if (mode === "table" || lost) {
    body = !levelSlice ? <div className="lm-msg" aria-busy="true">Loading the map…</div> : <TableView s={levelSlice} shown={levelVis} deg={levelDeg} sel={gl ? (selLevel === glLevel ? selI : null) : selI} onOpen={open}
      noun={gl ? NOUN[glLevel] : NOUN.file} diff={levelDiff} />;
  } else if (gl) {
    body = (
      <div className={side && cmp ? "lm-side" : "lm-one"} aria-busy={!glLevels}>
        {!glLevels && <div className="lm-msg lm-over">Loading the map…</div>}
        {(side && cmp ? (["a", "b"] as const) : [null]).map((pane) => (
          <WebGLMap key={pane ?? "one"} levels={pane && glDrawn ? onlySide(glDrawn, pane === "a" ? "earlier" : "later") : glDrawn} L={glL} hue={hueOf} trace={traceSet}
            sel={sel} onSelect={(x) => setSelKey(x)} onLevel={setGlLevel} focus={focusTick} onLost={setLost} camera={camHolder} />
        ))}
      </div>
    );
  } else if (!s) body = null;
  else if (s.nodes.id.length === 0 && !cA) body = <div className="lm-msg"><b>No files at this date</b><p>The history starts later. Pick a later date, or Now.</p></div>;
  else if (side && cA) {
    body = (
      <div className="lm-side">
        <figure><figcaption>{earlier}</figcaption>{svgPane(cA.slice, cA, "a", `${earlier}. `)}</figure>
        <figure><figcaption>{later}</figcaption>{svgPane(cA.slice, cA, "b", `${later}. `)}</figure>
      </div>
    );
  } else body = svgPane(fileSlice!, cA, null, cA ? `Compare ${earlier} with ${later}. ` : "");

  return (
    <div className="sh-frame">
      <AppShell onPick={(id) => selectId(id)} />
      <div className="sh-body">
        <AddedHint />
        <div className="lm">
          <div className="lm-bar">
            {back && <Link className="lm-chip" href={back}><ChevronLeft size={14} aria-hidden />Back to {backLabel(back)}</Link>}
            <label className="lm-at">
              <span>At</span>
              <input type="date" value={day} max={localDay(new Date(today))} onChange={(e) => { setDay(e.target.value); setKnob(null); setSha(""); }} aria-describedby="lm-at-note" />
            </label>
            <button type="button" className="btn" disabled={!day && !sha} onClick={() => { setDay(""); setKnob(null); setSha(""); }}>Now</button>
            <label className="lm-at">
              <span>Filter</span>
              <input type="search" value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Part of a path" />
            </label>
            <div className="lm-seg" role="group" aria-label="Show as">
              <button type="button" aria-pressed={mode === "map"} onClick={() => setMode("map")}>Map</button>
              <button type="button" aria-pressed={mode === "table"} onClick={() => setMode("table")}>Table</button>
            </div>
            {cmp && (
              <div className="lm-seg" role="group" aria-label="Compare as">
                <button type="button" aria-pressed={!side} onClick={() => setSide(false)}>Overlay</button>
                <button type="button" aria-pressed={side} onClick={() => setSide(true)}>Side by side</button>
              </div>
            )}
            {trace && <button type="button" className="lm-chip is-acc" onClick={() => setTrace(null)}>Tracing {nameOf(trace.id).split("/").pop()} · {trace.nodes.size} reached<X size={13} aria-label="Clear trace" /></button>}
            {ws && <CorrectionList ws={ws} />}
            {gl && <span className="lm-chip">{NOUN[glLevel].one[0].toUpperCase() + NOUN[glLevel].one.slice(1)} level</span>}
            <span id="lm-at-note" className="lm-note" aria-live="polite">
              {s && !g.error ? <>{counts}{cmp ? <> · comparing {earlier} with {later}</> : s.at && <> · at commit <code>{s.at.sha.slice(0, 7)}</code></>}</> : null}
              {(g.loading || (cmp && gB.loading)) && <> · Loading…</>}
            </span>
          </div>
          {cmp && <Legend />}
          <div className="lm-main">
            <div className="lm-canvas">
              {body}
              {lost && mode === "map" && <p className="lm-lost" role="status">The graphics context was lost, so the map shows as a table until it comes back.</p>}
              <p id="lm-walk-keys" className="sh-sr">Arrow keys move between folders, or between the files in a folder. Enter opens a folder, Escape goes back up. L steps through the files linked to the current one.</p>
              <p className="sh-sr" aria-live="polite">{walk.said}</p>
            </div>
            {s && ws && selSlice && selI !== null && (
              <Details ref={panel} ws={ws} at={at} s={selSlice} i={selI} name={nameOf} dateOf={dateOf}
                onPick={(i) => select(i, selLevel)} onOpenId={selectId} onTrace={(id) => void traceFrom(id)}
                onJump={(h) => pickCommit(h)} onClose={() => { setSelKey(null); back$.current?.focus(); }} />
            )}
          </div>
          {ws && (
            <div className="lm-time">
              <Timeline ws={ws} commits={commits} t={Math.min(t, today)} today={today} onPick={pickCommit}
                compare={cmp} onCompare={setCmp} repoHue={(_, i) => `var(${token(i)})`} />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** A compare's levels cut to one side: the earlier pane drops what was added, the later what was removed. */
function onlySide(levels: Record<Level, GLLevel>, which: "earlier" | "later"): Record<Level, GLLevel> {
  const drop: DiffState = which === "earlier" ? "added" : "removed";
  const out = {} as Record<Level, GLLevel>;
  for (const lv of ["service", "module", "file"] as Level[]) {
    const x = levels[lv];
    out[lv] = x.diff ? { ...x, diff: undefined, hide: x.diff.node.map((d, i) => d === drop || !!x.hide?.[i]) } : x;
  }
  return out;
}

function Legend() {
  return (
    <div className="lm-legend" role="note" aria-label="Compare legend">
      <span><i className="is-added" aria-hidden />Added</span>
      <span><i className="is-removed" aria-hidden />Removed</span>
      <span><i className="is-changed" aria-hidden />Changed</span>
      <span><i className="is-same" aria-hidden />Unchanged</span>
    </div>
  );
}

/**
 * FE-12's keyboard walk over the drawn map: arrows between siblings (folders, or the files of one folder), Enter
 * down a level, Escape up, L cycles the files linked to the current one. Each move is said in a polite live region.
 */
// ponytail: the spec names data-navigator; this slice has two levels (folder, file), which a key handler covers
// without the dependency. The WebGL map's levels are walked in the table, its accessible twin.
function useWalk(s: GraphSlice | undefined, vis: { nodes: number[] }, deg: { inn: number[]; out: number[] }, sel: number | null, select: (i: number | null) => void) {
  const [folder, setFolder] = useState<string | null>(null);
  const [said, setSaid] = useState("");
  const cycle = useRef<{ from: number; k: number } | null>(null);
  const tree = useMemo(() => {
    const m = new Map<string, number[]>();
    if (s) for (const i of vis.nodes) { const f = topFolder(s.nodes.name[i]); if (!m.has(f)) m.set(f, []); m.get(f)!.push(i); }
    for (const v of m.values()) v.sort((a, b) => s!.nodes.name[a].localeCompare(s!.nodes.name[b]));
    return { folders: [...m.keys()].sort(), files: m };
  }, [s, vis.nodes]);
  if (!s) return { folder, said, onFocus: () => {}, onKeyDown: () => {} };
  const name = (i: number) => s.nodes.name[i];
  const toFolder = (f: string) => { select(null); setFolder(f); setSaid(sayFolder(f, tree.files.get(f)?.length ?? 0)); };
  const toFile = (i: number, prefix = "") => { select(i); setFolder(topFolder(name(i))); setSaid(prefix + sayFile(name(i), deg.inn[i], deg.out[i])); };
  const here = sel !== null && tree.files.get(topFolder(name(sel)))?.includes(sel) ? sel : null;
  const onFocus = () => {
    if (here !== null) toFile(here);
    else if (folder && tree.files.has(folder)) toFolder(folder);
    else if (tree.folders.length) toFolder(tree.folders[0]);
  };
  const onKeyDown = (e: React.KeyboardEvent) => {
    const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (step) {
      e.preventDefault(); cycle.current = null;
      if (here !== null) {
        const sib = tree.files.get(topFolder(name(here)))!;
        toFile(sib[Math.max(0, Math.min(sib.length - 1, sib.indexOf(here) + step))]);
      } else {
        const k = folder ? tree.folders.indexOf(folder) : -1;
        const f = tree.folders[Math.max(0, Math.min(tree.folders.length - 1, k + step))];
        if (f) toFolder(f);
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      const first = folder && here === null ? tree.files.get(folder)?.[0] : undefined;
      if (first !== undefined) toFile(first);
    } else if (e.key === "Escape" && here !== null) {
      e.preventDefault(); e.stopPropagation(); cycle.current = null;
      toFolder(topFolder(name(here)));
    } else if (e.key.toLowerCase() === "l" && !e.metaKey && !e.ctrlKey && !e.altKey && here !== null) {
      e.preventDefault();
      const c = cycle.current && cycle.current.from !== here && linkedTo(cycle.current.from).includes(here) ? cycle.current : { from: here, k: 0 };
      const all = linkedTo(c.from);
      if (!all.length) { setSaid(`${name(c.from)} has no linked files shown`); return; }
      const to = all[c.k % all.length];
      cycle.current = { from: c.from, k: c.k + 1 };
      toFile(to, `linked ${(c.k % all.length) + 1} of ${all.length}: `);
    }
  };
  function linkedTo(i: number) {
    const shownSet = new Set(vis.nodes);
    const l = linked(s!, i);
    return [...l.out, ...l.in].filter((x, k, a) => shownSet.has(x) && a.indexOf(x) === k);
  }
  return { folder, said, onFocus, onKeyDown };
}
