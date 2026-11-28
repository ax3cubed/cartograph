"use client";

// A virtualised table (FE-11): only the rows in view are in the DOM, so a list of thousands stays cheap, and the next
// page is asked for when the reader nears the end of the loaded ones (F2). One tab stop for the table: arrows move and
// select, Enter moves focus to the aside, `t` opens the row on the map (the same keys as the other tables).
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { useRouter } from "next/navigation";

const OVERSCAN = 8;
const NEAR_END = 15;
const ASIDE = "view-aside";

export interface VGridProps<T> {
  rows: T[];
  rowId: (r: T) => string;
  label: string;
  /** The grid-template class of the head and every row (`ep-cols`). */
  cols: string;
  head: ReactNode;
  render: (r: T) => ReactNode;
  sel?: string;
  select: (id: string) => void;
  hasMore: boolean;
  more: () => void;
  loading: boolean;
  rowHeight?: number;
  /** Where `t` takes the row. */
  traceHref?: (id: string) => string | undefined;
  /** The viewport's tallest size, in px. */
  maxHeight?: number;
  /** The row's class, for a tone or a column set. */
  rowClass?: string;
  /** Said in place of the table when it has no rows and isn't loading. */
  empty?: ReactNode;
}

export default function VGrid<T>({ rows, rowId, label, cols, head, render, sel, select, hasMore, more, loading, rowHeight = 52, traceHref, maxHeight = 560, rowClass = "", empty }: VGridProps<T>) {
  const box = useRef<HTMLDivElement>(null);
  const router = useRouter();
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(maxHeight);
  const focusId = useRef<string | null>(null);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => setHeight(el.clientHeight || maxHeight);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [maxHeight, rows.length === 0]);

  const first = Math.max(0, Math.floor(top / rowHeight) - OVERSCAN);
  const last = Math.min(rows.length, Math.ceil((top + height) / rowHeight) + OVERSCAN);
  useEffect(() => { if (hasMore && !loading && last >= rows.length - NEAR_END) more(); }, [hasMore, loading, last, rows.length, more]);

  // after a key moved the selection, focus its row once it is in the DOM
  useEffect(() => {
    const id = focusId.current;
    if (!id) return;
    const el = box.current?.querySelector<HTMLElement>(`[data-rid="${CSS.escape(id)}"]`);
    if (el) { el.focus({ preventScroll: true }); focusId.current = null; }
  });

  const ids = rows.map(rowId);
  const selIndex = sel ? ids.indexOf(sel) : -1;
  const tabbable = selIndex >= first && selIndex < last ? selIndex : -1;

  const reveal = useCallback((i: number) => {
    const el = box.current;
    if (!el) return;
    const headH = el.querySelector<HTMLElement>(".vgrid-head")?.offsetHeight ?? 0;
    const y = i * rowHeight;
    if (y < el.scrollTop) el.scrollTop = y;
    else if (y + rowHeight > el.scrollTop + el.clientHeight - headH) el.scrollTop = y + rowHeight - el.clientHeight + headH;
  }, [rowHeight]);

  const onKeyDown = (e: KeyboardEvent) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-rid]");
    if (!row || e.target !== row) return;
    const id = row.dataset.rid!;
    const i = ids.indexOf(id);
    const move = (n: number) => {
      const j = Math.max(0, Math.min(ids.length - 1, n));
      const to = ids[j];
      if (to === undefined) return;
      select(to); focusId.current = to; reveal(j);
    };
    if (e.key === "ArrowDown") { e.preventDefault(); move(i + 1); }
    else if (e.key === "ArrowUp") { e.preventDefault(); move(i - 1); }
    else if (e.key === "PageDown") { e.preventDefault(); move(i + Math.floor(height / rowHeight)); }
    else if (e.key === "PageUp") { e.preventDefault(); move(i - Math.floor(height / rowHeight)); }
    else if (e.key === "Home") { e.preventDefault(); move(0); }
    else if (e.key === "End") { e.preventDefault(); move(ids.length - 1); }
    else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(id); document.getElementById(ASIDE)?.focus(); }
    else if (e.key === "t" && !e.metaKey && !e.ctrlKey) { const h = traceHref?.(id); if (h) { e.preventDefault(); router.push(h); } }
  };

  if (rows.length === 0 && !loading && empty) return <>{empty}</>;
  return (
    <div className="view-scroll">
      <div ref={box} role="grid" aria-label={label} aria-rowcount={rows.length + 1 + (hasMore ? 1 : 0)} aria-busy={loading || undefined}
        className="view-card vgrid" style={{ maxHeight }} tabIndex={tabbable < 0 ? 0 : undefined}
        onScroll={(e) => setTop(e.currentTarget.scrollTop)} onKeyDown={onKeyDown}>
        <div role="row" aria-rowindex={1} className={`view-cols ${cols} view-head-row view-k vgrid-head`}>{head}</div>
        <div role="rowgroup" className="vgrid-body" style={{ height: rows.length * rowHeight }}>
          {rows.slice(first, last).map((r, k) => {
            const id = rowId(r);
            const i = first + k;
            return (
              <div key={id} role="row" data-rid={id} aria-rowindex={i + 2} aria-selected={id === sel}
                tabIndex={i === tabbable ? 0 : -1} className={`view-cols ${cols} view-row ${rowClass}`}
                style={{ position: "absolute", top: i * rowHeight, height: rowHeight }} onClick={() => select(id)}>
                {render(r)}
              </div>
            );
          })}
        </div>
      </div>
      <p className="view-note" role="status" aria-live="polite">
        {loading ? "Loading…" : `${rows.length} loaded${hasMore ? ". Scroll for more." : "."}`}
      </p>
    </div>
  );
}
