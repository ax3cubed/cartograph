"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  forceCollide, forceLink, forceManyBody, forceSimulation, forceX, forceY, type Simulation, type SimulationNodeDatum,
} from "d3-force";
import { Maximize2, Minus, Plus } from "lucide-react";
import {
  LINKS, NODES, SERVICES, WORKSPACE, byId, children, commitBySha, dayMs, descendants, isActive,
  recentChange, type Link, type LinkType, type Node,
} from "@/lib/workspace";
import { HIDDEN_BELOW } from "@/lib/model";
import { serviceHue } from "./map/domains";
import { level, lkey, neighboursOf, score, visibleIds, type EffLink } from "./map/links";
import SymbolView from "./map/SymbolView";
import { isMenuKey, longPress, onMenuEvent, type MenuTarget } from "./map/NodeMenu";
import { useRoving } from "./sidebar/useRoving";
import "./map/map.css";

export type Layout = "clusters" | "adaptive" | "ontology";
export type Depth = 1 | 2 | 3;
type Pt = { x: number; y: number };
type Cam = { x: number; y: number; w: number; h: number };

const RANK: Record<Node["kind"], number> = { workspace: 0, service: 1, module: 2, file: 3 };
/** Roll a node up to the visible depth. A module opened in place (depth 2) keeps its files. */
const lift = (id: string, depth: Depth, open?: Set<string>) => {
  let n = byId.get(id)!;
  while (RANK[n.kind] > depth && !(n.kind === "file" && depth === 2 && open?.has(n.parent!))) n = byId.get(n.parent!)!;
  return n.id;
};
const nodeHue = (n: Node) => (n.kind === "workspace" ? "var(--accent)" : serviceHue(n.service));
/** A five-point star centred on (cx, cy), for the pinned marker. */
const star = (cx: number, cy: number, R: number) =>
  Array.from({ length: 10 }, (_, i) => {
    const r = i % 2 ? R * 0.45 : R, a = (i * Math.PI) / 5 - Math.PI / 2;
    return `${i ? "L" : "M"}${(cx + r * Math.cos(a)).toFixed(1)},${(cy + r * Math.sin(a)).toFixed(1)}`;
  }).join("") + "Z";
type MenuOpen = (t: MenuTarget, at: Pt | null, from: Element) => void;
/** Right-click, the ContextMenu key, Shift+F10 and a touch long-press all open the context menu for `t`. */
const menuProps = (t: MenuTarget, onMenu?: MenuOpen) => onMenu ? {
  onContextMenu: (e: React.MouseEvent) => { const el = e.currentTarget; onMenuEvent(e, (at) => onMenu(t, at, el)); },
  menuKey: (e: React.KeyboardEvent) => { if (!isMenuKey(e)) return false; e.preventDefault(); e.stopPropagation(); onMenu(t, null, e.currentTarget); return true; },
  press: (e: React.PointerEvent) => { const el = e.currentTarget; longPress(e, (at) => onMenu(t, at, el)); },
} : { onContextMenu: undefined, menuKey: () => false, press: () => {} };

// ---------- fixed layouts ----------

function clusterLayout() {
  const pos = new Map<string, Pt>([[WORKSPACE, { x: 0, y: 0 }]]);
  SERVICES.forEach((s, i) => {
    const a = (i / SERVICES.length) * Math.PI * 2 - Math.PI / 2;
    const sp = { x: Math.cos(a) * 340, y: Math.sin(a) * 255 };
    pos.set(s.id, sp);
    const mods = children(s.id);
    mods.forEach((m, j) => {
      const ma = a + (j - (mods.length - 1) / 2) * 1.25; // fan modules outward so clusters don't collide
      const mp = { x: sp.x + Math.cos(ma) * 100, y: sp.y + Math.sin(ma) * 100 };
      pos.set(m.id, mp);
      const files = children(m.id);
      files.forEach((f, k) => {
        const fa = ma + (k - (files.length - 1) / 2) * 0.95;
        pos.set(f.id, { x: mp.x + Math.cos(fa) * 40, y: mp.y + Math.sin(fa) * 40 });
      });
    });
  });
  return pos;
}

function ontologyLayout() {
  const pos = new Map<string, Pt>([[WORKSPACE, { x: 0, y: 0 }]]);
  const leaves = NODES.filter((n) => n.kind === "file");
  const ang = new Map<string, number>();
  leaves.forEach((f, i) => ang.set(f.id, (i / leaves.length) * Math.PI * 2 - Math.PI / 2 + 0.08));
  const mean = (ids: string[]) => ids.reduce((s, id) => s + ang.get(id)!, 0) / ids.length;
  for (const m of NODES.filter((n) => n.kind === "module")) ang.set(m.id, mean(children(m.id).map((c) => c.id)));
  for (const s of SERVICES) ang.set(s.id, mean(children(s.id).map((c) => c.id)));
  const R: Record<string, number> = { service: 120, module: 225, file: 330 };
  for (const n of NODES) {
    if (n.kind === "workspace") continue;
    const a = ang.get(n.id)!;
    pos.set(n.id, { x: Math.cos(a) * R[n.kind] * 1.3, y: Math.sin(a) * R[n.kind] });
  }
  return pos;
}

