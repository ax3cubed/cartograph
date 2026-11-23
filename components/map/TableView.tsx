"use client";

// FE-12: the map as two tables (files, links) over the same slice, filter and date as the map. Real <table>
// semantics; only the rows in view are in the DOM (fixed 32px rows, so the window needs no measuring).
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import type { GraphSlice } from "@/lib/api";
import { fmtNum } from "@/lib/date";
import type { DiffState } from "./levels";
import { sortRows, topFolder, type Dir, type Shown } from "./table";

const ROW = 32;
const EXTRA = 8; // rows drawn past each edge of the window

type Col = { key: string; label: string; num?: boolean; value: (i: number) => string | number; cell: (i: number) => ReactNode };

const CHANGE: Record<DiffState, string> = { added: "Added", removed: "Removed", changed: "Changed", unchanged: "Unchanged" };
const cap = (x: string) => x[0].toUpperCase() + x.slice(1);

export default function TableView({ s, shown: all, deg, sel, onOpen, noun = { one: "file", many: "files" }, diff }: {
  s: GraphSlice; shown: Shown; deg: { inn: number[]; out: number[] }; sel: number | null;
  /** Enter on a row: select that node and move focus to its details. */
  onOpen: (node: number, from: HTMLElement) => void;
  /** What a row is at this level of detail. */
  noun?: { one: string; many: string };
  /** Compare (I4): each node's and link's state, and the difference filter over them. */
  diff?: { node: DiffState[]; link: DiffState[] };
}) {
  const [tab, setTab] = useState<"files" | "links">("files");
  const [only, setOnly] = useState<DiffState | "all">("all");
  const name = (i: number) => s.nodes.name[i];
  const file = noun.one === "file";
  const shown = !diff || only === "all" ? all : { nodes: all.nodes.filter((i) => diff.node[i] === only), links: all.links.filter((j) => diff.link[j] === only) };
  const change = (st?: DiffState) => (st ? <span className={`lm-change is-${st}`}>{CHANGE[st]}</span> : null);
  const files: Col[] = [
    { key: "path", label: cap(noun.one), value: name, cell: (i) => <span className="lm-mono">{name(i)}</span> },
    ...(file ? [{ key: "folder", label: "Folder", value: (i: number) => topFolder(name(i)), cell: (i: number) => topFolder(name(i)) }] : []),
    { key: "out", label: file ? "Imports" : "Uses", num: true, value: (i) => deg.out[i], cell: (i) => deg.out[i] },
    { key: "in", label: file ? "Imported by" : "Used by", num: true, value: (i) => deg.inn[i], cell: (i) => deg.inn[i] },
    ...(diff ? [{ key: "change", label: "Change", value: (i: number) => diff.node[i], cell: (i: number) => change(diff.node[i]) }] : []),
  ];
  const links: Col[] = [
    { key: "from", label: "From", value: (j) => name(s.links.from[j]), cell: (j) => <span className="lm-mono">{name(s.links.from[j])}</span> },
    { key: "to", label: "To", value: (j) => name(s.links.to[j]), cell: (j) => <span className="lm-mono">{name(s.links.to[j])}</span> },
    { key: "kind", label: "Kind", value: (j) => s.links.kind[j], cell: (j) => s.links.kind[j] },
    { key: "level", label: "Confidence", value: (j) => s.links.score[j], cell: (j) => `${s.links.level[j][0].toUpperCase()}${s.links.level[j].slice(1)} · ${s.links.score[j].toFixed(2)}` },
    ...(file ? [] : [{ key: "weight", label: "File links", num: true, value: (j: number) => s.links.weight[j], cell: (j: number) => fmtNum(s.links.weight[j]) }]),
    ...(diff ? [{ key: "change", label: "Change", value: (j: number) => diff.link[j], cell: (j: number) => change(diff.link[j]) }] : []),
  ];
  return (
    <div className="lm-table">
      <div className="lm-trow">
        <div className="lm-seg" role="group" aria-label="Table">
          <button type="button" aria-pressed={tab === "files"} onClick={() => setTab("files")}>{cap(noun.many)} {fmtNum(shown.nodes.length)}</button>
          <button type="button" aria-pressed={tab === "links"} onClick={() => setTab("links")}>Links {fmtNum(shown.links.length)}</button>
        </div>
        {diff && (
          <label className="lm-at">
            <span>Show</span>
            <select value={only} onChange={(e) => setOnly(e.target.value as DiffState | "all")}>
              <option value="all">Everything</option>
              {(Object.keys(CHANGE) as DiffState[]).map((k) => <option key={k} value={k}>{CHANGE[k]} only</option>)}
            </select>
          </label>
        )}
      </div>
      {tab === "files"
        ? <Table key={`files${only}`} label={cap(noun.many)} cols={files} rows={shown.nodes} sel={(i) => i === sel} onEnter={(i, el) => onOpen(i, el)} />
        : <Table key={`links${only}`} label={file ? "Import links" : `Links between ${noun.many}`} cols={links} rows={shown.links} sel={(j) => sel !== null && s.links.from[j] === sel} onEnter={(j, el) => onOpen(s.links.from[j], el)} />}
      <p className="lm-hint">Arrow keys, Page Up, Page Down, Home and End move between rows. Enter opens the {noun.one}&apos;s {file ? "imports" : "links"}.</p>
    </div>
  );
}

