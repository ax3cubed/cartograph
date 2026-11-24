"use client";

// FE-09, board G04: an overview strip of all history and a detail strip, binned by range (3 months or less by day,
// 2 years or less by week, more by month) and stacked by repo. Bins come from …/timeline/density, never from raw
// commits: the overview once per generation, the detail strip per range. The knob snaps to the nearest commit,
// found by binary search over a sorted Int32Array of days. Arrows step a commit, Shift plus an arrow a bin.
import { useMemo, useState } from "react";
import { ArrowLeft, ArrowRight, ChevronDown, ChevronUp, GitCompareArrows } from "lucide-react";
import type { TimelineDensity, TimelineMarker } from "@/lib/api";
import { useWsRead } from "@/lib/api/live";
import { fmtDate } from "@/lib/date";
import { DAY, binDays, binFor, nearest } from "./bins";
import "./timeline.css";

export { DAY };

type Zoom = "year" | "month";
export type Tick = { sha: string; ms: number; repo: string; message: string };

export interface TimelineProps {
  ws: string;
  /** The first-parent timeline, oldest first. */
  commits: Tick[];
  /** The selected moment, in ms. */
  t: number;
  /** Latest selectable moment (now). */
  today: number;
  /** A commit was picked (its sha), or now (null). */
  onPick: (sha: string | null) => void;
  /** Decision I4: a second handle. `a` is the main date, `b` the one to compare it with (both ms). */
  compare: { a: number; b: number } | null;
  onCompare: (c: { a: number; b: number } | null) => void;
  /** A repo's hue, as on the map. */
  repoHue: (repo: string, i: number) => string;
}

const utc = (y: number, m: number, d: number) => Date.UTC(y, m, d, 12);
const clamp = (x: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, x));
const addMonths = (ms: number, n: number) => {
  const d = new Date(ms);
  const last = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + n + 1, 0)).getUTCDate();
  return utc(d.getUTCFullYear(), d.getUTCMonth() + n, Math.min(d.getUTCDate(), last));
};
const windowFor = (zoom: Zoom, t: number) => {
  const d = new Date(t), y = d.getUTCFullYear(), m = d.getUTCMonth();
  return zoom === "year" ? { start: utc(y, m - 11, 1), end: utc(y, m + 1, 0) } : { start: utc(y, m, 1), end: utc(y, m + 1, 0) };
};
const iso = (ms: number) => new Date(ms).toISOString().slice(0, 10);

