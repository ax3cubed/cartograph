"use client";

// FE-08: the map in WebGL, sigma 4 over graphology, for workspaces past I1's 2,000 drawn elements (the SVG map in
// LiveMap stays below that). One graph per level of detail, swapped as the camera crosses board 00's zoom marks and
// kept for the last 3 levels visited; every level shares one bounding box, so a swap never moves the camera. The
// canvas is aria-hidden: the table (FE-12) is its accessible twin. sigma loads on first use, so pages without a map
// don't carry it.
import { useEffect, useRef, useState } from "react";
import type Graph from "graphology";
import type Sigma from "sigma";
import type { GraphSlice } from "@/lib/api";
import { bundleWidth, levelFor, placeOf, zoomOf, type DiffState, type Layout, type Level } from "./levels";

export interface GLLevel {
  slice: GraphSlice;
  /** Compare (I4): the state of each node and link. */
  diff?: { node: DiffState[]; link: DiffState[] };
  /** The service each node belongs to, for its hue; and the holder a file sits in, for a seat when it has none. */
  service: (i: number) => string;
  holder?: (i: number) => string;
  /** Side by side: nodes this pane leaves out. */
  hide?: boolean[];
}

export interface WebGLMapProps {
  /** Null while the data loads: sigma starts anyway, so the first level draws as soon as it arrives. */
  levels: Record<Level, GLLevel> | null;
  L: Layout | null;
  /** Hue of a service id (a CSS colour value or var()). */
  hue: (service: string) => string;
  /** Nodes kept bright by a trace; the rest are dimmed. */
  trace?: Set<string> | null;
  sel: { level: Level; id: string } | null;
  onSelect: (s: { level: Level; id: string } | null) => void;
  onLevel: (l: Level) => void;
  /** Bumped to fly the camera to `sel`. */
  focus?: number;
  /** WebGL lost and not restored: the map offers the table. */
  onLost: (lost: boolean) => void;
  /** Side by side (I4): panes given one holder share one camera. */
  camera?: { cam?: unknown };
}

/** At most this many files are drawn at once; past it, only the services in view contribute theirs (I1). */
const MAX_FILES = 10_000;
/** sigma's ratio for each level's resting zoom (fit is 50%; levels.ts). */
const RATIO: Record<Level, number> = { service: 1, module: 50 / 100, file: 50 / 220 };

type Css = (v: string) => string;
/** Tokens as rgb() strings: sigma takes colours, not var(). Read through a hidden element so any CSS colour works. */
const tokens = (): Css => {
  const cs = getComputedStyle(document.documentElement);
  let probe = document.getElementById("lm-gl-probe");
  if (!probe) {
    probe = document.createElement("i");
    probe.id = "lm-gl-probe";
    probe.style.display = "none";
    document.body.appendChild(probe);
  }
  const el = probe;
  const cache = new Map<string, string>();
  return (v) => {
    let c = cache.get(v);
    if (c === undefined) {
      el.style.color = "";
      el.style.color = v.startsWith("--") ? cs.getPropertyValue(v).trim() : v;
      c = getComputedStyle(el).color || "rgb(128, 128, 128)";
      cache.set(v, c);
    }
    return c;
  };
};
const alpha = (rgb: string, a: number) => rgb.replace(/^rgba?\(([^,]+),([^,]+),([^,)]+).*$/, `rgba($1,$2,$3,${a})`);

const DIFF: Record<DiffState, string> = { added: "--up", removed: "--down", changed: "--warn", unchanged: "--edge" };

