"use client";

import { useMemo, useState } from "react";
import { ArrowLeft, ArrowRight, ChevronDown, ChevronUp, GitCompareArrows } from "lucide-react";
import { COMMITS, SERVICES, dayMs, serviceColor } from "@/lib/workspace";
import { DAY, HISTORY_START, REWRITES, dayOf, density, renameDays } from "@/lib/demo/history";
import "./timeline.css";

type Zoom = "year" | "month";
export interface TimelineProps {
  /** The selected day, in ms (12:00 UTC). */
  t: number;
  /** Latest selectable day. */
  today: number;
  onChange: (t: number) => void;
  /** Decision I4: a second handle. `a` is the first date, `b` the date to compare it with. */
  compare?: { a: number; b: number } | null;
  onCompare?: (c: { a: number; b: number } | null) => void;
}

const utc = (y: number, m: number, d: number) => Date.UTC(y, m, d, 12);
const fmt = (ms: number, o: Intl.DateTimeFormatOptions) => new Date(ms).toLocaleDateString("en-GB", { ...o, timeZone: "UTC" });
const clamp = (x: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, x));
const addMonths = (ms: number, n: number) => {
  const d = new Date(ms);
  const last = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + n + 1, 0)).getUTCDate();
  return utc(d.getUTCFullYear(), d.getUTCMonth() + n, Math.min(d.getUTCDate(), last));
};
/** Year zoom shows 12 months ending with t's month. Month zoom shows t's month. */
const windowFor = (zoom: Zoom, t: number) => {
  const d = new Date(t), y = d.getUTCFullYear(), m = d.getUTCMonth();
  return zoom === "year" ? { start: utc(y, m - 11, 1), end: utc(y, m + 1, 0) } : { start: utc(y, m, 1), end: utc(y, m + 1, 0) };
};

