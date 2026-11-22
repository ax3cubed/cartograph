"use client";

import { useState } from "react";
import { byId } from "@/lib/workspace";
import { SYMBOLS, SYMBOL_LINKS, symbolById, type Sym } from "@/lib/demo/symbols";
import { serviceHue } from "./domains";
import "./map.css";

const ROW = 30, HEAD = 30, PAD = 12, GAP = 14, SHOW = 3;
type Group = { file: string; syms: { s: Sym; kind: "call" | "type" }[] };

function group(items: { s: Sym; kind: "call" | "type" }[]): Group[] {
  const m = new Map<string, Group>();
  for (const it of items) (m.get(it.s.file) ?? m.set(it.s.file, { file: it.s.file, syms: [] }).get(it.s.file)!).syms.push(it);
  return [...m.values()];
}
export const symbolSides = (id: string) => ({
  calls: group(SYMBOL_LINKS.filter((l) => l.from === id).map((l) => ({ s: symbolById(l.to)!, kind: l.kind }))),
  callers: group(SYMBOL_LINKS.filter((l) => l.to === id).map((l) => ({ s: symbolById(l.from)!, kind: l.kind }))),
});

/** G11: a symbol with what it calls on the left and what calls it on the right, grouped by file. */
export default function SymbolView({ symbolId, onSymbol }: { symbolId: string; onSymbol: (id: string) => void }) {
  const me = symbolById(symbolId) ?? SYMBOLS[0];
  const { calls, callers } = symbolSides(me.id);
  const [more, setMore] = useState<{ calls: boolean; callers: boolean }>({ calls: false, callers: false });
  const CX = 500, CY = 320, W = 250;

  const side = (groups: Group[], x: number, dir: -1 | 1, key: "calls" | "callers") => {
    const shown = more[key] ? groups : groups.slice(0, SHOW);
    const hidden = groups.slice(shown.length);
    let y = 70;
    const els: React.ReactNode[] = [], curves: React.ReactNode[] = [];
    for (const g of shown) {
      const h = PAD * 2 + HEAD + g.syms.length * ROW;
      const f = byId.get(g.file)!;
      els.push(
        <g key={g.file} className="map-sg" transform={`translate(${x},${y})`}>
          <rect width={W} height={h} rx="12" />
          <circle className="map-sg-dot" cx="22" cy={PAD + 14} r="4" style={{ stroke: serviceHue(f.service) }} />
          <text className="map-sg-file" x="34" y={PAD + 18}>{f.id.split("/").slice(-2).join("/")}</text>
          {g.syms.map(({ s, kind }, i) => {
            const sy = PAD + HEAD + i * ROW;
            const cy = y + sy + 14, ex = dir < 0 ? x + W : x, cx = CX + dir * 48;
            curves.push(<path key={s.id} className={`map-sg-edge ${kind === "type" ? "is-type" : ""}`} d={`M${ex},${cy} C${(ex + cx) / 2},${cy} ${(ex + cx) / 2},${CY} ${cx},${CY}`} />);
            return (
              <g key={s.id} className="map-sym" transform={`translate(10,${sy})`} role="button" tabIndex={0} aria-label={`Focus ${s.name}`}
                onClick={() => onSymbol(s.id)} onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), onSymbol(s.id))}>
                <rect width={W - 20} height={ROW - 4} rx="8" />
                <rect className="map-sq" x="10" y={ROW / 2 - 7} width="9" height="9" rx="2" style={{ stroke: serviceHue(f.service) }} />
                <text x="28" y="17">{s.name}{s.kind === "function" ? "()" : ""}</text>
                {kind === "type" && <text className="map-sg-type" x={W - 30} y="17" textAnchor="end">type</text>}
              </g>
            );
          })}
        </g>,
      );
      y += h + GAP;
    }
    if (hidden.length) {
      const n = hidden.reduce((a, g) => a + g.syms.length, 0);
      els.push(
        <g key="more" className="map-sym map-more" transform={`translate(${x},${y})`} role="button" tabIndex={0}
          onClick={() => setMore({ ...more, [key]: true })} onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && setMore({ ...more, [key]: true })}>
          <rect width={W} height="48" rx="12" />
          <text x="14" y="21">Show {n} {key === "calls" ? "calls" : "callers"} in {hidden.length} files</text>
          <text className="map-sg-type" x="14" y="38">{hidden.map((g) => byId.get(g.file)!.name.replace(/\.\w+$/, "")).join(", ")}</text>
        </g>,
      );
    }
    return <>{curves}{els}</>;
  };

  const count = (g: Group[]) => `${g.reduce((a, x) => a + x.syms.length, 0)} in ${g.length} ${g.length === 1 ? "file" : "files"}`;
  return (
    <svg className="graph focus map-symsvg" viewBox="0 0 1000 700" preserveAspectRatio="xMidYMin meet" role="group" aria-label={`${me.name}: what it calls and what calls it`}>
      <text className="map-k-svg" x="40" y="52">Calls · {count(calls)}</text>
      <text className="map-k-svg" x="710" y="52">Called by · {count(callers)}</text>
      {calls.length === 0 && <text className="col-empty" x="165" y="120" textAnchor="middle">Calls nothing else in the index</text>}
      {callers.length === 0 && <text className="col-empty" x="835" y="120" textAnchor="middle">Nothing in the index calls this</text>}
      {side(calls, 40, -1, "calls")}
      {side(callers, 710, 1, "callers")}
      <circle className="halo" cx={CX} cy={CY} r="74" />
      <circle className="core" cx={CX} cy={CY} r="46" />
      <text className="map-core-t" x={CX} y={CY - 2} textAnchor="middle">{me.name}</text>
      <text className="map-core-t" x={CX} y={CY + 14} textAnchor="middle">{me.kind === "function" ? "()" : ""}</text>
    </svg>
  );
}