export default function Timeline({ ws, commits, t, today, onPick, compare: cmp, onCompare, repoHue }: TimelineProps) {
  const [zoom, setZoom] = useState<Zoom>("year");
  const [win, setWin] = useState(() => windowFor("year", t));
  const [hidden, setHidden] = useState<string[]>([]);
  // Keep the window around the selected moment. Adjusting state during render avoids a flash.
  if (t < win.start || t > win.end + DAY) setWin(windowFor(zoom, t));
  const pickZoom = (z: Zoom) => { setZoom(z); setWin(windowFor(z, t)); };

  const days = useMemo(() => Int32Array.from(commits, (c) => Math.floor(c.ms / DAY)), [commits]);
  const first = commits.length ? Math.min(commits[0].ms, today) : today - 365 * DAY;
  const span = win.end - win.start + DAY;
  const winDays = Math.round(span / DAY);
  const bin = binFor(winDays);
  const overBin = binFor((today - first) / DAY);
  const detail = useWsRead<TimelineDensity>(ws, "timeline/density", { bin, from: iso(win.start), to: iso(win.end + DAY) });
  const over = useWsRead<TimelineDensity>(ws, "timeline/density", { bin: overBin });
  const markers = useWsRead<TimelineMarker[]>(ws, "timeline/markers").data ?? [];
  const bySha = useMemo(() => new Map(commits.map((c) => [c.sha, c])), [commits]);
  const repos = over.data?.repos ?? detail.data?.repos ?? [];
  const shownRepo = (r: string) => !hidden.includes(r);

  // ponytail: one coordinate rule for the strip. Each day of the window owns a cell 1/days wide, and a day sits at the
  // centre of its cell. The native range is inset by half a cell, so its thumb lands on those same centres.
  const pct = (x: number) => clamp(((x - win.start + DAY / 2) / span) * 100, 0, 100);
  const edge = (x: number) => clamp(((x - win.start) / span) * 100, 0, 100);
  const overPct = (x: number) => clamp(((x - first) / Math.max(DAY, today - first)) * 100, 0, 100);

  const at = nearest(days, t);
  const here = at >= 0 && commits[at].ms <= t + DAY ? commits[at] : undefined;
  const idxOf = (ms: number) => { const i = nearest(days, ms); return i; };
  /** Snap `ms` to a commit and pick it; past the last commit's day is now. */
  const go = (ms: number) => {
    const x = clamp(ms, first, today);
    if (!commits.length || Math.floor(x / DAY) > days[days.length - 1]) { onPick(null); if (cmp) onCompare({ a: today, b: cmp.b }); return; }
    const c = commits[idxOf(x)];
    onPick(c.sha);
    if (cmp) onCompare({ a: c.ms, b: cmp.b });
  };
  const goB = (ms: number) => { if (!cmp) return; const i = idxOf(clamp(ms, first, today)); onCompare({ a: cmp.a, b: i >= 0 ? commits[i].ms : today }); };
  const step = (cur: number, n: number, set: (ms: number) => void) => {
    const i = idxOf(cur);
    const j = clamp(i + n, 0, commits.length - 1);
    if (n > 0 && i === commits.length - 1) set(today + DAY); else if (j >= 0) set(commits[j].ms);
  };
  const prev = at > 0 ? commits[at - 1] : undefined;
  const next = here ? commits[at + 1] : commits[0];

  const onKey = (cur: number, set: (ms: number) => void) => (e: React.KeyboardEvent) => {
    const dir = e.key === "ArrowRight" || e.key === "ArrowUp" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowDown" ? -1 : 0;
    if (dir) { e.preventDefault(); if (e.shiftKey) set(cur + dir * binDays(bin) * DAY); else step(cur, dir, set); return; }
    const to: Record<string, number> = { PageUp: addMonths(cur, 1), PageDown: addMonths(cur, -1), Home: first, End: today + DAY };
    if (!(e.key in to)) return;
    e.preventDefault();
    set(to[e.key]);
  };
  const valueText = (x: number) => {
    const i = idxOf(x);
    const c = i >= 0 ? commits[i] : undefined;
    return x >= today ? "Now" : `${fmtDate(x, "full")}${c ? `, commit ${c.sha.slice(0, 7)}: ${c.message.slice(0, 120)}` : ""}`;
  };
  const label = zoom === "year"
    ? [fmtDate(win.start, "my"), fmtDate(win.end, "my")]
    : [fmtDate(win.start, "dm"), fmtDate(win.end, "dmy")];
  const axis = zoom === "year"
    ? Array.from({ length: 12 }, (_, i) => ({ text: fmtDate(addMonths(win.start, i), "m"), at: edge(addMonths(win.start, i)) }))
    : [1, 8, 15, 22, 29].map((d) => ({ text: String(d), at: edge(win.start + (d - 1) * DAY) }));
  const inset = { ["--half" as string]: `${50 / winDays}%` };
  const toggleCompare = () => onCompare(cmp ? null : { a: t, b: prev?.ms ?? first });
  const band = cmp ? [Math.min(cmp.a, cmp.b), Math.max(cmp.a, cmp.b)] : null;
  const shiftBy = (n: number) => go(zoom === "year" ? addMonths(t, 12 * n) : addMonths(t, n));
  const posOf = (m: TimelineMarker) => bySha.get(m.at.sha)?.ms;

  const stack = (b: TimelineDensity["bins"][number]) => repos.map((r, k) => ({ r, k, n: shownRepo(r) ? b.by_repo[k] ?? 0 : 0 })).filter((x) => x.n > 0);
  const total = (b: TimelineDensity["bins"][number]) => stack(b).reduce((a, x) => a + x.n, 0);
  const bins = detail.data?.bins ?? [];
  const peak = Math.max(1, ...bins.map(total));
  const overBins = over.data?.bins ?? [];
  const overPeak = Math.max(1, ...overBins.map(total));
  const startOf = (b: { start: string }) => Date.parse(b.start);
  const widthOf = (list: { start: string }[], i: number, unit: number) => (i + 1 < list.length ? startOf(list[i + 1]) - startOf(list[i]) : binDays(unit === 0 ? bin : overBin) * DAY);
  const inWin = (x: number) => x >= win.start && x < win.end + DAY;
  const dots = commits.filter((c) => inWin(c.ms));

  return (
    <footer className="timeline tl" aria-label="Timeline">
      <div className="tl-range">
        <span className="tl-k">Range</span>
        <b className="tl-label" aria-live="polite">{label[0]} to<br />{label[1]}</b>
        <span className="tl-sub">{bin === "day" ? "daily" : bin === "week" ? "weekly" : "monthly"} bins</span>
        <div className="tl-shift">
          <button type="button" onClick={() => shiftBy(1)} disabled={t >= today} aria-label={zoom === "year" ? "Next year" : "Next month"}><ChevronUp size={13} /></button>
          <button type="button" onClick={() => shiftBy(-1)} disabled={t <= first} aria-label={zoom === "year" ? "Previous year" : "Previous month"}><ChevronDown size={13} /></button>
        </div>
      </div>

      <div className="tl-steps">
        <div className="tl-btns">
          <button type="button" className="round-btn is-ghost" disabled={!prev} onClick={() => prev && go(prev.ms)} aria-label="Previous commit"><ArrowLeft size={18} /></button>
          <button type="button" className="round-btn is-ghost" disabled={!next} onClick={() => next && go(next.ms)} aria-label="Next commit"><ArrowRight size={18} /></button>
        </div>
        <div className="tl-seg" role="group" aria-label="Zoom">
          <button type="button" aria-pressed={zoom === "year"} onClick={() => pickZoom("year")}>Year</button>
          <button type="button" aria-pressed={zoom === "month"} onClick={() => pickZoom("month")}>Month</button>
        </div>
        <button type="button" className="tl-compare" aria-pressed={!!cmp} onClick={toggleCompare}><GitCompareArrows size={13} aria-hidden /> Compare</button>
      </div>

      <div className="tl-main">
        <div className="tl-repos" role="group" aria-label="Repositories shown in the strip">
          {repos.length > 1 && repos.map((r, k) => (
            <button key={r} type="button" aria-pressed={shownRepo(r)} onClick={() => setHidden(shownRepo(r) ? [...hidden, r] : hidden.filter((x) => x !== r))}>
              <i style={{ background: repoHue(r, k) }} aria-hidden />{r}
            </button>
          ))}
        </div>
        <div className="tl-over" aria-hidden onPointerDown={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          go(first + clamp((e.clientX - r.left) / r.width, 0, 1) * (today - first));
        }}>
          {overBins.map((b, i) => (
            <span key={b.start} style={{ left: `${overPct(startOf(b))}%`, width: `${Math.max(0.2, (widthOf(overBins, i, 1) / Math.max(DAY, today - first)) * 100)}%`, height: `${Math.max(6, (total(b) / overPeak) * 100)}%` }}
              className={startOf(b) + widthOf(overBins, i, 1) > win.start && startOf(b) <= win.end ? "is-in" : ""} />
          ))}
          <i className="tl-brush" style={{ left: `${overPct(Math.max(win.start, first))}%`, width: `${Math.max(0.5, overPct(Math.min(win.end, today)) - overPct(Math.max(win.start, first)))}%` }} />
          {markers.filter((m) => m.kind === "rewrite" && posOf(m) !== undefined).map((m, i) => <i key={i} className="tl-rewrite" style={{ left: `${overPct(posOf(m)!)}%` }} title={m.label} />)}
        </div>

        <div className="tl-strip">
          <div className="tl-bubble" style={{ left: `${pct(Math.min(t, win.end))}%` }} aria-hidden>
            {t >= today ? "Now" : fmtDate(t, "dm")}{here && t < today && <> · <span className="tl-sha">{here.sha.slice(0, 7)}</span></>}
          </div>
          {cmp && <div className="tl-bubble is-b" style={{ left: `${pct(cmp.b)}%` }} aria-hidden>B · {fmtDate(cmp.b, "dmy")}</div>}
          <div className="tl-bars" aria-hidden>
            {bins.map((b, i) => (
              <div key={b.start} className={`tl-bin${startOf(b) > t ? " is-after" : ""}`}
                style={{ position: "absolute", bottom: 0, left: `${edge(startOf(b))}%`, height: `${(total(b) / peak) * 100}%`, width: `${(widthOf(bins, i, 0) / span) * 100}%` }}>
                {stack(b).map((x) => <span key={x.r} style={{ flex: x.n, background: repoHue(x.r, x.k) }} />)}
              </div>
            ))}
            {band && <i className="tl-band" style={{ left: `${pct(band[0])}%`, width: `${pct(band[1]) - pct(band[0])}%` }} />}
            {markers.filter((m) => m.kind === "rewrite" && posOf(m) !== undefined && inWin(posOf(m)!)).map((m, i) => (
              <i key={i} className="tl-rewrite is-strip" style={{ left: `${pct(posOf(m)!)}%` }} title={m.label} />
            ))}
            <i className="tl-cursor" style={{ left: `${pct(Math.min(t, win.end))}%` }} />
          </div>
          <div className="tl-marks" aria-hidden>
            {dots.length <= 400 && dots.map((c) => <b key={c.sha} className={`tl-dot${c.ms <= t ? " is-past" : ""}`} style={{ left: `${pct(c.ms)}%`, ["--c" as string]: repoHue(c.repo, Math.max(0, repos.indexOf(c.repo))) }} />)}
            {markers.filter((m) => m.kind === "rename" && posOf(m) !== undefined && inWin(posOf(m)!)).map((m, i) => <em key={i} className="tl-rename" style={{ left: `${pct(posOf(m)!)}%` }} title={m.label} />)}
          </div>
          <input type="range" className="tl-scrub" style={inset} min={win.start} max={win.end} step={DAY} value={clamp(t, win.start, win.end)}
            onChange={(e) => go(+e.target.value)} onKeyDown={onKey(t, go)}
            aria-label={cmp ? "Date A" : "Date"} aria-valuetext={valueText(t)} />
          {cmp && (
            <input type="range" className="tl-scrub is-b" style={inset} min={win.start} max={win.end} step={DAY} value={clamp(cmp.b, win.start, win.end)}
              onChange={(e) => goB(+e.target.value)} onKeyDown={onKey(cmp.b, goB)}
              aria-label="Date B, to compare with" aria-valuetext={valueText(cmp.b)} />
          )}
          <div className="tl-axis" aria-hidden>{axis.map((m, i) => <span key={i} style={{ left: `${m.at}%` }}>{m.text}</span>)}</div>
        </div>
      </div>
    </footer>
  );
}
