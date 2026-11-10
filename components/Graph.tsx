"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import {
  forceCollide, forceLink, forceManyBody, forceSimulation, forceX, forceY, type Simulation, type SimulationNodeDatum,
} from "d3-force";
import { Maximize2, Minus, Plus } from "lucide-react";
import {
  LINKS, NODES, SERVICES, WORKSPACE, byId, children, commitBySha, dayMs, descendants, isActive, neighbours,
  recentChange, serviceColor, type Link, type LinkType, type Node,
} from "@/lib/workspace";

export type Layout = "clusters" | "adaptive" | "ontology";
export type Depth = 1 | 2 | 3;
type Pt = { x: number; y: number };
type Cam = { x: number; y: number; w: number; h: number };

const RANK: Record<Node["kind"], number> = { workspace: 0, service: 1, module: 2, file: 3 };
const lift = (id: string, depth: Depth) => {
  let n = byId.get(id)!;
  while (RANK[n.kind] > depth) n = byId.get(n.parent!)!;
  return n.id;
};
const lkey = (l: Link) => `${l.from}>${l.to}`;

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

  const clampW = (w: number) => Math.min(base.w * 1.8, Math.max(base.w / 5, w));
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

function ZoomControls({ camera }: { camera: ReturnType<typeof useCamera> }) {
  return (
    <div className="zoom">
      <span className="zoom-pct">{Math.round(camera.zoom * 100)}%</span>
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
  onOpen: (id: string) => void;
  onSelect: (id: string) => void;
  onDepth: (d: Depth) => void;
}

export default function Graph(p: Props) {
  return p.focus && p.focusId ? <FocusGraph key={p.focusId} {...p} focusId={p.focusId} /> : <MapGraph {...p} />;
}

// ---------- clusters / adaptive / ontology ----------

type SimNode = SimulationNodeDatum & { id: string };
const BASE: Cam = { x: -500, y: -380, w: 1000, h: 880 }; // extra room at the bottom for the Ask bar

function MapGraph({ layout, depthSetting, t, scopeId, highlight, typeFilter, minUsedBy, onOpen, onSelect, onDepth }: Props) {
  const camera = useCamera(BASE);
  const { zoom } = camera;
  const [hover, setHover] = useState<string | null>(null);
  // comprehension depth follows zoom: far = services, mid = modules, close = files
  const autoDepth: Depth = zoom < 0.85 ? 1 : zoom < 1.75 ? 2 : 3;
  const depth: Depth = depthSetting === "auto" ? autoDepth : depthSetting;
  useEffect(() => onDepth(autoDepth), [autoDepth, onDepth]);

  const scope = useMemo(() => (scopeId ? descendants(scopeId) : null), [scopeId]);

  const shown = useMemo(
    () => NODES.filter((n) =>
      RANK[n.kind] <= depth &&
      (n.kind !== "workspace" || layout === "ontology") &&
      (firstSeen.get(n.id) ?? 0) <= t &&
      (n.kind !== "file" || fanIn(n.id, t) >= minUsedBy || highlight?.nodes.has(n.id))),
    [depth, layout, t, minUsedBy, highlight],
  );
  const shownIds = useMemo(() => new Set(shown.map((n) => n.id)), [shown]);

  // links rolled up to the visible depth: file links become module or service links
  const edges = useMemo(() => {
    const m = new Map<string, { from: string; to: string; count: number; types: Set<LinkType>; added: boolean; removed: boolean; hot: boolean }>();
    for (const l of LINKS) {
      if (typeFilter !== "all" && l.type !== typeFilter) continue;
      const change = recentChange(l, t);
      const active = isActive(l, t);
      if (!active && change !== "removed") continue;
      const a = lift(l.from, depth), b = lift(l.to, depth);
      if (a === b || !shownIds.has(a) || !shownIds.has(b)) continue;
      const k = `${a}>${b}`;
      const e = m.get(k) ?? { from: a, to: b, count: 0, types: new Set<LinkType>(), added: false, removed: true, hot: false };
      if (active) (e.count++, (e.removed = false));
      e.types.add(l.type);
      if (change === "added") e.added = true;
      if (highlight?.links.has(lkey(l))) e.hot = true;
      m.set(k, e);
    }
    return [...m.values()];
  }, [typeFilter, t, depth, shownIds, highlight]);

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
  const hlLifted = useMemo(() => (highlight ? new Set([...highlight.nodes].map((id) => lift(id, depth))) : null), [highlight, depth]);
  const hoverSet = useMemo(() => {
    if (!hover) return null;
    const s = new Set([hover]);
    for (const e of edges) if (!e.removed && (e.from === hover || e.to === hover)) s.add(e.from).add(e.to);
    return s;
  }, [hover, edges]);
  const lit = (id: string) => (hoverSet ? hoverSet.has(id) : hlLifted ? hlLifted.has(id) : scope ? scope.has(id) : true);

  const curve = (a: Pt, b: Pt, bend: number) => {
    const cx = ((a.x + b.x) / 2) * bend, cy = ((a.y + b.y) / 2) * bend;
    return `M${a.x.toFixed(1)},${a.y.toFixed(1)} Q${cx.toFixed(1)},${cy.toFixed(1)} ${b.x.toFixed(1)},${b.y.toFixed(1)}`;
  };
  const live = layout === "adaptive";
  // zooming spreads things apart but keeps dots and text near screen size
  const k = Math.pow(zoom, -0.8), kr = Math.pow(zoom, -0.55);
  const labelFiles = zoom > 2.1 || layout === "ontology";

  return (
    <>
      <ZoomControls camera={camera} />
      <svg
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
            const d = curve(pos(e.from), pos(e.to), layout === "ontology" ? 0.25 : 1);
            const hot = (hover && (e.from === hover || e.to === hover)) || e.hot;
            const dim = !hot && !(lit(e.from) && lit(e.to));
            const cls = ["link", e.types.size === 1 ? `t-${[...e.types][0]}` : "", e.added ? "added" : "", e.removed ? "removed" : "", hot ? "is-hot" : "", dim ? "is-dim" : "", layout === "ontology" && !hot && !e.added ? "is-faint" : ""].join(" ");
            return (
              <path key={e.from + e.to} d={d} className={cls}
                style={{ strokeWidth: 1 + Math.log2(Math.max(e.count, 1)) * 0.9, ...(live ? {} : { d: `path('${d}')` }) } as React.CSSProperties}>
                <title>{`${byId.get(e.from)!.name} → ${byId.get(e.to)!.name}: ${e.count} link${e.count === 1 ? "" : "s"} (${[...e.types].join(", ")})`}</title>
              </path>
            );
          })}
        </g>

        {shown.map((n) => {
          const { x, y } = pos(n.id);
          const r = radius(n, t) * kr;
          const hub = n.kind === "service" || n.kind === "workspace";
          const showLabel = hub || n.kind === "module" || hover === n.id || (hoverSet?.has(n.id) ?? false) || labelFiles || (n.kind === "file" && (hlLifted?.has(n.id) ?? false));
          const right = layout === "ontology" ? x >= 0 : true;
          const hidden = RANK[n.kind] === depth && depth < 3 ? descendants(n.id).size - 1 : 0;
          return (
            <g
              key={n.id}
              className={`node k-${n.kind} ${lit(n.id) ? "" : "is-dim"} ${hover === n.id ? "is-hover" : ""} ${live ? "is-draggable" : ""}`}
              style={{ transform: `translate(${x}px, ${y}px)`, ["--c" as string]: n.kind === "workspace" ? "var(--accent)" : serviceColor(n.service) }}
              tabIndex={0}
              role="button"
              aria-label={`${n.kind} ${n.name}${hidden ? `, ${hidden} items inside` : ""}`}
              onPointerDown={(e) => dragNode(e, n.id)}
              onClick={() => !camera.moved.current && onSelect(n.id)}
              onDoubleClick={() => onOpen(n.id)}
              onKeyDown={(e) => (e.key === "Enter" ? (e.preventDefault(), onOpen(n.id)) : e.key === " " && (e.preventDefault(), onSelect(n.id)))}
              onPointerEnter={() => setHover(n.id)}
              onPointerLeave={() => setHover(null)}
              onFocus={() => setHover(n.id)}
              onBlur={() => setHover(null)}
            >
              <circle className="hit" r={Math.max(r + 6 * k, 12 * k)} />
              <circle className={n.kind === "file" ? "ring" : "dot"} r={r} />
              {hidden > 0 && <text className="count" y={4 * k} textAnchor="middle">{hidden}</text>}
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

function FocusGraph({ t, focusId, typeFilter, highlight, onOpen }: Props & { focusId: string }) {
  const camera = useCamera(FOCUS_BASE);
  const [hover, setHover] = useState<string | null>(null);
  const node = byId.get(focusId)!;
  const { dependsOn, usedBy } = neighbours(focusId, t);
  const keep = (ls: Link[]) => typeFilter === "all" || ls.some((l) => l.type === typeFilter);
  const left = [...dependsOn].filter(([, ls]) => keep(ls));
  const right = [...usedBy].filter(([, ls]) => keep(ls));

  const place = (i: number, count: number, side: -1 | 1): Pt => {
    const spread = Math.min(1.9, 0.34 * (count - 1));
    const a = count === 1 ? 0 : -spread / 2 + (i / (count - 1)) * spread;
    return { x: Math.round(side * Math.cos(a) * 330), y: Math.round(Math.sin(a) * 270) };
  };

  const column = (items: [string, Link[]][], side: -1 | 1) =>
    items.map(([id, ls], i) => {
      const other = byId.get(id)!;
      const pt = place(i, items.length, side);
      const hot = hover === id || ls.some((l) => highlight?.links.has(lkey(l)));
      const d = `M0,0 C${pt.x * 0.45},0 ${pt.x * 0.55},${pt.y} ${pt.x},${pt.y}`;
      const change = ls.map((l) => recentChange(l, t)).find(Boolean);
      return (
        <g key={id}>
          <path className={`spoke ${hot ? "is-hot" : ""} ${change ?? ""}`} d={d} />
          <g
            className="leaf"
            style={{ transform: `translate(${pt.x}px, ${pt.y}px)`, ["--c" as string]: serviceColor(other.service) }}
            tabIndex={0}
            role="button"
            aria-label={`Open ${other.name}, ${ls.map((l) => l.type).join(", ")}`}
            onClick={() => !camera.moved.current && onOpen(id)}
            onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), onOpen(id))}
            onPointerEnter={() => setHover(id)}
            onPointerLeave={() => setHover(null)}
            onFocus={() => setHover(id)}
            onBlur={() => setHover(null)}
          >
            <circle className="hit" r="12" />
            <circle className="ring" r="5.5" />
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
      <ZoomControls camera={camera} />
      <svg
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
        <text className="lbl lbl-core" y="72" textAnchor="middle">
          {hubLabel(node.name).map((s, i) => <tspan key={i} x="0" dy={i ? 18 : 0}>{s}</tspan>)}
        </text>
      </svg>
    </>
  );
}