// round so server and client render identical transforms (hydration)
const rounded = (m: Map<string, Pt>) => new Map([...m].map(([k, v]) => [k, { x: Math.round(v.x * 10) / 10, y: Math.round(v.y * 10) / 10 }]));
const FIXED = { clusters: rounded(clusterLayout()), ontology: rounded(ontologyLayout()) };

const firstSeen = new Map<string, number>();
for (const n of NODES) {
  const ds = descendants(n.id);
  firstSeen.set(n.id, Math.min(...LINKS.filter((l) => ds.has(l.from) || ds.has(l.to)).map((l) => dayMs(commitBySha.get(l.added)!.date))));
}

const fanIn = (id: string, t: number) => LINKS.filter((l) => l.to === id && isActive(l, t)).length;
const radius = (n: Node, t: number) =>
  n.kind === "workspace" ? 30 : n.kind === "service" ? 25 : n.kind === "module" ? 11 : 4 + Math.min(fanIn(n.id, t), 7) * 1.1;
const hubLabel = (name: string) => {
  const [head, ...rest] = name.split("-");
  return rest.length ? [head, rest.join("-")] : [name];
};
const hash = (s: string) => Math.abs([...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) | 0, 7));

// ---------- camera: viewBox pan/zoom (stays crisp), wheel, drag, pinch ----------