/** Overview of the whole history, a zoomed density strip, scrubber, previous/next commit and an optional compare handle. */
export default function Timeline({ t, today, onChange, compare, onCompare }: TimelineProps) {
  const [zoom, setZoom] = useState<Zoom>("year");
  const [win, setWin] = useState(() => windowFor("year", t));
  const [own, setOwn] = useState<{ a: number; b: number } | null>(null);
  const cmp = compare !== undefined ? compare : own;
  const setCmp = (c: { a: number; b: number } | null) => { if (compare === undefined) setOwn(c); onCompare?.(c); };

  // Keep the window around the selected day. Adjusting state during render avoids a flash.
  if (t < win.start || t > win.end) setWin(windowFor(zoom, t));
  const pickZoom = (z: Zoom) => { setZoom(z); setWin(windowFor(z, t)); };

  // ponytail: one coordinate rule for the strip. Each day of the window owns a cell 1/days wide (the bins use the same
  // cells), and a day sits at the centre of its cell. The native range is inset by half a cell, minus half a thumb, so
  // its thumb centre, which travels min..max over width - thumb, lands on those same cell centres at any width.
  const span = win.end - win.start + DAY;
  const days = span / DAY;
  const edge = (x: number) => clamp(((x - win.start) / span) * 100, 0, 100);
  const pct = (x: number) => clamp(((x - win.start + DAY / 2) / span) * 100, 0, 100);
  const move = (x: number) => clamp(x, HISTORY_START, today);
  const go = (x: number) => { const n = move(x); onChange(n); if (cmp) setCmp({ a: n, b: cmp.b }); };
  const goB = (x: number) => cmp && setCmp({ a: cmp.a, b: clamp(x, HISTORY_START, today) });

  const binDays = zoom === "year" ? 7 : 1;
  const bins = useMemo(() => density(win.start, win.end, binDays), [win, binDays]);
  const overview = useMemo(() => density(HISTORY_START, today, "month"), [today]);
  const peak = Math.max(1, ...bins.map((b) => b.total));
  const overPeak = Math.max(1, ...overview.map((b) => b.total));
  const renames = useMemo(() => renameDays(), []);
  const commits = COMMITS.map((c) => ({ ...c, ms: dayMs(c.date) }));
  const here = [...commits].reverse().find((c) => c.ms <= t);
  const prev = [...commits].reverse().find((c) => c.ms < t);
  const next = commits.find((c) => c.ms > t && c.ms <= today);
  const first = HISTORY_START;
  const inWin = (x: number) => x >= win.start && x <= win.end;

  const label = zoom === "year"
    ? [fmt(win.start, { month: "short", year: "numeric" }), fmt(win.end, { month: "short", year: "numeric" })]
    : [fmt(win.start, { day: "numeric", month: "short" }), fmt(win.end, { day: "numeric", month: "short", year: "numeric" })];
  const axis = zoom === "year"
    ? Array.from({ length: 12 }, (_, i) => fmt(addMonths(win.start, i), { month: "short" }))
    : [1, 8, 15, 22, 29].map((d) => String(d));
  const axisPos = (i: number) => (zoom === "year" ? edge(addMonths(win.start, i)) : edge(win.start + (i * 7) * DAY));
  const inset = { ["--half" as string]: `${50 / days}%` };

  const onKey = (cur: number, set: (x: number) => void) => (e: React.KeyboardEvent) => {
    const step: Record<string, number> = {
      ArrowRight: cur + DAY, ArrowUp: cur + DAY, ArrowLeft: cur - DAY, ArrowDown: cur - DAY,
      PageUp: addMonths(cur, 1), PageDown: addMonths(cur, -1), Home: first, End: today,
    };
    if (!(e.key in step)) return;
    e.preventDefault();
    set(step[e.key]);
  };
  const valueText = (x: number) => {
    const c = [...commits].reverse().find((k) => k.ms <= x);
    return `${fmt(x, { weekday: "long", day: "numeric", month: "long", year: "numeric" })}${c ? `, latest commit ${c.sha}` : ""}`;
  };
  const toggleCompare = () => setCmp(cmp ? null : { a: t, b: move(addMonths(t, -1)) });
  const band = cmp ? [Math.min(cmp.a, cmp.b), Math.max(cmp.a, cmp.b)] : null;
  const shiftBy = (n: number) => go(zoom === "year" ? addMonths(t, 12 * n) : addMonths(t, n));

  return (
    <footer className="timeline tl" aria-label="Timeline">
      <div className="tl-range">
        <span className="tl-k">Range</span>
        <b className="tl-label" aria-live="polite">{label[0]} to<br />{label[1]}</b>
        <span className="tl-sub">{zoom === "year" ? "weekly bins" : "daily bins"}</span>
        <div className="tl-shift">
          <button type="button" onClick={() => shiftBy(1)} disabled={t >= today} aria-label={zoom === "year" ? "Next year" : "Next month"}><ChevronUp size={13} /></button>
          <button type="button" onClick={() => shiftBy(-1)} disabled={t <= first} aria-label={zoom === "year" ? "Previous year" : "Previous month"}><ChevronDown size={13} /></button>
        </div>
      </div>

      <div className="tl-steps">
        <div className="tl-btns">
          <button type="button" className="round-btn is-ghost" disabled={!prev} onClick={() => prev && go(prev.ms)} aria-label="Previous commit" title={prev?.message}><ArrowLeft size={18} /></button>
          <button type="button" className="round-btn is-ghost" disabled={!next} onClick={() => next && go(next.ms)} aria-label="Next commit" title={next?.message}><ArrowRight size={18} /></button>
        </div>
        <div className="tl-seg" role="group" aria-label="Zoom">
          <button type="button" aria-pressed={zoom === "year"} onClick={() => pickZoom("year")}>Year</button>
          <button type="button" aria-pressed={zoom === "month"} onClick={() => pickZoom("month")}>Month</button>
        </div>
        <button type="button" className="tl-compare" aria-pressed={!!cmp} onClick={toggleCompare}><GitCompareArrows size={13} aria-hidden /> Compare</button>
      </div>

      <div className="tl-main">
        <div className="tl-over" aria-hidden onPointerDown={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          go(first + clamp((e.clientX - r.left) / r.width, 0, 1) * (today - first));
        }}>
          {overview.map((b) => <span key={b.start} style={{ height: `${Math.max(6, (b.total / overPeak) * 100)}%` }} className={b.start >= win.start - 31 * DAY && b.start <= win.end ? "is-in" : ""} />)}
          <i className="tl-brush" style={{ left: `${((Math.max(win.start, first) - first) / (today - first)) * 100}%`, width: `${((Math.min(win.end, today) - Math.max(win.start, first)) / (today - first)) * 100}%` }} />
          {REWRITES.map((r) => <i key={r.id} className="tl-rewrite" style={{ left: `${((dayOf(r.date) - first) / (today - first)) * 100}%` }} title={`${r.repo} history rewritten ${fmt(dayOf(r.date), { day: "numeric", month: "short", year: "numeric" })}`} />)}
        </div>

        <div className="tl-strip">
          <div className="tl-bubble" style={{ left: `${pct(t)}%` }} aria-hidden>
            {fmt(t, { day: "numeric", month: "short" })}{here && <> · <span className="tl-sha">{here.sha}</span></>}
          </div>
          {cmp && <div className="tl-bubble is-b" style={{ left: `${pct(cmp.b)}%` }} aria-hidden>B · {fmt(cmp.b, { day: "numeric", month: "short", year: "2-digit" })}</div>}
          <div className="tl-bars" aria-hidden>
            {bins.map((b) => (
              <div key={b.start} className={`tl-bin${b.start > t ? " is-after" : ""}${b.start > today ? " is-future" : ""}`} style={{ height: `${(b.total / peak) * 100}%`, width: `${(binDays * DAY / span) * 100}%` }}
                title={`${b.total} commits, ${fmt(b.start, { day: "numeric", month: "short" })}`}>
                {SERVICES.map((s) => b.byRepo[s.id] > 0 && <span key={s.id} style={{ flex: b.byRepo[s.id], background: s.color }} />)}
              </div>
            ))}
            {band && <i className="tl-band" style={{ left: `${pct(band[0])}%`, width: `${pct(band[1]) - pct(band[0])}%` }} />}
            {REWRITES.filter((r) => inWin(dayOf(r.date))).map((r) => (
              <i key={r.id} className="tl-rewrite is-strip" style={{ left: `${pct(dayOf(r.date))}%` }} title={`${r.repo} history rewritten, sync replayed from ${r.mergeBase}`} />
            ))}
            <i className="tl-cursor" style={{ left: `${pct(t)}%` }} />
          </div>
          <div className="tl-marks" aria-hidden>
            {commits.filter((c) => inWin(c.ms)).map((c) => <b key={c.sha} className={`tl-dot${c.ms <= t ? " is-past" : ""}`} style={{ left: `${pct(c.ms)}%`, ["--c" as string]: serviceColor(c.repo) }} title={c.message} />)}
            {renames.filter(inWin).map((r) => <em key={r} className="tl-rename" style={{ left: `${pct(r)}%` }} title="A file was renamed" />)}
          </div>
          <input type="range" className="tl-scrub" style={inset} min={win.start} max={win.end} step={DAY} value={clamp(t, win.start, win.end)}
            onChange={(e) => go(+e.target.value)} onKeyDown={onKey(t, go)}
            aria-label={cmp ? "Date A" : "Date"} aria-valuetext={valueText(t)} />
          {cmp && (
            <input type="range" className="tl-scrub is-b" style={inset} min={win.start} max={win.end} step={DAY} value={clamp(cmp.b, win.start, win.end)}
              onChange={(e) => goB(+e.target.value)} onKeyDown={onKey(cmp.b, goB)}
              aria-label="Date B, to compare with" aria-valuetext={valueText(cmp.b)} />
          )}
          <div className="tl-axis" aria-hidden>{axis.map((m, i) => <span key={i} style={{ left: `${axisPos(i)}%` }}>{m}</span>)}</div>
        </div>
      </div>
    </footer>
  );
}
