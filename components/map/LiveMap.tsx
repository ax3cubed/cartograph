"use client";

import { useEffect, useMemo, useRef, useState, type WheelEvent as ReactWheel } from "react";
import type { GraphSlice } from "@/lib/api";
import { useGraph, useLive, useWorkspaces } from "@/lib/api/live";
import { endOfLocalDay, localDay } from "@/lib/api/at";
import Atlas from "../Atlas";
import { ShellFrame } from "../shell/AppShell";
import "./liveMap.css";

/** The map route: the demo atlas, or with the binary the workspace's own files and import links (FE-06). */
export default function MapRoute() {
  return useLive() ? <LiveMap /> : <Atlas />;
}

/** I1: the SVG path draws at most this many nodes plus links; above it the fast renderer (FE-08) takes over. */
const MAX_DRAWN = 2000;
const GOLDEN = Math.PI * (3 - Math.sqrt(5));
const STEP = 9; // spacing between files in a folder

type Pt = { x: number; y: number };
const hash = (s: string) => { let h = 2166136261; for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619); return h >>> 0; };
const topFolder = (path: string) => (path.includes("/") ? path.slice(0, path.indexOf("/")) : ".");
const n = (k: number, what: string) => `${k.toLocaleString()} ${what}${k === 1 ? "" : "s"}`;
const hue = (i: number) => (i < 6 ? `var(--hue-${i + 1})` : "var(--steel)");

/**
 * Positions for one slice. Stored positions (I2) win when the index has them. Until it does, each top folder is a
 * disc of files (a sunflower, ordered by a hash of the id so a file keeps its rank as others come and go), and the
 * discs are packed on shelves, largest first.
 */
// ponytail: client layout until IDX-25 stores x, y in the index; then this is the `stored` branch only.
function layout(s: GraphSlice) {
  const n = s.nodes.id.length;
  const folder = s.nodes.name.map(topFolder);
  const groups = [...new Set(folder)].sort();
  const pos: Pt[] = new Array(n);
  const stored = s.nodes.x.some((x, i) => x !== 0 || s.nodes.y[i] !== 0);
  const centres = new Map<string, Pt & { r: number }>();
  if (stored) {
    for (let i = 0; i < n; i++) pos[i] = { x: s.nodes.x[i], y: s.nodes.y[i] };
  } else {
    const members = new Map<string, number[]>(groups.map((g) => [g, []]));
    folder.forEach((g, i) => members.get(g)!.push(i));
    const bySize = [...groups].sort((a, b) => members.get(b)!.length - members.get(a)!.length || a.localeCompare(b));
    const radius = (g: string) => STEP * Math.sqrt(members.get(g)!.length + 1) + 24;
    const area = bySize.reduce((a, g) => a + (2 * radius(g)) ** 2, 0);
    // an empty slice (no tracked files at this date) has no folders to size the shelf by
    const width = bySize.length ? Math.max(2 * radius(bySize[0]), Math.sqrt(area) * 1.3) : 0;
    let x = 0, y = 0, row = 0;
    for (const g of bySize) {
      const r = radius(g);
      if (x > 0 && x + 2 * r > width) { x = 0; y += row; row = 0; }
      const c = { x: x + r, y: y + r, r };
      centres.set(g, c);
      x += 2 * r; row = Math.max(row, 2 * r);
      members.get(g)!.sort((a, b) => hash(s.nodes.id[a]) - hash(s.nodes.id[b])).forEach((i, k) => {
        const rr = STEP * Math.sqrt(k + 0.5), a = k * GOLDEN;
        pos[i] = { x: c.x + rr * Math.cos(a), y: c.y + rr * Math.sin(a) };
      });
    }
  }
  const xs = pos.map((p) => p.x), ys = pos.map((p) => p.y);
  const pad = 40;
  const box = n ? { x: Math.min(...xs) - pad, y: Math.min(...ys) - pad, w: Math.max(...xs) - Math.min(...xs) + 2 * pad, h: Math.max(...ys) - Math.min(...ys) + 2 * pad } : { x: 0, y: 0, w: 100, h: 100 };
  return { pos, folder, groups, centres, box };
}