function useCamera(base: Cam) {
  const svg = useRef<SVGSVGElement>(null);
  const [cam, setCam] = useState(base);
  const camRef = useRef(cam);
  camRef.current = cam;
  const raf = useRef(0);
  const moved = useRef(false);

  const clampW = (w: number) => Math.min(base.w * 2.6, Math.max(base.w / 5, w));
  const animateTo = (to: Cam) => {
    cancelAnimationFrame(raf.current);
    const from = camRef.current, t0 = performance.now();
    const step = (now: number) => {
      const k = Math.min(1, (now - t0) / 420), e = 1 - Math.pow(1 - k, 4);
      setCam({ x: from.x + (to.x - from.x) * e, y: from.y + (to.y - from.y) * e, w: from.w + (to.w - from.w) * e, h: from.h + (to.h - from.h) * e });
      if (k < 1) raf.current = requestAnimationFrame(step);
    };
    raf.current = requestAnimationFrame(step);
  };
  const toSvg = (cx: number, cy: number) => new DOMPoint(cx, cy).matrixTransform(svg.current!.getScreenCTM()!.inverse());
  const scale = () => svg.current!.getScreenCTM()!.a;
  const zoomAt = (factor: number, p: Pt, animate = false) => {
    const c = camRef.current;
    const w = clampW(c.w / factor), f = w / c.w;
    const next = { x: p.x - (p.x - c.x) * f, y: p.y - (p.y - c.y) * f, w, h: c.h * f };
    if (animate) animateTo(next);
    else (cancelAnimationFrame(raf.current), setCam(next));
  };
  const centre = () => ({ x: camRef.current.x + camRef.current.w / 2, y: camRef.current.y + camRef.current.h / 2 });
  const fitTo = (pts: Pt[], pad = 90) => {
    if (!pts.length) return animateTo(base);
    const xs = pts.map((p) => p.x), ys = pts.map((p) => p.y);
    const minX = Math.min(...xs) - pad, maxX = Math.max(...xs) + pad, minY = Math.min(...ys) - pad, maxY = Math.max(...ys) + pad;
    const ratio = base.w / base.h;
    const w = clampW(Math.min(Math.max(maxX - minX, (maxY - minY) * ratio, base.w / 3), base.w * 1.2));
    const h = w / ratio;
    animateTo({ x: (minX + maxX) / 2 - w / 2, y: (minY + maxY) / 2 - h / 2, w, h });
  };

  useEffect(() => {
    const el = svg.current!;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      // pinch (ctrl+wheel) and mouse wheels zoom; two-finger trackpad scroll pans
      const mouseWheel = e.deltaMode === 1 || (Math.abs(e.deltaY) >= 40 && e.deltaX === 0 && Number.isInteger(e.deltaY));
      if (e.ctrlKey || mouseWheel) zoomAt(Math.exp(-e.deltaY * (e.ctrlKey ? 0.012 : 0.0018)), toSvg(e.clientX, e.clientY));
      else {
        const s = scale();
        cancelAnimationFrame(raf.current);
        setCam((c) => ({ ...c, x: c.x + e.deltaX / s, y: c.y + e.deltaY / s }));
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  });

  // background drag = pan, two pointers = pinch
  const pointers = useRef(new Map<number, Pt>());
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.current.size > 1) return; // second finger joins the existing gesture
    moved.current = false;
    cancelAnimationFrame(raf.current);
    let pinch = 0;
    const move = (ev: PointerEvent) => {
      const prev = pointers.current.get(ev.pointerId);
      if (!prev) return;
      pointers.current.set(ev.pointerId, { x: ev.clientX, y: ev.clientY });
      if (pointers.current.size >= 2) {
        const [a, b] = [...pointers.current.values()];
        const d = Math.hypot(a.x - b.x, a.y - b.y);
        if (pinch) zoomAt(d / pinch, toSvg((a.x + b.x) / 2, (a.y + b.y) / 2));
        pinch = d;
        moved.current = true;
        return;
      }
      if (!moved.current && Math.abs(ev.clientX - e.clientX) + Math.abs(ev.clientY - e.clientY) < 4) return;
      moved.current = true;
      const s = scale();
      setCam((c) => ({ ...c, x: c.x - (ev.clientX - prev.x) / s, y: c.y - (ev.clientY - prev.y) / s }));
    };
    const up = (ev: PointerEvent) => {
      pointers.current.delete(ev.pointerId);
      pinch = 0;
      if (pointers.current.size) return;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  return { svg, cam, zoom: base.w / cam.w, moved, toSvg, onPointerDown, fitTo, home: () => animateTo(base), zoomBy: (f: number) => zoomAt(f, centre(), true) };
}

/** Zoom buttons sit on the map; the zoom text goes into the canvas bar's status line (`slot`) so nothing overlaps. */
function ZoomControls({ camera, info, slot }: { camera: ReturnType<typeof useCamera>; info?: string; slot?: HTMLElement | null }) {
  return (
    <div className="zoom">
      {slot && createPortal(<>{Math.round(camera.zoom * 100)}%{info ? ` · ${info}` : ""}</>, slot)}
      <button className="round-btn" onClick={() => camera.zoomBy(1 / 1.35)} aria-label="Zoom out"><Minus size={16} /></button>
      <button className="round-btn" onClick={() => camera.zoomBy(1.35)} aria-label="Zoom in"><Plus size={16} /></button>
      <button className="round-btn" onClick={camera.home} aria-label="Fit map"><Maximize2 size={15} /></button>
    </div>
  );
}

// ---------- public component ----------

interface Props {
  focus: boolean;
  layout: Layout;
  depthSetting: Depth | "auto";
  t: number;
  focusId: string | null;
  scopeId: string | null;
  highlight: { nodes: Set<string>; links: Set<string> } | null;
  typeFilter: LinkType | "all";
  minUsedBy: number;
  links: EffLink[]; // links after corrections
  showWeak: boolean;
  compare: { a: number; b: number } | null;
  edgeSel: { a: string; b: string } | null;
  symbolId: string | null;
  onEdge: (e: { a: string; b: string }) => void;
  onSymbol: (id: string) => void;
  onOpen: (id: string) => void;
  onSelect: (id: string) => void;
  onDepth: (d: Depth) => void;
  /** Where the zoom and labelled text renders (the canvas bar's status line). */
  statusSlot?: HTMLElement | null;
  /** Pinned node ids: they get a star and an accent ring. */
  pinned?: Set<string>;
  /** Open the context menu for a node or link (`at` null = from the keyboard). */
  onMenu?: MenuOpen;
}

/** Map items are one tab stop; arrows move between nodes first, then links. */
const ROVE = [".node, .leaf", ".map-aggbtn", ".link-hit"];

export default function Graph(p: Props) {
  if (p.focus && p.focusId && p.symbolId) return <SymbolView symbolId={p.symbolId} onSymbol={p.onSymbol} />;
  return p.focus && p.focusId ? <FocusGraph key={p.focusId} {...p} focusId={p.focusId} /> : <MapGraph {...p} />;
}

// ---------- clusters / adaptive / ontology ----------

type SimNode = SimulationNodeDatum & { id: string };
const BASE: Cam = { x: -500, y: -380, w: 1000, h: 880 }; // extra room at the bottom for the Ask bar

function MapGraph({ layout, depthSetting, t, scopeId, highlight, typeFilter, minUsedBy, links, showWeak, compare, edgeSel, onEdge, onOpen, onSelect, onDepth, statusSlot, pinned, onMenu }: Props) {
  const camera = useCamera(BASE);
  const rove = useRoving(camera.svg, ROVE);
  const { zoom } = camera;
  const [hover, setHover] = useState<string | null>(null);
  // comprehension depth follows zoom: far = services, mid = modules, close = files
  const autoDepth: Depth = zoom < 0.6 ? 1 : zoom < 1.4 ? 2 : 3; // board 00: services below 60%, modules to 140%, files above
  const [open, setOpen] = useState<Set<string>>(new Set()); // modules whose files are shown in place
  const depth: Depth = depthSetting === "auto" ? autoDepth : depthSetting;
  useEffect(() => onDepth(autoDepth), [autoDepth, onDepth]);

  const scope = useMemo(() => (scopeId ? descendants(scopeId) : null), [scopeId]);

  const lo = compare ? Math.min(compare.a, compare.b) : t, hi = compare ? Math.max(compare.a, compare.b) : t;
  const passes = useMemo(() => visibleIds(links, depth, hi, typeFilter, minUsedBy, showWeak), [links, depth, hi, typeFilter, minUsedBy, showWeak]);
  const shown = useMemo(
    () => NODES.filter((n) =>
      (RANK[n.kind] <= depth || (n.kind === "file" && depth === 2 && open.has(n.parent!))) &&
      (n.kind !== "workspace" || layout === "ontology") &&
      (firstSeen.get(n.id) ?? 0) <= hi &&
      (n.kind === "workspace" || passes.has(n.id) || passes.has(n.parent!) && open.has(n.parent!) || highlight?.nodes.has(n.id))),
    [depth, layout, hi, passes, highlight, open],
  );
  const shownIds = useMemo(() => new Set(shown.map((n) => n.id)), [shown]);

  // links rolled up to the visible depth: file links become module or service links
  type Edge = { from: string; to: string; count: number; types: Set<LinkType>; added: boolean; removed: boolean; hot: boolean; best: number; fixed: boolean; cross: boolean; a: number; b: number; cmp?: string };
  const edges = useMemo(() => {
    const m = new Map<string, Edge>();
    for (const l of links) {
      if (typeFilter !== "all" && l.type !== typeFilter) continue;
      if (!showWeak && score(l) < HIDDEN_BELOW) continue;
      const A = compare ? isActive(l, lo) : false, B = compare ? isActive(l, hi) : false;
      const active = compare ? B : isActive(l, t);
      const change = compare ? null : recentChange(l, t);
      if (compare ? !A && !B : !active && change !== "removed") continue;
      const a = lift(l.from, depth, open), b = lift(l.to, depth, open);
      if (a === b || !shownIds.has(a) || !shownIds.has(b)) continue;
      const k = `${a}>${b}`;
      const e = m.get(k) ?? { from: a, to: b, count: 0, types: new Set<LinkType>(), added: false, removed: true, hot: false, best: 0, fixed: false, cross: false, a: 0, b: 0 };
      if (active) (e.count++, (e.removed = false));
      if (A) e.a++;
      if (B) e.b++;
      e.types.add(l.type);
      e.best = Math.max(e.best, score(l));
      if (l.fix) e.fixed = true;
      if (byId.get(l.from)!.service !== byId.get(l.to)!.service) e.cross = true;
      if (change === "added") e.added = true;
      if (highlight?.links.has(lkey(l))) e.hot = true;
      m.set(k, e);
    }
    const out = [...m.values()];
    if (compare) for (const e of out) e.cmp = e.a === 0 ? "added" : e.b === 0 ? "removed" : e.a !== e.b ? "changed" : "same";
    return out;
  }, [links, typeFilter, showWeak, t, lo, hi, compare, depth, open, shownIds, highlight]);

  // ----- adaptive (force) layout -----
  const sim = useRef<Simulation<SimNode, undefined> | null>(null);
  const simNodes = useRef(new Map<string, SimNode>());
  const lastFixed = useRef<"clusters" | "ontology">("clusters");
  if (layout !== "adaptive") lastFixed.current = layout;
  const [, tick] = useState(0);
  const shownKey = shown.map((n) => n.id).join("|");
  const edgeKey = edges.map((e) => e.from + e.to).join("|");

  useEffect(() => {
    if (layout !== "adaptive") {
      sim.current?.stop();
      simNodes.current.clear();
      return;
    }
    const seed = FIXED[lastFixed.current];
    const nodes = shown.map((n): SimNode => {
      const prev = simNodes.current.get(n.id);
      if (prev) return prev;
      // new nodes grow out of their parent
      let parent: string | null = n.parent;
      while (parent && !simNodes.current.has(parent)) parent = byId.get(parent)!.parent;
      const from = parent ? simNodes.current.get(parent)! : seed.get(n.id)!;
      const h = hash(n.id);
      return { id: n.id, x: from.x! + (h % 17) - 8, y: from.y! + ((h >> 5) % 17) - 8 };
    });
    const known = simNodes.current.size > 0;
    simNodes.current = new Map(nodes.map((n) => [n.id, n]));
    type L = { source: string; target: string; dist: number; s: number };
    const tree: L[] = shown.filter((n) => n.parent && shownIds.has(n.parent)).map((n) => ({
      source: n.id, target: n.parent!, dist: n.kind === "file" ? 34 : n.kind === "module" ? 80 : 150, s: 0.7,
    }));
    const deps: L[] = edges.map((e) => ({ source: e.from, target: e.to, dist: 150, s: 0.05 + Math.min(e.count, 5) * 0.02 }));
    const anchor = (id: string) => FIXED.clusters.get(byId.get(id)!.service || id) ?? { x: 0, y: 0 };
    const kind = (d: SimNode) => byId.get(d.id)!.kind;
    const s = forceSimulation(nodes)
      .force("charge", forceManyBody<SimNode>().strength((d) => (kind(d) === "service" ? -500 : kind(d) === "module" ? -160 : -60)))
      .force("link", forceLink<SimNode, L & SimulationNodeDatum>([...tree, ...deps] as never).id((d) => d.id).distance((l) => (l as unknown as L).dist).strength((l) => (l as unknown as L).s))
      .force("collide", forceCollide<SimNode>((d) => radius(byId.get(d.id)!, t) + (kind(d) === "file" ? 6 : 18)))
      .force("x", forceX<SimNode>((d) => anchor(d.id).x).strength(0.05))
      .force("y", forceY<SimNode>((d) => anchor(d.id).y).strength(0.05))
      .alpha(known ? 0.6 : 1)
      .on("tick", () => tick((v) => v + 1));
    sim.current = s;
    return () => void s.stop();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layout, shownKey, edgeKey]);

  const pos = (id: string): Pt => {
    if (layout === "adaptive") {
      const n = simNodes.current.get(id);
      if (n) return { x: n.x!, y: n.y! };
    }
    return FIXED[layout === "ontology" ? "ontology" : "clusters"].get(id)!;
  };

  // drag a node in adaptive mode; in fixed layouts a drag pans the map
  const dragNode = (e: React.PointerEvent, id: string) => {
    if (layout !== "adaptive" || e.button !== 0) return;
    e.stopPropagation();
    const n = simNodes.current.get(id);
    if (!n) return;
    camera.moved.current = false;
    const start = { x: e.clientX, y: e.clientY };
    const move = (ev: PointerEvent) => {
      if (!camera.moved.current && Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) < 4) return;
      camera.moved.current = true;
      const p = camera.toSvg(ev.clientX, ev.clientY);
      n.fx = p.x;
      n.fy = p.y;
      sim.current?.alphaTarget(0.25).restart();
    };
    const up = () => {
      n.fx = n.fy = null;
      sim.current?.alphaTarget(0);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  // fly to the evidence when an answer arrives
  const hlKey = highlight ? [...highlight.nodes].join("|") : "";
  useEffect(() => {
    if (!highlight) return;
    const timer = setTimeout(() => camera.fitTo([...highlight.nodes].map((id) => pos(id)).filter(Boolean)), 60);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hlKey]);

  // emphasis: hover beats answer beats rail scope
  const hlLifted = useMemo(() => (highlight ? new Set([...highlight.nodes].map((id) => lift(id, depth, open))) : null), [highlight, depth, open]);
  const hoverSet = useMemo(() => {
    if (!hover) return null;
    const s = new Set([hover]);
    for (const e of edges) if (!e.removed && (e.from === hover || e.to === hover)) s.add(e.from).add(e.to);
    return s;
  }, [hover, edges]);
  const touched = useMemo(() => {
    if (!compare) return null;
    const s = new Set<string>();
    for (const e of edges) if (e.cmp !== "same") s.add(e.from).add(e.to);
    for (const n of shown) if ((firstSeen.get(n.id) ?? 0) > lo) s.add(n.id);
    return s;
  }, [compare, edges, shown, lo]);
  const lit = (id: string) => (touched ? touched.has(id) : hoverSet ? hoverSet.has(id) : hlLifted ? hlLifted.has(id) : scope ? scope.has(id) : true);

  const curve = (a: Pt, b: Pt, bend: number) => {
    const cx = ((a.x + b.x) / 2) * bend, cy = ((a.y + b.y) / 2) * bend;
    return `M${a.x.toFixed(1)},${a.y.toFixed(1)} Q${cx.toFixed(1)},${cy.toFixed(1)} ${b.x.toFixed(1)},${b.y.toFixed(1)}`;
  };
  const live = layout === "adaptive";
  // zooming spreads things apart but keeps dots and text near screen size
  const k = Math.pow(zoom, -0.8), kr = Math.pow(zoom, -0.55);
  const labelFiles = zoom > 2.1 || layout === "ontology";
  // top 12 services by fan-in carry labels at service level
  const topSvc = useMemo(() => new Set<string>([...SERVICES].sort((p, q) => fanIn(q.id, t) - fanIn(p.id, t)).slice(0, 12).map((x) => x.id)), [t]);
  const atDepth = shown.filter((n) => RANK[n.kind] === depth);
  const zoomInfo = `${["", "services", "modules", "files"][depth]} · ${atDepth.filter((n) => n.kind !== "service" || topSvc.has(n.id)).length} of ${atDepth.length} labelled`;

  return (
    <>
      <ZoomControls camera={camera} info={zoomInfo} slot={statusSlot} />
      <svg
        onKeyDown={(e) => rove.onKey(e)}
        onFocus={rove.onFocus}
        ref={camera.svg}
        className={`graph ${live ? "is-live" : ""}`}
        style={{ ["--k" as string]: k }}
        preserveAspectRatio="xMidYMin meet"
        viewBox={`${camera.cam.x} ${camera.cam.y} ${camera.cam.w} ${camera.cam.h}`}
        onPointerDown={camera.onPointerDown}
        role="group"
        aria-label={`${layout} map, ${["", "services", "modules", "files"][depth]} visible. Drag to pan, scroll or pinch to zoom.`}
      >
        {[180, 290, 400].map((r) => <ellipse key={r} className="orbit" rx={r * 1.3} ry={r} />)}

        <g className="tree-edges">
          {shown.filter((n) => n.parent && shownIds.has(n.parent)).map((n) => {
            const d = curve(pos(n.parent!), pos(n.id), layout === "ontology" ? 1.25 : 1);
            return <path key={n.id} d={d} style={live ? undefined : ({ d: `path('${d}')` } as React.CSSProperties)} className={lit(n.id) ? "" : "is-dim"} />;
          })}
        </g>

        <g className="links">
          {edges.map((e) => {
            const bend = layout === "ontology" ? 0.25 : 1;
            const pa = pos(e.from), pb = pos(e.to);
            const d = curve(pa, pb, bend);
            const hot = (hover && (e.from === hover || e.to === hover)) || e.hot;
            const sel = edgeSel && ((edgeSel.a === e.from && edgeSel.b === e.to) || (descendants(edgeSel.a).has(e.from) && descendants(edgeSel.b).has(e.to)));
            const dim = !hot && !sel && (e.cmp ? e.cmp === "same" : !(lit(e.from) && lit(e.to)));
            const lvl = e.best >= 0.9 ? "exact" : e.best >= 0.6 ? "likely" : "guessed";
            const cls = ["link", lvl !== "exact" ? `c-${lvl}` : "", e.cross ? "is-cross" : "", e.cmp && e.cmp !== "same" ? `cmp-${e.cmp}` : "", e.added && !compare ? "added" : "", e.removed && !compare ? "removed" : "", hot ? "is-hot" : "", sel ? "is-sel" : "", dim ? "is-dim" : "", layout === "ontology" && !hot && !e.added && !compare ? "is-faint" : ""].join(" ");
            const w = Math.min(10, (e.cross ? 2.25 : 1.25) * Math.sqrt(Math.max(e.count, 1)));
            const mx = (pa.x + pb.x) / 4 + ((pa.x + pb.x) / 2) * bend / 2, my = (pa.y + pb.y) / 4 + ((pa.y + pb.y) / 2) * bend / 2;
            const label = `${byId.get(e.from)!.name} → ${byId.get(e.to)!.name}: ${e.count} link${e.count === 1 ? "" : "s"} (${[...e.types].join(", ")})${lvl !== "exact" ? `, ${lvl} at best` : ""}${e.fixed ? ", overridden" : ""}`;
            const pick = () => !camera.moved.current && onEdge({ a: e.from, b: e.to });
            const mp = menuProps({ type: "link", a: e.from, b: e.to }, onMenu);
            return (
              <g key={e.from + e.to}>
                <path className="link-hit" d={d} style={live ? undefined : ({ d: `path('${d}')` } as React.CSSProperties)} tabIndex={-1} role="button" aria-label={`Inspect ${label}`}
                  aria-haspopup="menu" onContextMenu={mp.onContextMenu} onPointerDown={mp.press}
                  onClick={pick} onKeyDown={(ev) => mp.menuKey(ev) || ((ev.key === "Enter" || ev.key === " ") && (ev.preventDefault(), pick()))}
                  onPointerEnter={() => setHover(e.from)} onPointerLeave={() => setHover(null)} />
                <path d={d} className={cls}
                  style={{ strokeWidth: `calc(${w}px * var(--link-w))`, ["--so" as string]: 0.2 + 0.3 * e.best, ...(live ? {} : { d: `path('${d}')` }) } as React.CSSProperties}>
                  <title>{label}</title>
                </path>
                {e.fixed && <rect className="map-dia" x={mx - 5} y={my - 5} width="10" height="10" rx="1.5" transform={`rotate(45 ${mx} ${my})`} />}
              </g>
            );
          })}
        </g>

        {shown.map((n) => {
          const { x, y } = pos(n.id);
          const r = radius(n, t) * kr;
          const hub = n.kind === "service" || n.kind === "workspace";
          const showLabel = (hub && (n.kind !== "service" || topSvc.has(n.id) || depth > 1)) || n.kind === "module" || hover === n.id || (hoverSet?.has(n.id) ?? false) || labelFiles || (n.kind === "file" && (hlLifted?.has(n.id) ?? false));
          const right = layout === "ontology" ? x >= 0 : true;
          const hidden = RANK[n.kind] === depth && depth < 3 ? descendants(n.id).size - 1 : 0;
          const agg = n.kind === "module" && depth === 2 ? children(n.id).length : 0;
          const isOpen = open.has(n.id);
          const toggle = () => setOpen((o) => { const x = new Set(o); x.has(n.id) ? x.delete(n.id) : x.add(n.id); return x; });
          const added = !!compare && (firstSeen.get(n.id) ?? 0) > lo;
          const isPinned = !!pinned?.has(n.id) && n.kind !== "workspace";
          const mp = menuProps({ type: "node", id: n.id }, onMenu);
          return (
            <g
              key={n.id}
              className={`node k-${n.kind} ${lit(n.id) ? "" : "is-dim"} ${hover === n.id ? "is-hover" : ""} ${added ? "is-added" : ""} ${live ? "is-draggable" : ""}`}
              style={{ transform: `translate(${x}px, ${y}px)`, ["--c" as string]: nodeHue(n) }}
              tabIndex={-1}
              role="button"
              aria-label={`${n.kind} ${n.name}${isPinned ? ", pinned" : ""}${hidden ? `, ${hidden} items inside` : ""}`}
              aria-haspopup="menu"
              onPointerDown={(e) => { mp.press(e); dragNode(e, n.id); }}
              onContextMenu={mp.onContextMenu}
              onClick={() => !camera.moved.current && onSelect(n.id)}
              onDoubleClick={() => onOpen(n.id)}
              onKeyDown={(e) => mp.menuKey(e) || (e.key === "Enter" ? (e.preventDefault(), onOpen(n.id)) : e.key === " " && (e.preventDefault(), onSelect(n.id)))}
              onPointerEnter={() => setHover(n.id)}
              onPointerLeave={() => setHover(null)}
              onFocus={() => setHover(n.id)}
              onBlur={() => setHover(null)}
            >
              <circle className="hit" r={Math.max(r + 6 * k, 12 * k)} />
              <circle className={n.kind === "file" ? "ring" : "dot"} r={r} />
              {isPinned && <>
                <circle className="map-pin-ring" r={r + 3.5 * k} />
                <path className="map-pin-star" d={star((r + 2 * k) * 0.71, -(r + 2 * k) * 0.71, 7 * k)} />
              </>}
              {hidden > 0 && <text className="count" y={4 * k} textAnchor="middle">{hidden}</text>}
              {agg > 0 && (
                <g className="map-aggbtn" role="button" tabIndex={-1} aria-pressed={isOpen} aria-label={`${isOpen ? "Hide" : "Show"} ${agg} files in ${n.name}`}
                  onPointerDown={(e) => e.stopPropagation()} onClick={(e) => (e.stopPropagation(), toggle())}
                  onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), e.stopPropagation(), toggle())}>
                  <circle className="map-agg" r={r + 7 * k} />
                </g>
              )}
              {showLabel && (
                hub && layout !== "ontology" ? (
                  <text className="lbl lbl-hub" y={r + 16 * k} textAnchor="middle">
                    {hubLabel(n.name).map((s, i) => <tspan key={i} x="0" dy={i ? 13 * k : 0}>{s}</tspan>)}
                  </text>
                ) : n.kind === "file" && layout === "ontology" ? (
                  <text className="lbl lbl-file" x={right ? r + 6 * k : -r - 6 * k} y={3.5 * k} textAnchor={right ? "start" : "end"}>{n.name}</text>
                ) : (
                  <text className={`lbl ${hub ? "lbl-hub" : n.kind === "module" ? "lbl-mod" : "lbl-file"}`} y={r + 12 * k} textAnchor="middle">{n.name}</text>
                )
              )}
            </g>
          );
        })}
      </svg>
    </>
  );
}