function Table({ label, cols, rows, sel, onEnter }: { label: string; cols: Col[]; rows: number[]; sel: (i: number) => boolean; onEnter: (i: number, el: HTMLElement) => void }) {
  const [sort, setSort] = useState<{ key: string; dir: Dir }>({ key: cols[0].key, dir: "ascending" });
  const col = cols.find((c) => c.key === sort.key) ?? cols[0];
  // rows change with the slice and the filter; the columns are rebuilt on every render, so key on the sort only
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const sorted = useMemo(() => sortRows(rows, col.value, sort.dir), [rows, sort.key, sort.dir]);
  const box = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(40 * ROW);
  const [act, setAct] = useState(0); // the row that holds the tab stop
  const focusAct = useRef(false);
  const at = Math.min(act, Math.max(0, sorted.length - 1));

  const first = Math.max(0, Math.floor(top / ROW) - EXTRA);
  const last = Math.min(sorted.length, Math.ceil((top + height) / ROW) + EXTRA);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => {
    if (!focusAct.current) return;
    focusAct.current = false;
    box.current?.querySelector<HTMLElement>(`tr[data-k="${at}"]`)?.focus({ preventScroll: true });
  });

  const move = (k: number) => {
    const el = box.current;
    if (!el || !sorted.length) return;
    const to = Math.max(0, Math.min(sorted.length - 1, k));
    const head = ROW; // the sticky header row covers the top of the box
    let t = el.scrollTop;
    if (to * ROW < t) t = to * ROW;
    else if ((to + 1) * ROW + head > t + el.clientHeight) t = (to + 1) * ROW + head - el.clientHeight;
    el.scrollTop = t;
    setTop(t);
    setAct(to);
    focusAct.current = true;
  };
  const page = Math.max(1, Math.floor(height / ROW) - 2);
  const onKey = (e: KeyboardEvent<HTMLTableRowElement>) => {
    const k = Number(e.currentTarget.dataset.k);
    const go = (n: number) => { e.preventDefault(); move(n); };
    if (e.key === "ArrowDown") go(k + 1);
    else if (e.key === "ArrowUp") go(k - 1);
    else if (e.key === "PageDown") go(k + page);
    else if (e.key === "PageUp") go(k - page);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(sorted.length - 1);
    else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onEnter(sorted[k], e.currentTarget); }
  };
  const actShown = at >= first && at < last;

  return (
    // While the row holding the tab stop is scrolled out of the window, the box takes the stop instead.
    <div ref={box} className="lm-tbox" tabIndex={actShown ? -1 : 0} aria-label={actShown ? undefined : label}
      onScroll={(e) => setTop(e.currentTarget.scrollTop)}
      onFocus={(e) => { if (e.target === e.currentTarget && sorted.length) move(at); }}>
      <table aria-label={label} aria-rowcount={sorted.length + 1}>
        <thead>
          <tr aria-rowindex={1}>
            {cols.map((c) => (
              <th key={c.key} scope="col" aria-sort={c.key === sort.key ? sort.dir : "none"} className={c.num ? "is-num" : undefined}>
                <button type="button" onClick={() => setSort({ key: c.key, dir: c.key === sort.key && sort.dir === "ascending" ? "descending" : "ascending" })}>
                  {c.label}<span aria-hidden>{c.key === sort.key ? (sort.dir === "ascending" ? " ↑" : " ↓") : ""}</span>
                </button>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {first > 0 && <tr aria-hidden="true" style={{ height: first * ROW }} />}
          {sorted.slice(first, last).map((i, n) => {
            const k = first + n;
            return (
              <tr key={i} data-k={k} aria-rowindex={k + 2} aria-current={sel(i) || undefined} tabIndex={k === at ? 0 : -1}
                onKeyDown={onKey} onClick={(e) => { setAct(k); onEnter(i, e.currentTarget); }}>
                {cols.map((c) => <td key={c.key} className={c.num ? "is-num" : undefined}>{c.cell(i)}</td>)}
              </tr>
            );
          })}
          {last < sorted.length && <tr aria-hidden="true" style={{ height: (sorted.length - last) * ROW }} />}
        </tbody>
      </table>
      {sorted.length === 0 && <p className="lm-none lm-tempty">Nothing matches the filter at this date.</p>}
    </div>
  );
}