function LiveMap() {
  const { current, loading: wsLoading } = useWorkspaces();
  const [at, setAt] = useState(""); // "" = now; else a local YYYY-MM-DD
  const g = useGraph(current?.id, at ? endOfLocalDay(at) : "");
  const s = g.data;
  const [picked, setSel] = useState<number | null>(null);
  const [view, setView] = useState<{ x: number; y: number; w: number; h: number } | null>(null);
  const svg = useRef<SVGSVGElement>(null);
  const drag = useRef<{ x: number; y: number } | null>(null);

  const size = s ? s.nodes.id.length + s.links.from.length : 0;
  const L = useMemo(() => (s && size <= MAX_DRAWN ? layout(s) : null), [s, size]);
  useEffect(() => { setView(null); }, [L?.box.w, L?.box.h]);
  // keep the selection on the same file across slices; drop it when the file isn't live at the new date
  const selId = useRef<string | null>(null);
  useEffect(() => {
    if (!s) return;
    const i = selId.current ? s.nodes.id.indexOf(selId.current) : -1;
    setSel(i >= 0 ? i : null);
  }, [s]);
  const select = (i: number | null) => { selId.current = i === null || !s ? null : s.nodes.id[i]; setSel(i); };
  // A new slice renders once with the old index before the effect above maps it by id. Use only an index that still
  // names the selected file in this slice, so a smaller slice is never read past its end.
  const sel = s && picked !== null && picked < s.nodes.id.length && s.nodes.id[picked] === selId.current ? picked : null;
  useEffect(() => {
    const k = (e: KeyboardEvent) => { if (e.key === "Escape") select(null); };
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  });

  const links = useMemo(() => {
    if (!s || sel === null) return { out: [] as number[], in: [] as number[] };
    const out: number[] = [], inn: number[] = [];
    s.links.from.forEach((f, j) => { if (f === sel) out.push(s.links.to[j]); if (s.links.to[j] === sel) inn.push(f); });
    return { out, in: inn };
  }, [s, sel]);

  const box = view ?? L?.box;
  const u = L ? Math.max(L.box.w, L.box.h) / 900 : 1; // one screen pixel, roughly, at the fitted zoom
  const onWheel = (e: ReactWheel<SVGSVGElement>) => {
    if (!box || !svg.current) return;
    const r = svg.current.getBoundingClientRect();
    const k = Math.exp(e.deltaY * 0.0015);
    const px = box.x + ((e.clientX - r.left) / r.width) * box.w, py = box.y + ((e.clientY - r.top) / r.height) * box.h;
    setView({ x: px - (px - box.x) * k, y: py - (py - box.y) * k, w: box.w * k, h: box.h * k });
  };
  const onMove = (e: React.PointerEvent<SVGSVGElement>) => {
    if (!drag.current || !box || !svg.current) return;
    const r = svg.current.getBoundingClientRect();
    const dx = ((e.clientX - drag.current.x) / r.width) * box.w, dy = ((e.clientY - drag.current.y) / r.height) * box.h;
    drag.current = { x: e.clientX, y: e.clientY };
    setView({ ...box, x: box.x - dx, y: box.y - dy });
  };

  const today = localDay(new Date()); // the picker's days are local, not UTC
  const name = (i: number) => s!.nodes.name[i];

  return (
    <ShellFrame>
      <div className="lm">
        <div className="lm-bar">
          <label className="lm-at">
            <span>At</span>
            <input type="date" value={at} max={today} onChange={(e) => setAt(e.target.value)} aria-describedby="lm-at-note" />
          </label>
          <button type="button" className="btn" disabled={!at} onClick={() => setAt("")}>Now</button>
          <span id="lm-at-note" className="lm-note" aria-live="polite">
            {s && !g.error ? <>{n(s.nodes.id.length, "file")} · {n(s.links.from.length, "import link")}{s.at && <> · at commit <code>{s.at.sha.slice(0, 7)}</code></>}</> : null}
            {g.loading && <> · Loading…</>}
          </span>
        </div>
        <div className="lm-main">
          <div className="lm-canvas">
            {g.error ? (
              <div className="lm-msg" role="alert"><b>{g.error.what}</b><p>{g.error.why}. {g.error.try_}.</p></div>
            ) : !current && !wsLoading ? (
              <div className="lm-msg"><b>No workspace yet</b><p>Index a repository with cartograph index, then reload.</p></div>
            ) : !s ? (
              <div className="lm-msg" aria-busy="true">Loading the map…</div>
            ) : !L ? (
              <div className="lm-msg"><b>{size.toLocaleString()} files and links at this date</b><p>This view draws up to {MAX_DRAWN.toLocaleString()}. Switching to the fast renderer, which isn&apos;t in this build yet (FE-08).</p></div>
            ) : s.nodes.id.length === 0 ? (
              <div className="lm-msg"><b>No files at this date</b><p>The history starts later. Pick a later date, or Now.</p></div>
            ) : (
              <svg ref={svg} className="lm-svg" viewBox={`${box!.x} ${box!.y} ${box!.w} ${box!.h}`} role="img" aria-label={`Map of ${s.nodes.id.length} files and ${s.links.from.length} import links`}
                onWheel={onWheel} onPointerDown={(e) => { drag.current = { x: e.clientX, y: e.clientY }; }} onPointerMove={onMove} onPointerUp={() => { drag.current = null; }} onPointerLeave={() => { drag.current = null; }}
                onClick={(e) => { if (e.target === e.currentTarget) select(null); }}>
                {[...L.centres].map(([g, c]) => (
                  <g key={g}><circle className="lm-group" cx={c.x} cy={c.y} r={c.r} /><text className="lm-glabel" x={c.x} y={c.y - c.r + 16 * u} fontSize={11 * u}>{g}</text></g>
                ))}
                <g className={sel === null ? undefined : "lm-dim"}>
                  {s.links.from.map((f, j) => {
                    const a = L.pos[f], b = L.pos[s.links.to[j]];
                    const on = sel !== null && (f === sel || s.links.to[j] === sel);
                    return <line key={s.links.id[j]} className={on ? "lm-link is-on" : "lm-link"} x1={a.x} y1={a.y} x2={b.x} y2={b.y} />;
                  })}
                </g>
                {s.nodes.id.map((id, i) => (
                  <circle key={id} className={i === sel ? "lm-node is-sel" : "lm-node"} cx={L.pos[i].x} cy={L.pos[i].y} r={(i === sel ? 6 : 4) * u}
                    style={{ fill: hue(L.groups.indexOf(L.folder[i])) }} onClick={() => select(i)}>
                    <title>{name(i)}</title>
                  </circle>
                ))}
                {sel !== null && <text className="lm-label" x={L.pos[sel].x + 9 * u} y={L.pos[sel].y + 4 * u} fontSize={12 * u} strokeWidth={3 * u}>{name(sel)}</text>}
              </svg>
            )}
          </div>
          {s && sel !== null && (
            <aside className="lm-panel" aria-label="Selected file">
              <h2>{name(sel).split("/").pop()}</h2>
              <p className="lm-path">{name(sel)}</p>
              <List title="Imports" ids={links.out} name={name} onPick={select} />
              <List title="Imported by" ids={links.in} name={name} onPick={select} />
              <button type="button" className="btn" onClick={() => select(null)}>Close</button>
            </aside>
          )}
        </div>
      </div>
    </ShellFrame>
  );
}

function List({ title, ids, name, onPick }: { title: string; ids: number[]; name: (i: number) => string; onPick: (i: number) => void }) {
  return (
    <section>
      <h3>{title} <span>{ids.length}</span></h3>
      {ids.length === 0 ? <p className="lm-none">None at this date.</p> : (
        <ul>{ids.map((i) => <li key={i}><button type="button" onClick={() => onPick(i)}>{name(i)}</button></li>)}</ul>
      )}
    </section>
  );
}