// ---------- focus (ego) view ----------

const FOCUS_BASE: Cam = { x: -560, y: -330, w: 1120, h: 660 };

function FocusGraph({ t, focusId, typeFilter, highlight, links, showWeak, onEdge, onOpen, statusSlot, pinned, onMenu }: Props & { focusId: string }) {
  const camera = useCamera(FOCUS_BASE);
  const rove = useRoving(camera.svg, ROVE);
  const [hover, setHover] = useState<string | null>(null);
  const node = byId.get(focusId)!;
  const shownLinks = useMemo(() => links.filter((l) => showWeak || score(l) >= HIDDEN_BELOW), [links, showWeak]);
  const { dependsOn, usedBy } = neighboursOf(shownLinks, focusId, t);
  const keep = (ls: EffLink[]) => typeFilter === "all" || ls.some((l) => l.type === typeFilter);
  const left = [...dependsOn].filter(([, ls]) => keep(ls));
  const right = [...usedBy].filter(([, ls]) => keep(ls));

  const place = (i: number, count: number, side: -1 | 1): Pt => {
    const spread = Math.min(1.9, 0.34 * (count - 1));
    const a = count === 1 ? 0 : -spread / 2 + (i / (count - 1)) * spread;
    return { x: Math.round(side * Math.cos(a) * 330), y: Math.round(Math.sin(a) * 270) };
  };

  const column = (items: [string, EffLink[]][], side: -1 | 1) =>
    items.map(([id, ls], i) => {
      const other = byId.get(id)!;
      const pt = place(i, items.length, side);
      const hot = hover === id || ls.some((l) => highlight?.links.has(lkey(l)));
      const d = `M0,0 C${pt.x * 0.45},0 ${pt.x * 0.55},${pt.y} ${pt.x},${pt.y}`;
      const change = ls.map((l) => recentChange(l, t)).find(Boolean);
      const best = Math.max(...ls.map(score));
      const inspect = () => !camera.moved.current && onEdge(side < 0 ? { a: focusId, b: id } : { a: id, b: focusId });
      const lm = menuProps(side < 0 ? { type: "link", a: focusId, b: id } : { type: "link", a: id, b: focusId }, onMenu);
      const nm = menuProps({ type: "node", id }, onMenu);
      const isPinned = !!pinned?.has(id);
      return (
        <g key={id}>
          <path className="link-hit" d={d} tabIndex={-1} role="button" aria-label={`Inspect ${ls.length} link${ls.length === 1 ? "" : "s"} with ${other.name}`}
            aria-haspopup="menu" onContextMenu={lm.onContextMenu} onPointerDown={lm.press}
            onClick={inspect} onKeyDown={(e) => lm.menuKey(e) || ((e.key === "Enter" || e.key === " ") && (e.preventDefault(), inspect()))} />
          <path className={`spoke ${best < 0.9 ? "c-" + (best >= 0.6 ? "likely" : "guessed") : ""} ${hot ? "is-hot" : ""} ${change ?? ""}`} d={d} style={{ ["--so" as string]: 0.2 + 0.3 * best }} />
          {ls.some((l) => l.fix) && <rect className="map-dia" x={pt.x / 2 - 5} y={pt.y / 2 - 5} width="10" height="10" rx="1.5" transform={`rotate(45 ${pt.x / 2} ${pt.y / 2})`} />}
          <g
            className="leaf"
            style={{ transform: `translate(${pt.x}px, ${pt.y}px)`, ["--c" as string]: serviceHue(other.service) }}
            tabIndex={-1}
            role="button"
            aria-label={`Open ${other.name}${isPinned ? ", pinned" : ""}, ${ls.map((l) => l.type).join(", ")}`}
            aria-haspopup="menu" onContextMenu={nm.onContextMenu} onPointerDown={nm.press}
            onClick={() => !camera.moved.current && onOpen(id)}
            onKeyDown={(e) => nm.menuKey(e) || ((e.key === "Enter" || e.key === " ") && (e.preventDefault(), onOpen(id)))}
            onPointerEnter={() => setHover(id)}
            onPointerLeave={() => setHover(null)}
            onFocus={() => setHover(id)}
            onBlur={() => setHover(null)}
          >
            <circle className="hit" r="12" />
            <circle className="ring" r="5.5" />
            {isPinned && <path className="map-pin-star" d={star(6, -6, 4.5)} />}
            <text className="lbl lbl-leaf" x={side * 13} y="4" textAnchor={side < 0 ? "end" : "start"}>
              {other.name}
              <tspan className="lbl-type" dx="7">{[...new Set(ls.map((l) => l.type))].join(" · ")}</tspan>
            </text>
          </g>
        </g>
      );
    });

  return (
    <>
      <ZoomControls camera={camera} info="focus view" slot={statusSlot} />
      <svg
        onKeyDown={(e) => rove.onKey(e)}
        onFocus={rove.onFocus}
        ref={camera.svg}
        className="graph focus"
        viewBox={`${camera.cam.x} ${camera.cam.y} ${camera.cam.w} ${camera.cam.h}`}
        onPointerDown={camera.onPointerDown}
        role="group"
        aria-label={`Links of ${node.name}. Drag to pan, scroll or pinch to zoom.`}
      >
        <ellipse className="orbit" rx="330" ry="270" />
        <text className="col-head" x="-330" y="-296" textAnchor="middle">Depends on</text>
        <text className="col-head" x="330" y="-296" textAnchor="middle">Used by</text>
        {left.length === 0 && <text className="col-empty" x="-330" y="4" textAnchor="middle">No outgoing links at this date</text>}
        {right.length === 0 && <text className="col-empty" x="330" y="4" textAnchor="middle">Nothing depends on this yet</text>}
        {column(left, -1)}
        {column(right, 1)}
        <circle className="halo" r="120" />
        <circle className="core" r="46" />
        {pinned?.has(focusId) && <path className="map-pin-star" d={star(34, -34, 9)}><title>Pinned</title></path>}
        <text className="lbl lbl-core" y="72" textAnchor="middle">
          {hubLabel(node.name).map((s, i) => <tspan key={i} x="0" dy={i ? 18 : 0}>{s}</tspan>)}
        </text>
      </svg>
    </>
  );
}