function build(GraphCtor: typeof Graph, lv: GLLevel, level: Level, L: Layout, css: Css, hue: (s: string) => string, cull: ((x: number, y: number, r: number) => boolean) | null) {
  const g = new GraphCtor({ type: "directed", multi: false, allowSelfLoops: false });
  const s = lv.slice, n = s.nodes;
  const lineW = level === "service" ? 0.35 : level === "module" ? 0.12 : 0.035;
  // context: the level above, as faint discs the nodes sit in
  if (level !== "service") {
    const seen = new Set<string>();
    for (let i = 0; i < n.id.length; i++) {
      const box = level === "module" ? lv.service(i) : lv.holder?.(i);
      if (!box || seen.has(box)) continue;
      seen.add(box);
      const d = L.modules.get(box) ?? L.services.get(box);
      if (!d || (cull && !cull(d.x, d.y, d.r))) continue;
      g.addNode(`ctx:${box}`, { x: d.x, y: d.y, size: d.r, color: alpha(css(hue(level === "module" ? box : lv.service(i))), 0.08), ring: 0, zIndex: 0, ctx: true, label: "" });
    }
  }
  // the top 12 by fan-in get a label
  const fanIn = new Array<number>(n.id.length).fill(0);
  s.links.to.forEach((t, j) => { fanIn[t] += s.links.weight[j]; });
  const top = new Set([...fanIn.keys()].sort((a, b) => fanIn[b] - fanIn[a] || a - b).slice(0, 12));
  let files = 0;
  for (let i = 0; i < n.id.length; i++) {
    const kind = n.kind[i];
    const p = placeOf(L, n.id[i], kind === "file" ? "file" : kind === "module" ? "module" : "service", lv.holder?.(i));
    if ((cull && !cull(p.x, p.y, p.r)) || lv.hide?.[i]) continue;
    if (kind === "file" && ++files > MAX_FILES) break;
    const st = lv.diff?.node[i];
    const base = css(hue(lv.service(i)));
    const color = st === "removed" ? alpha(css("--down"), 0.55) : st === "unchanged" ? alpha(base, 0.35) : st === "added" ? css("--up") : base;
    const r = kind === "file" ? p.r : p.r * (level === "service" ? 0.5 : 0.6);
    const fill = kind === "file" ? color : alpha(color, kind === "service" ? 0.9 : 0.95);
    g.addNode(n.id[i], {
      x: p.x, y: p.y, size: r, color: fill, c0: fill,
      ring: kind === "file" ? 1 : 0, zIndex: 1,
      label: kind === "file" ? n.name[i].split("/").pop() : n.name[i], top: top.has(i), labelColor: css("--text"),
    });
  }
  for (let j = 0; j < s.links.id.length; j++) {
    const a = n.id[s.links.from[j]], b = n.id[s.links.to[j]];
    if (!g.hasNode(a) || !g.hasNode(b) || g.hasEdge(a, b)) continue;
    const st = lv.diff?.link[j];
    const lvl = s.links.level[j];
    const dotted = lvl !== "exact";
    const ec = alpha(css(st ? DIFF[st] : "--edge"), st === "unchanged" ? 0.25 : lvl === "guessed" ? 0.35 : 0.6);
    g.addDirectedEdgeWithKey(s.links.id[j], a, b, {
      size: bundleWidth(s.links.weight[j], lineW),
      color: ec, c0: ec,
      // DESIGN.md link encoding: exact solid, likely dotted, guessed fainter dotted; removed (compare) dashed
      dash: st === "removed" ? 6 : dotted ? 2 : 1e5, gap: st === "removed" ? 4 : dotted ? 3 : 1, // solid: one dash longer than any line
    });
  }
  return g;
}

/** sigma, graphology and sigma's programs, fetched once. LiveMap starts this when the workspace is big enough. */
let libs: Promise<[typeof import("sigma"), typeof import("graphology"), typeof import("sigma/rendering")]> | null = null;
export const loadLibs = () => (libs ??= Promise.all([import("sigma"), import("graphology"), import("sigma/rendering")]));

export default function WebGLMap(p: WebGLMapProps) {
  const box = useRef<HTMLDivElement>(null);
  const S = useRef<{ sigma: Sigma; Graph: typeof Graph; level: Level; cache: Map<string, Graph>; css: Css } | null>(null);
  const props = useRef(p);
  props.current = p;
  const [ready, setReady] = useState(0);
  const [failed, setFailed] = useState(false);

  // one sigma per mount
  useEffect(() => {
    let dead = false;
    let sigma: Sigma | null = null;
    (async () => {
      const [{ default: SigmaCtor }, { default: GraphCtor }, R] = await loadLibs();
      if (dead || !box.current) return;
      // Files are rings: a fragment layer that keeps the outer band of the disc (DESIGN.md: hollow for files).
      const ring = {
        name: "ring",
        uniforms: [],
        attributes: [{ name: "fillColor", size: 4 as const, type: WebGL2RenderingContext.UNSIGNED_BYTE, normalized: true, source: "color" }, { name: "ring", size: 1 as const, type: WebGL2RenderingContext.FLOAT, source: "ring" }],
        glsl: `
vec4 layer_ring(vec4 v_fillColor, float v_ring) {
  float w = max(1.5 * context.pixelToUV, 0.22 * context.shapeSize);
  float band = smoothstep(-w - context.aaWidth, -w, context.sdf);
  return vec4(v_fillColor.rgb, v_fillColor.a * mix(1.0, band, v_ring));
}`,
      };
      const css = tokens();
      try {
        sigma = new SigmaCtor(new GraphCtor(), box.current, {
          primitives: {
            nodes: {
              shapes: [R.sdfCircle()], variables: { ring: { type: "number", default: 0 } }, layers: [ring],
              label: { font: { family: getComputedStyle(document.body).fontFamily, weight: "600", size: 12 }, margin: 6 },
            },
            edges: { variables: { dash: { type: "number", default: 1 }, gap: { type: "number", default: 0 } }, layers: [R.layerDashed({ dashSize: { attribute: "dash", default: 1, mode: "pixels" }, gapSize: { attribute: "gap", default: 1, mode: "pixels" }, align: 0 })] },
          },
          styles: {
            nodes: {
              x: { attribute: "x" }, y: { attribute: "y" }, size: { attribute: "size" }, color: { attribute: "color" },
              ring: { attribute: "ring", defaultValue: 0 }, zIndex: { attribute: "zIndex", defaultValue: 1 },
              label: { attribute: "label" }, labelColor: { attribute: "labelColor" }, labelSize: 12,
              labelVisibility: (a: Record<string, unknown>, st: { isHovered: boolean; isHighlighted: boolean }) => (a.ctx ? "hidden" : a.top || st.isHovered || st.isHighlighted ? "visible" : "hidden"),
              // DESIGN.md focus: the selected node gets a soft accent halo
              backdropVisibility: { whenState: "isHighlighted", then: "visible", else: "hidden" },
              backdropColor: alpha(css("--accent"), 0.18), backdropShadowColor: alpha(css("--accent"), 0.6), backdropShadowBlur: 14, backdropPadding: 4,
              cursor: "pointer",
            },
            edges: { size: { attribute: "size" }, color: { attribute: "color" }, dash: { attribute: "dash", defaultValue: 1 }, gap: { attribute: "gap", defaultValue: 0 } },
          },
          settings: {
            itemSizesReference: "positions", autoRescale: "once", zoomToSizeRatioFunction: (r: number) => r,
            renderEdgeLabels: false, hideEdgesOnMove: false, minEdgeThickness: 0.6, labelRenderedSizeThreshold: 0,
            minCameraRatio: 0.02, maxCameraRatio: 3, stagePadding: 24,
          },
        } as never);
      } catch {
        props.current.onLost(true);
        setFailed(true);
        return;
      }
      const { L, camera } = props.current;
      if (L) sigma.setCustomBBox({ x: L.box.x, y: L.box.y });
      if (camera?.cam) sigma.setCamera(camera.cam as ReturnType<Sigma["getCamera"]>);
      else if (camera) camera.cam = sigma.getCamera();
      S.current = { sigma, Graph: GraphCtor, level: "service", cache: new Map(), css };
      sigma.on("clickNode", ({ node }) => {
        const cur = S.current!;
        props.current.onSelect(node.startsWith("ctx:") ? null : { level: cur.level, id: node });
      });
      sigma.on("clickStage", () => props.current.onSelect(null));
      sigma.on("webglContextLost", () => props.current.onLost(true));
      sigma.on("webglContextRestored", () => { props.current.onLost(false); S.current?.cache.clear(); setReady((r) => r + 1); });
      const cam = sigma.getCamera();
      if (!camera || camera.cam === cam) cam.setState({ x: 0.5, y: 0.5, ratio: RATIO.service, angle: 0 });
      let moved = 0;
      cam.on("updated", (c) => {
        const cur = S.current;
        if (!cur) return;
        const want = levelFor(c.ratio);
        if (want !== cur.level) { cur.level = want; props.current.onLevel(want); setReady((r) => r + 1); }
        else if (want === "file") { cancelAnimationFrame(moved); moved = requestAnimationFrame(() => setReady((r) => r + 1)); }
      });
      setReady(1);
    })();
    return () => { dead = true; sigma?.kill(); S.current = null; };
  }, []);

  // theme or contrast changed: colours are read again
  useEffect(() => {
    const mo = new MutationObserver(() => { if (S.current) { S.current.css = tokens(); S.current.cache.clear(); setReady((r) => r + 1); } });
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-contrast", "data-hues"] });
    return () => mo.disconnect();
  }, []);

  // the drawn graph: this level's, from the cache when the data hasn't changed
  const { levels, L, hue, sel, trace, focus } = p;
  useEffect(() => {
    const cur = S.current;
    if (!cur || !ready || !levels || !L) return;
    const { sigma } = cur;
    sigma.setCustomBBox({ x: L.box.x, y: L.box.y });
    const level = cur.level;
    const lv = levels[level];
    let cull: ((x: number, y: number, r: number) => boolean) | null = null;
    let key = `${level}`;
    if (level === "file" && lv.slice.nodes.id.length > MAX_FILES) {
      const { width, height } = sigma.getDimensions();
      const a = sigma.viewportToGraph({ x: 0, y: 0 }), b = sigma.viewportToGraph({ x: width, y: height });
      const [x0, x1, y0, y1] = [Math.min(a.x, b.x), Math.max(a.x, b.x), Math.min(a.y, b.y), Math.max(a.y, b.y)];
      cull = (x, y, r) => x + r >= x0 && x - r <= x1 && y + r >= y0 && y - r <= y1;
      // the services in view name the culled graph, so panning inside them reuses it
      key += `:${[...L.services].filter(([, d]) => cull!(d.x, d.y, d.r)).map(([id]) => id).join(",")}`;
    }
    const tagged = cur.cache.get(key) as (Graph & { _of?: unknown }) | undefined;
    let g = tagged && tagged._of === lv ? tagged : undefined;
    if (!g) {
      g = build(cur.Graph, lv, level, L, cur.css, hue, cull);
      (g as Graph & { _of?: unknown })._of = lv;
      cur.cache.set(key, g);
      while (cur.cache.size > 3) cur.cache.delete(cur.cache.keys().next().value!);
    }
    if (sigma.getGraph() !== g) {
      sigma.setGraph(g);
      // FE-08's gate is timed from the data's arrival to the first drawn level (LiveMap marks the arrival)
      if (g.order && !performance.getEntriesByName("cartograph:map-drawn").length) sigma.once("afterRender", () => performance.mark("cartograph:map-drawn"));
    }
    // selection, and a trace's dimming
    const want = sel && sel.level === level ? sel.id : null;
    const dim = (c: string) => alpha(c, 0.12);
    g.forEachNode((id, a) => {
      const on = id === want;
      if (sigma.getNodeState(id).isHighlighted !== on) sigma.setNodeState(id, { isHighlighted: on });
      if (a.ctx) return;
      const lit = !trace || trace.has(id);
      g.mergeNodeAttributes(id, { color: lit ? a.c0 : dim(a.c0), zIndex: lit && trace ? 2 : 1 });
    });
    g.forEachEdge((e, a, src, dst) => {
      const lit = !trace || (trace.has(src) && trace.has(dst));
      g.setEdgeAttribute(e, "color", lit ? a.c0 : dim(a.c0));
    });
    sigma.refresh();
  }, [ready, levels, L, hue, sel, trace]);

  // fly to the selection
  useEffect(() => {
    const cur = S.current;
    if (!cur || !focus || !sel) return;
    const node = cur.sigma.getNodeDisplayData(sel.id);
    if (!L) return;
    const pl = placeOf(L, sel.id, sel.level);
    const at = node ? { x: node.x, y: node.y } : cur.sigma.graphToFramedGraph?.({ x: pl.x, y: pl.y });
    if (at) void cur.sigma.getCamera().animate({ ...at, ratio: RATIO[sel.level] }, { duration: 400 });
    // focus is the trigger; sel and L are read as they are then
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus]);

  return (
    <div className="lm-gl" aria-hidden>
      <div ref={box} className="lm-gl-stage" />
      {failed && <p className="lm-note">This browser can&apos;t draw the map in WebGL.</p>}
      <span className="lm-gl-zoom">{S.current ? `${Math.round(zoomOf(S.current.sigma.getCamera().ratio))}%` : ""}</span>
    </div>
  );
}
