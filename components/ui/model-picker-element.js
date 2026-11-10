/* <model-picker> — dark "reasoning mixer" model picker (vendored web component, plain JS on purpose).
   Changes from the supplied source, everything else is as given:
   - rows come from the `models` property ([{ id, name, color? }]) instead of a hardcoded list,
     so any model configured in Settings can appear;
   - the hardcoded "GPT 5.6" family prefix is gone and the `${FALY}` typo (a ReferenceError) is fixed;
   - `inline` attribute: the trigger sizes to its content and the panel anchors to its right edge,
     so it can sit inside a composer; label column width is the `--lw` custom property.
   Attributes: model="<id>", tier="0..4", ultra (boolean), open (boolean), inline (boolean).
   Emits: "change" { model, tierIndex, tier, ultra, label }. */

const DEFAULT_MODELS = [
  { id: 'sol', name: 'Sol' },
  { id: 'terra', name: 'Terra' },
  { id: 'luna', name: 'Luna' },
];
const PALETTE = ['#F59F4F', '#5FE893', '#8E64F0', '#4FC3E8', '#F0648C', '#E8D04F'];
const TIERS = ['Low', 'Medium', 'High', 'Extra high', 'Max'];
const DEFAULT_TIER = 2;

const rnd = (a, b) => a + Math.random() * (b - a);
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

const sparkleSpans = (n) => Array.from({ length: n }, () =>
  `<span style="--sx:${rnd(-1, 99).toFixed(1)}%;--sy:${rnd(-10, 102).toFixed(1)}%;--ss:${rnd(1.2, 3.4).toFixed(1)}px;--sd:${rnd(1.3, 2.6).toFixed(2)}s;--sdel:${rnd(0, 2.2).toFixed(2)}s"></span>`).join('');

/* Dashed zigzag rails traced from the reference video: two per side (upper +
   lower), each a faint full-path ghost with a bright dash pattern marching
   down it. Geometry in viewBox px: amplitude 5, bends every 8, 4 legs (~38 long). */
const zigPts = (x0, y0, dx, legs, step) => {
  const p = [`${x0},${y0}`];
  for (let i = 1; i <= legs; i++) p.push(`${i % 2 ? x0 - dx : x0},${y0 + i * step}`);
  return p.join(' ');
};
const ZIGZAGS = [           // [points, marchDurS, delayS]
  [zigPts(10.5, 30, 5, 4, 8), 0.26, 0],
  [zigPts(10.5, 110, 5, 4, 8), 0.30, -0.12],
  [zigPts(51.5, 28, -5, 4, 8), 0.28, -0.05],
  [zigPts(51.5, 108, -5, 4, 8), 0.32, -0.2],
];
const tickLines = () => ZIGZAGS.map(([pts, dur, del]) =>
  `<polyline class="ghost" points="${pts}" pathLength="38"/>` +
  `<polyline class="run" points="${pts}" pathLength="38" style="--ctw:${dur}s;--cdel:${del}s"/>`
).join('');

const rowsHTML = (models) => models.map((m, mi) => {
  const c = esc(m.color || PALETTE[mi % PALETTE.length]);
  return `
          <div class="row" data-m="${esc(m.id)}" style="--c:${c};--ga:color-mix(in oklab, ${c} 90%, white);--gb:${c}">
            <span class="mlabel" title="${esc(m.id)}">${esc(m.name)}</span>
            <div class="track">
              <div class="fill"><div class="sparkles" aria-hidden="true">${sparkleSpans(11)}</div></div>
              ${TIERS.map((t, i) => `<button class="cell" role="radio" aria-checked="false" tabindex="-1" data-i="${i}" aria-label="${esc(m.name)} — ${t}"></button>`).join('')}
              <div class="thumb" aria-hidden="true"></div>
            </div>
          </div>`;
}).join('');

const bolt = `<svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true"><path fill="currentColor" d="M13.2 2 4.9 13.4h5.2L8.9 22l8.3-11.4H12z"/></svg>`;
const chevron = `<svg class="chev-ic" viewBox="0 0 24 24" width="13" height="13" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" d="m6 9.5 6 6 6-6"/></svg>`;
const html = `
<div class="root">
  <div class="bar">
    <button class="pill" aria-haspopup="dialog" aria-expanded="false" aria-controls="mp-panel" aria-label="Model">
      ${bolt}<span class="labelwrap t-text-swap"></span>${chevron}
    </button>
  </div>
  <div class="panel t-dropdown" id="mp-panel" data-origin="bottom-center" role="dialog" aria-label="Model and reasoning effort" inert>
    <div class="panel-inner">
      <div class="grid-zone">
        <div class="headers" aria-hidden="true"><span class="hspace"></span>${TIERS.map(t => `<span>${t}</span>`).join('')}</div>
        <div class="matrix" role="radiogroup" aria-label="Model and reasoning effort"></div>
      </div>
      <div class="lever-zone">
        <span class="ultra-label">ULTRA</span>
        <button class="lever" type="button" role="switch" aria-checked="false" aria-label="Ultra mode">
          <span class="trough" aria-hidden="true"></span>
          <span class="groove" aria-hidden="true"></span>
          <span class="tline" aria-hidden="true"></span>
          <span class="stem" aria-hidden="true"></span>
          <span class="ball" aria-hidden="true"></span>
          <svg class="confetti" viewBox="0 0 62 152" preserveAspectRatio="none" aria-hidden="true">${tickLines()}</svg>
        </button>
      </div>
    </div>
  </div>
</div>`;

const css = `
:host {
  /* motion tokens (transitions.dev scale) */
  --duration-quick: 150ms;
  --duration-fast: 250ms;
  --ease-smooth-out: cubic-bezier(0.22, 1, 0.36, 1);
  --dropdown-open-dur: 250ms;
  --dropdown-close-dur: 150ms;
  --dropdown-pre-scale: 0.97;
  --dropdown-closing-scale: 0.99;
  --dropdown-ease: var(--ease-smooth-out);
  --text-swap-dur: 150ms;
  --text-swap-translate-y: 4px;
  --text-swap-blur: 2px;
  --text-swap-ease: ease-in-out;
  /* gravity curve for the ball drop: bezier fallback, linear() bounce where supported */
  --drop: cubic-bezier(0.5, 0, 0.9, 0.6);
  /* DNA palette */
  --page: #101010;
  --panel: #1B1B1B;
  --sunken: #232323;
  --well: #2E2E2E;
  --dot: #515151;
  --txt-dim: #8A8A8A;
  --txt-hdr: #9E9E9E;
  --txt-label: #C9CAC5;
  --thumb: #FDFDFD;
  --ultra: #9D74F7;
  --ball-red: #E13753;
  --speckle:
    radial-gradient(rgba(255,255,255,.075) 1.5px, transparent 2px),
    radial-gradient(rgba(255,255,255,.05) 1.3px, transparent 1.8px),
    radial-gradient(rgba(0,0,0,.20) 1.5px, transparent 2px);
  --speckle-size: 29px 27px, 21px 25px, 35px 31px;
  --speckle-pos: 4px 6px, 14px 18px, 9px 2px;
  --lw: 62px; /* model label column */

  display: block;
  width: min(500px, 100%);
  font: 400 15px/1.25 -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, "Segoe UI", Roboto, sans-serif;
  -webkit-font-smoothing: antialiased;
  color: var(--txt-label);
}
/* inline: lives inside a composer; trigger hugs its label, panel hangs off the trigger's right edge */
:host([inline]) { display: inline-block; width: auto; --lw: 132px; }
:host([inline]) .panel { left: auto; right: var(--mp-panel-right, 0px); width: min(580px, calc(100vw - 32px)); }
:host([inline]) .t-dropdown[data-origin="bottom-center"] { transform-origin: bottom right; }
:host([inline]) .mlabel { font-size: 13px; }
:host([inline]) .pill { height: 36px; }
@supports (transition-timing-function: linear(0, 1)) {
  :host {
    --drop: linear(0, 0.0031 3.8%, 0.0123 7.7%, 0.0276 11.5%, 0.0491 15.4%, 0.0767 19.2%, 0.1104 23.1%, 0.1503 26.9%, 0.1963 30.8%, 0.2485 34.6%, 0.3067 38.5%, 0.3712 42.3%, 0.4417 46.2%, 0.5184 50%, 0.6012 53.8%, 0.6902 57.7%, 0.7853 61.5%, 0.8865 65.4%, 0.9939 69.2%, 0.9797 73.1%, 0.9642 76.9%, 0.9548 80.8%, 0.9516 84.6%, 0.9545 88.5%, 0.9635 92.3%, 0.9787 96.2%, 1 100%);
  }
}
*, *::before, *::after { box-sizing: border-box; }
button { font: inherit; color: inherit; background: none; border: 0; padding: 0; cursor: pointer; }
button:focus-visible { outline: 2px solid rgba(157,116,247,.8); outline-offset: 2px; border-radius: 8px; }

.root { position: relative; }
.no-anim, .no-anim * { transition: none !important; }

/* ── popover (transitions.dev menu-dropdown, verbatim mechanics) ── */
.t-dropdown {
  transform-origin: top left;
  transform: scale(var(--dropdown-pre-scale));
  opacity: 0;
  pointer-events: none;
  visibility: hidden;
  transition:
    transform var(--dropdown-open-dur) var(--dropdown-ease),
    opacity   var(--dropdown-open-dur) var(--dropdown-ease);
  will-change: transform, opacity;
}
.t-dropdown[data-origin="bottom-center"] { transform-origin: bottom center; }
.t-dropdown.is-open { transform: scale(1); opacity: 1; pointer-events: auto; visibility: visible; }
.t-dropdown.is-closing {
  transform: scale(var(--dropdown-closing-scale));
  opacity: 0;
  pointer-events: none;
  visibility: visible;
  transition:
    transform var(--dropdown-close-dur) var(--dropdown-ease),
    opacity   var(--dropdown-close-dur) var(--dropdown-ease);
}

.panel {
  position: absolute;
  bottom: calc(100% + 10px);
  left: 0; right: 0;
  z-index: 20;
  background: var(--panel);
  border: 1px solid rgba(255,255,255,.06);
  border-radius: 18px;
  box-shadow: 0 30px 60px -20px rgba(0,0,0,.7), 0 10px 24px rgba(0,0,0,.5);
  transition-property: transform, opacity, box-shadow, border-color;
  user-select: none;
  -webkit-user-select: none;
}
.root.ultra .panel {
  border-color: rgba(157,116,247,.32);
  box-shadow: 0 30px 60px -20px rgba(0,0,0,.7), 0 10px 24px rgba(0,0,0,.5),
    0 0 36px -6px rgba(157,116,247,.35), 0 0 0 1px rgba(157,116,247,.12);
}
.panel-inner { display: flex; gap: 12px; padding: 12px 14px 14px 16px; }
.panel-inner.pulse { animation: panel-pulse 320ms var(--ease-smooth-out); }
@keyframes panel-pulse { 0% { transform: scale(1); } 38% { transform: scale(1.014); } 100% { transform: scale(1); } }

/* ── grid ── */
.grid-zone { flex: 1; min-width: 0; }
.headers { display: flex; align-items: center; height: 30px; font-size: 12.5px; color: var(--txt-hdr); }
.headers .hspace { width: var(--lw); flex: none; }
.headers span:not(.hspace) { flex: 1; text-align: center; white-space: nowrap; }
.matrix { position: relative; padding: 7px 0; }
.well {
  position: absolute; left: var(--lw); right: 0; top: 0; bottom: 0;
  background: var(--well);
  border-radius: 20px;
  box-shadow: inset 0 1px 3px rgba(0,0,0,.4), inset 0 0 0 1px rgba(255,255,255,.025);
}
.row { display: flex; align-items: center; height: 45px; position: relative; }
.mlabel {
  width: var(--lw); flex: none; padding-right: 8px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-size: 16px; color: var(--txt-label);
  transition: color 200ms ease;
}
.row.active .mlabel { color: var(--c); }
.root.ultra .row.active .mlabel { color: #A47DF8; }

.track { flex: 1; position: relative; height: 100%; display: flex; touch-action: pan-y pinch-zoom; }
.cell { position: relative; flex: 1; height: 100%; z-index: 2; border-radius: 12px; cursor: pointer; }
.cell::before {
  content: ""; position: absolute; left: 50%; top: 50%;
  width: 5px; height: 5px; margin: -2.5px 0 0 -2.5px; border-radius: 50%;
  background: var(--dot);
  transition: transform 120ms ease, background-color 120ms ease;
}
.cell:hover::before, .cell:focus-visible::before { background: var(--c); transform: scale(1.7); }
.root.ultra .row.active .cell:hover::before { background: #C4ADF9; }
.cell:focus-visible { outline: none; z-index: 4; }
.cell:focus-visible::after {
  content: ""; position: absolute; left: 50%; top: 50%; width: 22px; height: 22px;
  margin: -11px 0 0 -11px; border-radius: 50%;
  box-shadow: 0 0 0 2px rgba(255,255,255,.4);
}

.fill {
  position: absolute; left: 8px; top: 50%;
  height: 28px; transform: translateY(-50%);
  width: calc(20% * (var(--i, 2) + 0.5) + 8px);
  border-radius: 999px;
  background: linear-gradient(180deg, var(--ga), var(--gb));
  box-shadow: 0 3px 10px rgba(0,0,0,.30);
  opacity: 1; z-index: 1;
  transition: width 260ms var(--ease-smooth-out), opacity 150ms ease-out, box-shadow 300ms ease;
}
.fill::after {  /* violet skin: gradients can't transition, opacity can */
  content: ""; position: absolute; inset: 0; border-radius: inherit; z-index: 0;
  background: linear-gradient(180deg, #AE87F8, #9B6DF4);
  opacity: 0; transition: opacity 220ms ease;
}
/* Keep the violet skin on hidden/collapsing fills while ULTRA is active.
   Otherwise the old row briefly reveals its model color after losing .active. */
.root.ultra .fill::after { opacity: 1; }
.row:not(.active) .fill { width: 40px; opacity: 0; transition: width 150ms ease-in, opacity 130ms ease-in; }
.root.ultra .row.active .fill {
  box-shadow: 0 3px 10px rgba(0,0,0,.30), 0 0 18px rgba(157,116,247,.42),
    0 0 46px -6px rgba(157,116,247,.35);
}
.thumb {
  position: absolute; top: 50%; z-index: 3;
  width: 32px; height: 32px; margin-top: -16px;
  left: calc(20% * (var(--i, 2) + 0.5) - 16px);
  border-radius: 50%;
  background: radial-gradient(circle at 38% 32%, #FFFFFF, #F4F4F4 62%, #E9E9E9);
  box-shadow: 0 1px 2px rgba(0,0,0,.35), 0 4px 9px rgba(0,0,0,.35),
    0 0 12px 4px rgba(0,0,0,.20);
  cursor: grab;
  transition: left 260ms var(--ease-smooth-out), opacity 130ms ease-in, transform 130ms ease-in;
  touch-action: none;
}
/* parked at the collapsed fill's leading edge so activation moves fill and
   thumb by the same delta — identical spring overshoot, edges stay locked */
.row:not(.active) .thumb { opacity: 0; transform: scale(.8); pointer-events: none; left: 16px; }
/* while scrubbing, one --x variable drives both boxes — their edges are
   derived from the same value and structurally cannot separate.
   .flowing = same geometry with transitions kept, so the bar glides into a
   newly-entered row (cross-row drag) before locking to 1:1 tracking */
.track.dragging .thumb, .track.flowing .thumb { cursor: grabbing; left: calc(var(--x, 50%) - 16px); }
.track.dragging .fill, .track.flowing .fill { width: calc(var(--x, 50%) + 8px); }
.track.dragging .thumb { transition: none; }
.track.dragging .fill { transition: opacity 150ms ease-out, box-shadow 300ms ease; }

/* sparkles inside ULTRA fill */
.sparkles { position: absolute; inset: -6px -2px; z-index: 1; opacity: 0; transition: opacity 250ms ease; }
.sparkles span {
  position: absolute; left: var(--sx); top: var(--sy);
  width: var(--ss); height: var(--ss); border-radius: 50%;
  background: #fff;
  opacity: 0;
  animation: twinkle var(--sd) ease-in-out var(--sdel) infinite paused;
}
.sparkles span:nth-child(3n) { filter: blur(.6px); }
.root.ultra .row.active .sparkles { opacity: 1; }
.root.ultra .row.active .sparkles span { animation-play-state: running; }
@keyframes twinkle {
  0%, 100% { opacity: 0; transform: translate(0, 0) scale(.6); }
  45% { opacity: .95; transform: translate(2px, -1.5px) scale(1); }
}

/* ── ULTRA lever ── */
.lever-zone { width: 86px; flex: none; display: flex; flex-direction: column; align-items: center; padding-top: 4px; }
.ultra-label {
  font-size: 11px; font-weight: 700; letter-spacing: .18em;
  color: #8F8F8F; padding-left: .18em;
  transition: color 300ms ease, text-shadow 300ms ease;
  will-change: transform;
}
.root.ultra .ultra-label {
  color: #BFA2FC; text-shadow: 0 0 14px rgba(157,116,247,.85);
  animation: label-flicker 480ms linear both;  /* neon-tube ignition stutter (from 24fps frames) */
}
@keyframes label-flicker {
  0%, 8%    { color: #9A93A8; text-shadow: 0 0 0 rgba(157,116,247,0); }
  9%, 17%   { color: #E2D8FE; text-shadow: 0 0 18px rgba(157,116,247,.95), 0 0 4px rgba(205,180,255,.9); }
  18%, 26%  { color: #6F5F9A; text-shadow: 0 0 3px rgba(157,116,247,.2); }
  27%, 38%  { color: #CDBBFB; text-shadow: 0 0 14px rgba(157,116,247,.85); }
  39%, 47%  { color: #8E79C4; text-shadow: 0 0 6px rgba(157,116,247,.4); }
  48%, 100% { color: #BFA2FC; text-shadow: 0 0 14px rgba(157,116,247,.85); }
}

.lever { position: relative; flex: 1; width: 62px; margin-top: 8px; min-height: 130px; touch-action: pan-y pinch-zoom; cursor: grab; border-radius: 32px; }
.lever:active { cursor: grabbing; }
.lever:focus-visible { outline: 2px solid rgba(157,116,247,.8); outline-offset: 3px; border-radius: 32px; }
.lever::after {  /* crisp violet outline that materializes in ULTRA */
  content: ""; position: absolute; inset: -4px; border-radius: 35px;
  border: 1px solid rgba(157,116,247,.55);
  box-shadow: 0 0 10px -3px rgba(157,116,247,.45);
  opacity: 0; transition: opacity 300ms ease; pointer-events: none;
}
.root.ultra .lever::after { opacity: 1; }
.trough {
  position: absolute; inset: 0; border-radius: 32px;
  background: var(--sunken);
  background-image: var(--speckle);
  background-size: var(--speckle-size);
  background-position: var(--speckle-pos);
  box-shadow: inset 0 2px 5px rgba(0,0,0,.5), inset 0 -1px 2px rgba(255,255,255,.03);
}
.groove {  /* recessed channel the ball travels in */
  position: absolute; left: 50%; width: 26px; margin-left: -13px;
  top: 8px; bottom: 8px; border-radius: 13px;
  background: #161616;
  box-shadow: inset 0 2px 4px rgba(0,0,0,.6), inset 0 0 0 1px rgba(0,0,0,.35),
    inset 0 -1px 2px rgba(255,255,255,.02);
}
.tline {
  position: absolute; left: 50%; width: 4px; margin-left: -2px;
  top: 26px; bottom: 24px; border-radius: 3px;
  background: linear-gradient(90deg, #7E7E7E, #C9C9C9 45%, #8F8F8F);
  box-shadow: 0 1px 2px rgba(0,0,0,.5);
}
.stem {  /* the track itself ignites: runs from the track top down to the ball's center */
  position: absolute; left: 50%; width: 5px; margin-left: -2.5px;
  top: 26px; height: calc((100% - 56px) * var(--p, 0) + 2px);
  border-radius: 4px;
  background: linear-gradient(180deg, #EFE7FE, #C9AFF9 30%, #AC88F5);
  box-shadow: 0 0 12px 2px rgba(157,116,247,.65), 0 0 30px 6px rgba(157,116,247,.25);
  opacity: 0;
  transition: height 260ms var(--ease-smooth-out), opacity 150ms ease;
}
.ball {
  position: absolute; left: 50%; margin-left: -17px;
  width: 34px; height: 34px; border-radius: 50%;
  top: calc(10px + (100% - 56px) * var(--p, 0));
  background: radial-gradient(circle at 32% 26%, #FBC7C4, #F2687B 30%, var(--ball-red) 58%, #AC2141);
  box-shadow: 0 3px 7px rgba(0,0,0,.5), inset 0 -3px 6px rgba(100,8,28,.35);
  transition: top 260ms var(--ease-smooth-out), transform 200ms ease, box-shadow 350ms ease;
  touch-action: none;
}
.ball::after {  /* violet skin cross-fades in when lit */
  content: ""; position: absolute; inset: 0; border-radius: 50%;
  background: radial-gradient(circle at 34% 28%, #F9F6FF, #CDB6F9 36%, #A98BF4 66%, #9070EA);
  opacity: 0; transition: opacity 180ms ease;
}
.lever.lit .stem { opacity: 1; }
.lever.lit .ball::after { opacity: 1; }
.lever.dropping .ball { transition: top 220ms var(--drop), transform 200ms ease, box-shadow 350ms ease; }
.lever.dropping .stem { transition: height 220ms var(--drop), opacity 150ms ease; }
.lever.dragging .ball, .lever.dragging .stem { transition: none; }
.root.ultra .settled .ball {  /* landing bloom: subtle swell, the glow does the talking */
  transform: scale(1.06);
  transition: top 260ms var(--ease-smooth-out), transform 280ms var(--ease-smooth-out),
              box-shadow 420ms var(--ease-smooth-out) 40ms;
  box-shadow: 0 3px 8px rgba(0,0,0,.4), 0 0 22px 5px rgba(157,116,247,.55),
    0 0 55px 14px rgba(157,116,247,.28);
}
.root.ultra .trough {  /* faint violet ambience inside the tube while engaged */
  box-shadow: inset 0 2px 5px rgba(0,0,0,.5), inset 0 0 14px -6px rgba(157,116,247,.18);
}

/* zigzag energy rails: faint full path + bright 4.5/4.5 dash pattern marching
   along it (pathLength=38 keeps dash units ≈ px; one 9-unit period per lap) */
.confetti { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; pointer-events: none; }
.confetti polyline { fill: none; stroke-width: 1.2; stroke-linecap: round; stroke-linejoin: round; }
.confetti .ghost {
  stroke: rgba(157,116,247,.20);
  opacity: 0; transition: opacity 250ms ease;
}
.confetti .run {
  stroke: #B49CF6;
  stroke-dasharray: 4.5 4.5;
  stroke-dashoffset: 9;
  opacity: 0; transition: opacity 200ms ease;
}
.root.ultra .confetti .ghost { opacity: 1; }
.root.ultra .confetti .run { opacity: 1; animation: dash-run var(--ctw) linear var(--cdel) infinite; }
@keyframes dash-run { from { stroke-dashoffset: 9; } to { stroke-dashoffset: 0; } }

/* ── trigger ── */
.bar { display: flex; justify-content: center; }
.pill {
  display: flex; align-items: center; gap: 8px;
  height: 40px; padding: 0 13px 0 14px;
  border-radius: 999px;
  background: var(--sunken);
  color: #F2F2F2;
  white-space: nowrap;
  transition: background-color 140ms ease, transform 120ms ease;
}
.pill:hover { background: #2A2A2A; }
.pill:active { transform: scale(.97); }
.labelwrap b {
  font-weight: 600; display: inline-block; vertical-align: bottom;
  max-width: 18ch; overflow: hidden; text-overflow: ellipsis;
}
.labelwrap .tier { color: var(--txt-dim); margin-left: 5px; font-weight: 400; }
.chev-ic { color: #9A9A9A; transition: transform 200ms var(--ease-smooth-out); }
.root.open .chev-ic { transform: rotate(180deg); }

/* ── text swap (transitions.dev, verbatim) ── */
.t-text-swap {
  display: inline-block;
  transform: translateY(0);
  filter: blur(0);
  opacity: 1;
  transition:
    transform var(--text-swap-dur) var(--text-swap-ease),
    filter    var(--text-swap-dur) var(--text-swap-ease),
    opacity   var(--text-swap-dur) var(--text-swap-ease);
  will-change: transform, filter, opacity;
}
.t-text-swap.is-exit {
  transform: translateY(calc(var(--text-swap-translate-y) * -1));
  filter: blur(var(--text-swap-blur));
  opacity: 0;
}
.t-text-swap.is-enter-start {
  transform: translateY(var(--text-swap-translate-y));
  filter: blur(var(--text-swap-blur));
  opacity: 0;
  transition: none;
}

@media (prefers-reduced-motion: reduce) {
  .t-dropdown, .t-text-swap, .fill, .fill::after, .thumb, .ball, .ball::after, .stem,
  .chev-ic, .cell::before, .pill, .lever::after, .sparkles, .ultra-label, .mlabel, .confetti polyline { transition: none !important; }
  .sparkles span, .confetti polyline, .panel-inner.pulse,
  .root.ultra .ultra-label { animation: none !important; }
  .root.ultra .confetti polyline { opacity: .9; stroke-dashoffset: 0; }
  .t-text-swap.is-exit, .t-text-swap.is-enter-start {
    transform: none; filter: none; opacity: 1;
  }
}
`;

class ModelPicker extends HTMLElement {
  static observedAttributes = ['model', 'tier', 'ultra', 'open'];

  #models = DEFAULT_MODELS;
  #model = DEFAULT_MODELS[0].id;
  #tier = DEFAULT_TIER;
  #ultra = false;
  #open = false;
  #connected = false;
  #hasConnected = false;
  #openRevision = 0;
  #els = {};
  #closeTimer = 0;
  #reflecting = false;
  #openListenerDocument = null;
  #cancelGesture = null;
  /* A pointer-sequence token consumes only that sequence's trailing click.
     A new pointerdown clears stale tokens; keyboard/programmatic clicks pass. */
  #suppressedClick = null;
  #consumeSuppressedClick(el, event) {
    const suppressed = this.#suppressedClick;
    if (!suppressed || (suppressed.element !== el && suppressed.alternate !== el)) return false;
    if (event.detail === 0) return false;
    if (typeof event.pointerId === 'number' && event.pointerId !== suppressed.pointerId) return false;
    this.#suppressedClick = null;
    return true;
  }
  #armSuppress(element, pointerId, alternate = null) {
    this.#suppressedClick = { element, alternate, pointerId };
  }
  #clearSuppressedClick() { this.#suppressedClick = null; }

  #normalizeModel(value) {
    return this.#models.some(model => model.id === value) ? value : this.#models[0].id;
  }

  #normalizeTier(value) {
    if (value === null || String(value).trim() === '') return DEFAULT_TIER;
    const number = Number(value);
    if (!Number.isFinite(number)) return DEFAULT_TIER;
    return Math.min(TIERS.length - 1, Math.max(0, Math.round(number)));
  }

  #reflect(callback) {
    const wasReflecting = this.#reflecting;
    this.#reflecting = true;
    try {
      callback();
    } finally {
      this.#reflecting = wasReflecting;
    }
  }

  #reflectAttribute(name, value) {
    this.#reflect(() => {
      if (value === null) this.removeAttribute(name);
      else this.setAttribute(name, value);
    });
  }

  constructor() {
    super();
    const rootEl = this.attachShadow({ mode: 'open' });
    rootEl.innerHTML = `<style>${css}</style>${html}`;
    const $ = (s) => rootEl.querySelector(s);
    this.#els = {
      root: $('.root'), panel: $('.panel'), panelInner: $('.panel-inner'),
      matrix: $('.matrix'), rows: [],
      lever: $('.lever'), ball: $('.ball'),
      pill: $('.pill'), label: $('.labelwrap'),
    };
    this.#renderRows();
  }

  #wired = false;

  /* The model list. Setting it rebuilds the rows; the current selection is kept when it still exists. */
  get models() { return this.#models; }
  set models(list) {
    const next = Array.isArray(list) && list.length ? list : DEFAULT_MODELS;
    if (JSON.stringify(next) === JSON.stringify(this.#models)) return;
    this.#cancelActiveGesture();
    this.#models = next;
    this.#model = this.#normalizeModel(this.#model);
    this.#renderRows();
    this.#renderState({ animate: false });
    this.#setLabelImmediate();
  }

  #renderRows() {
    this.#els.matrix.innerHTML = `<div class="well" aria-hidden="true"></div>${rowsHTML(this.#models)}`;
    this.#els.rows = [...this.#els.matrix.querySelectorAll('.row')];
    if (this.#wired) this.#wireGrid(); // listeners lived on the replaced rows
  }

  connectedCallback() {
    const firstConnection = !this.#hasConnected;
    this.#hasConnected = true;
    this.#connected = true;
    this.#cancelActiveGesture({ suppressClick: false });
    this.#resetTransientState();
    this.#syncAttributes();
    this.#renderState({ animate: false });
    this.#setLabelImmediate();
    if (!this.#wired) {
      this.#wired = true;
      this.#wireBar();
      this.#wireGrid();
      this.#wireLever();
    }
    this.#setOpen(this.#open, {
      animate: false,
      focus: firstConnection && this.#open,
      forceFocus: firstConnection,
      restoreFocus: false,
    });
  }

  disconnectedCallback() {
    this.#connected = false;
    this.#cancelActiveGesture();
    this.#removeOpenListeners();
    clearTimeout(this.#swapTimer);
    clearTimeout(this.#closeTimer);
    clearTimeout(this.#celebrateTimer);
    this.#resetTransientState();
    this.#clearSuppressedClick();
  }

  attributeChangedCallback(name, oldValue, v) {
    if (this.#reflecting || !this.#els.root || oldValue === v) return;
    const keepGridFocus = this.#els.matrix.contains(this.shadowRoot.activeElement);
    this.#cancelActiveGesture();
    if (this.getAttribute(name) !== v) return; // a focus handler supplied a newer value

    if (name === 'open') {
      this.#setOpen(v !== null, {
        animate: this.#connected,
        focus: this.#connected && v !== null,
        restoreFocus: true,
      });
      return;
    }

    if (name === 'model') {
      this.#model = this.#normalizeModel(v);
      if (v !== null && v !== this.#model) this.#reflectAttribute('model', this.#model);
    }
    if (name === 'tier') {
      this.#tier = this.#normalizeTier(v);
      const canonical = String(this.#tier);
      if (v !== null && v !== canonical) this.#reflectAttribute('tier', canonical);
    }
    if (name === 'ultra') this.#ultra = v !== null;
    this.#renderState({ animate: false });
    this.#setLabelImmediate();
    if (keepGridFocus && this.#connected) this.#focusElement(this.#selectedCell());
  }

  #syncAttributes() {
    const model = this.getAttribute('model');
    const tier = this.getAttribute('tier');

    this.#model = this.#normalizeModel(model);
    this.#tier = this.#normalizeTier(tier);
    this.#ultra = this.hasAttribute('ultra');
    this.#open = this.hasAttribute('open');

    if (model !== null && model !== this.#model) this.#reflectAttribute('model', this.#model);
    if (tier !== null && tier !== String(this.#tier)) this.#reflectAttribute('tier', String(this.#tier));
  }

  get value() {
    return { model: this.#model, tierIndex: this.#tier, tier: TIERS[this.#tier], ultra: this.#ultra };
  }

  #modelData() {
    return this.#models.find(model => model.id === this.#model) ?? this.#models[0];
  }

  #labelText() {
    return `${this.#modelData().name} ${TIERS[this.#tier]}`;
  }

  #labelHTML() {
    return `<b>${esc(this.#modelData().name)}</b><span class="tier">${TIERS[this.#tier]}</span>`;
  }

  #setLabelImmediate() {
    clearTimeout(this.#swapTimer);
    this.#els.label.classList.remove('is-exit', 'is-enter-start');
    this.#els.label.innerHTML = this.#labelHTML();
    this.#els.pill.setAttribute('aria-label', this.#labelText());
  }

  #emit() {
    this.#reflect(() => {
      this.setAttribute('model', this.#model);
      this.setAttribute('tier', String(this.#tier));
      this.toggleAttribute('ultra', this.#ultra);
    });
    this.dispatchEvent(new CustomEvent('change', {
      bubbles: true,
      composed: true,
      detail: { ...this.value, label: this.#labelText() },
    }));
  }

  /* sync DOM to state */
  #renderState({ animate = true } = {}) {
    const { root, rows } = this.#els;
    if (!animate) root.classList.add('no-anim');
    for (const row of rows) {
      const active = row.dataset.m === this.#model;
      row.classList.toggle('active', active);
      if (active) row.querySelector('.track').style.setProperty('--i', this.#tier);
      row.querySelectorAll('.cell').forEach((c, i) => {
        const sel = active && i === this.#tier;
        c.setAttribute('aria-checked', sel);
        c.tabIndex = sel ? 0 : -1;
      });
    }
    root.classList.toggle('ultra', this.#ultra);
    this.#els.lever.setAttribute('aria-checked', String(this.#ultra));
    this.#els.lever.style.setProperty('--p', this.#ultra ? 1 : 0);
    this.#els.lever.classList.toggle('lit', this.#ultra);
    if (!this.#ultra) {
      clearTimeout(this.#celebrateTimer);
      this.#els.lever.classList.remove('dropping', 'settled');
      this.#els.panelInner.classList.remove('pulse');
    } else if (!this.#els.lever.classList.contains('dropping')) {
      this.#els.lever.classList.add('settled');
    }
    this.#els.pill.setAttribute('aria-label', this.#labelText());
    if (!animate) { void root.offsetWidth; root.classList.remove('no-anim'); }
  }

  #selectedCell() {
    const row = this.#els.rows.find(candidate => candidate.dataset.m === this.#model);
    return row?.querySelectorAll('.cell')[this.#tier] ?? null;
  }

  #focusElement(element) {
    if (!element) return;
    try {
      element.focus({ preventScroll: true });
    } catch (_) {
      element.focus();
    }
  }

  #select(model, tier, { swap = true } = {}) {
    this.#cancelActiveGesture();
    model = this.#normalizeModel(model);
    tier = this.#normalizeTier(tier);
    if (model === this.#model && tier === this.#tier) return;
    this.#model = model;
    this.#tier = tier;
    this.#renderState();
    if (swap) this.#swapLabel(); else this.#setLabelImmediate();
    this.#emit();
  }

  #prefersReducedMotion() {
    return this.ownerDocument.defaultView?.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
  }

  #readCssTime(property, fallback) {
    const view = this.ownerDocument.defaultView;
    const raw = view?.getComputedStyle(this).getPropertyValue(property).trim() ?? '';
    const value = Number.parseFloat(raw);
    if (!Number.isFinite(value) || value < 0) return fallback;
    if (raw.endsWith('ms')) return value;
    if (raw.endsWith('s')) return value * 1000;
    return value;
  }

  /* transitions.dev text-swap three-phase choreography */
  #swapLabel() {
    const el = this.#els.label;
    const next = this.#labelHTML();
    this.#els.pill.setAttribute('aria-label', this.#labelText());
    clearTimeout(this.#swapTimer);
    const dur = this.#readCssTime('--text-swap-dur', 150);
    if (this.#prefersReducedMotion() || dur === 0) {
      el.classList.remove('is-exit', 'is-enter-start');
      el.innerHTML = next;
      return;
    }
    if (el.innerHTML === next) {
      el.classList.remove('is-exit', 'is-enter-start');
      return;
    }
    el.classList.add('is-exit');
    this.#swapTimer = setTimeout(() => {
      el.innerHTML = this.#labelHTML();
      el.classList.remove('is-exit');
      el.classList.add('is-enter-start');
      void el.offsetHeight;
      el.classList.remove('is-enter-start');
    }, dur);
  }
  #swapTimer = 0;

  /* ── popover open/close ── */
  #wireBar() {
    this.#els.pill.addEventListener('click', () => this.#open ? this.close() : this.open());
  }

  #onDocDown = (e) => {
    if (!e.composedPath().includes(this)) {
      this.#requestOpen(false, { animate: true, restoreFocus: false });
    }
  };

  #onDocKeyDown = (e) => {
    if (e.key === 'Escape' && this.#open && !e.defaultPrevented) {
      e.preventDefault();
      this.close();
    }
  };

  #addOpenListeners() {
    if (!this.#connected) return;
    const doc = this.ownerDocument;
    if (this.#openListenerDocument === doc) return;
    this.#removeOpenListeners();
    this.#openListenerDocument = doc;
    doc.addEventListener('pointerdown', this.#onDocDown, true);
    doc.addEventListener('keydown', this.#onDocKeyDown);
  }

  #removeOpenListeners() {
    const doc = this.#openListenerDocument;
    if (!doc) return;
    doc.removeEventListener('pointerdown', this.#onDocDown, true);
    doc.removeEventListener('keydown', this.#onDocKeyDown);
    this.#openListenerDocument = null;
  }

  #requestOpen(on, options) {
    this.#reflect(() => this.toggleAttribute('open', on));
    this.#setOpen(on, options);
  }

  #setOpen(on, { animate = true, focus = false, forceFocus = false, restoreFocus = true } = {}) {
    const revision = ++this.#openRevision;
    on = Boolean(on);
    const wasOpen = this.#open;
    const { panel, root, rows } = this.#els;
    const panelHadFocus = panel.contains(this.shadowRoot.activeElement);
    this.#open = on;
    if (!on) this.#cancelActiveGesture();
    if (revision !== this.#openRevision) return;

    // Repeated close() calls must not cut short an in-flight exit animation.
    if (!on && !wasOpen && animate && this.#connected) {
      panel.classList.remove('is-open');
      panel.setAttribute('inert', '');
      root.classList.remove('open');
      this.#els.pill.setAttribute('aria-expanded', 'false');
      this.#removeOpenListeners();
      return;
    }

    clearTimeout(this.#closeTimer);

    if (on) {
      panel.classList.remove('is-closing');
      panel.classList.add('is-open');
      panel.removeAttribute('inert');
      root.classList.add('open');
      this.#els.pill.setAttribute('aria-expanded', 'true');
      this.#addOpenListeners();
      if (wasOpen) {
        if (focus && forceFocus && this.#connected) this.#focusElement(this.#selectedCell());
        return;
      }
      if (focus && this.#connected) this.#focusElement(this.#selectedCell());
      if (revision !== this.#openRevision) return;

      // pour the active fill in from zero alongside the panel scale-in
      const active = rows.find(row => row.classList.contains('active'));
      if (animate && this.#connected && active && !this.#prefersReducedMotion()) {
        active.classList.add('no-anim');
        active.classList.remove('active');
        void active.offsetWidth;
        active.classList.remove('no-anim');
        active.classList.add('active');
      }
      return;
    }

    panel.classList.remove('is-open');
    panel.setAttribute('inert', '');
    root.classList.remove('open');
    this.#els.pill.setAttribute('aria-expanded', 'false');
    this.#removeOpenListeners();

    const shouldAnimate = wasOpen && animate && this.#connected && !this.#prefersReducedMotion();
    if (shouldAnimate) {
      panel.classList.add('is-closing');
      const closeMs = this.#readCssTime('--dropdown-close-dur', 150);
      if (closeMs === 0) panel.classList.remove('is-closing');
      else this.#closeTimer = setTimeout(() => panel.classList.remove('is-closing'), closeMs);
    } else {
      panel.classList.remove('is-closing');
    }

    if (wasOpen && restoreFocus && panelHadFocus && this.#connected) {
      this.#focusElement(this.#els.pill);
    }
  }

  open() {
    this.#requestOpen(true, { animate: true, focus: true, restoreFocus: false });
  }

  close() {
    this.#requestOpen(false, { animate: true, restoreFocus: true });
  }

  #resetTransientState() {
    const { panel, panelInner, lever, label, rows } = this.#els;
    panel.classList.remove('is-closing');
    panel.toggleAttribute('inert', !this.#open);
    panelInner.classList.remove('pulse');
    label.classList.remove('is-exit', 'is-enter-start');
    lever.classList.remove('dragging', 'dropping');
    for (const row of rows) {
      const track = row.querySelector('.track');
      track.classList.remove('dragging', 'flowing');
      track.style.removeProperty('--x');
    }
  }

  #cancelActiveGesture({ suppressClick = this.#connected } = {}) {
    const cancel = this.#cancelGesture;
    if (!cancel) return;
    this.#cancelGesture = null;
    cancel(null, suppressClick);
  }

  /* ── grid: click, drag, keyboard ── */
  #wireGrid() {
    for (const row of this.#els.rows) {
      const track = row.querySelector('.track');
      const model = row.dataset.m;

      track.addEventListener('click', (event) => {
        if (!this.#consumeSuppressedClick(track, event)) return;
        event.preventDefault();
        event.stopImmediatePropagation();
      }, true);

      // selection rides the click event so mouse, touch-tap, keyboard
      // activation, and programmatic .click() all take the same path
      row.querySelectorAll('.cell').forEach((cell) => {
        cell.addEventListener('click', () => {
          this.#select(model, +cell.dataset.i);
        });
      });

      track.addEventListener('pointerdown', (e) => {
        if (e.button !== 0 || !e.isPrimary) return;
        this.#clearSuppressedClick();
        this.#cancelActiveGesture();
        const win = this.ownerDocument.defaultView;
        if (!win) return;
        const tracks = this.#els.rows.map(r => r.querySelector('.track'));
        const bands = this.#els.rows.map(r => r.getBoundingClientRect());
        const rect = track.getBoundingClientRect(); // all tracks share x geometry
        if (rect.width <= 0) return;
        const cellW = rect.width / TIERS.length;
        const startX = e.clientX, startY = e.clientY;
        const startModel = this.#model;
        const startTier = this.#tier;
        let dragging = false;
        let curIdx = this.#els.rows.findIndex(r => r.classList.contains('active'));
        if (curIdx < 0) curIdx = 0;
        let lastStop = this.#tier;
        let lockTimer = 0;

        const posToStop = (x) => Math.min(TIERS.length - 1, Math.max(0, Math.round((x - rect.left) / cellW - 0.5)));
        const clampX = (cx) => Math.min(rect.width - 25, Math.max(25, cx - rect.left));
        const rowAt = (y) => {
          if (y < bands[0].bottom) return 0;
          if (y >= bands[bands.length - 1].top) return bands.length - 1;
          return Math.max(0, bands.findIndex(b => y >= b.top && y < b.bottom));
        };
        const resetTracks = () => {
          for (const candidate of tracks) {
            candidate.classList.remove('dragging', 'flowing');
            candidate.style.removeProperty('--x');
          }
        };

        // the bar hands off: old row collapses, the bar glides into the new
        // row toward the pointer (.flowing), then locks to 1:1 (.dragging)
        const enterRow = (idx, x) => {
          const old = tracks[curIdx];
          old.classList.remove('dragging', 'flowing');
          old.style.removeProperty('--x');
          clearTimeout(lockTimer);
          curIdx = idx;
          const next = tracks[idx];
          next.style.setProperty('--x', `${x}px`);
          next.classList.add('flowing');
          lockTimer = setTimeout(() => next.classList.replace('flowing', 'dragging'), 240);
          this.#model = this.#models[idx].id;
          this.#renderState();
          this.#setLabelImmediate();
        };

        const move = (ev) => {
          if (ev.pointerId !== e.pointerId) return;
          if (!dragging && Math.abs(ev.clientX - startX) < 4 && Math.abs(ev.clientY - startY) < 6) return;
          const x = clampX(ev.clientX);
          if (!dragging) {
            dragging = true;
            try { track.setPointerCapture(e.pointerId); } catch (_) {}
            const pressedIdx = this.#els.rows.indexOf(row);
            if (pressedIdx === curIdx) tracks[curIdx].classList.add('dragging');
            else enterRow(pressedIdx, x);
          }
          const idx = rowAt(ev.clientY);
          if (idx !== curIdx) enterRow(idx, x);
          else tracks[curIdx].style.setProperty('--x', `${x}px`);
          const stop = posToStop(ev.clientX);
          if (stop !== lastStop) { // instant label while scrubbing — no animation on high-frequency updates
            lastStop = stop;
            this.#tier = stop;
            this.#setLabelImmediate();
          }
        };

        let cancel;
        const detach = () => {
          win.removeEventListener('pointermove', move);
          win.removeEventListener('pointerup', up);
          win.removeEventListener('pointercancel', cancel);
          clearTimeout(lockTimer);
          try {
            if (track.hasPointerCapture(e.pointerId)) track.releasePointerCapture(e.pointerId);
          } catch (_) {}
          if (this.#cancelGesture === cancel) this.#cancelGesture = null;
        };

        const up = (ev) => {
          if (ev.pointerId !== e.pointerId) return;
          const keepGridFocus = this.#els.matrix.contains(this.shadowRoot.activeElement);
          detach();
          if (!dragging) return; // plain press: the ensuing click selects
          const t = tracks[curIdx];
          // Capture retargeting differs across engines, so accept either the
          // destination row or the original pointerdown row exactly once.
          this.#armSuppress(t, e.pointerId, track);
          resetTracks();
          const stop = posToStop(ev.clientX);
          t.style.setProperty('--i', stop);
          this.#tier = stop;
          this.#renderState();
          this.#setLabelImmediate();
          if (keepGridFocus && this.#connected) this.#focusElement(this.#selectedCell());
          if (this.#model !== startModel || this.#tier !== startTier) this.#emit();
        };

        cancel = (ev, suppressClick = false) => {
          if (ev && ev.pointerId !== e.pointerId) return;
          const keepGridFocus = this.#els.matrix.contains(this.shadowRoot.activeElement);
          detach();
          resetTracks();
          if (suppressClick) this.#armSuppress(track, e.pointerId);
          if (!dragging) {
            if (keepGridFocus && this.#connected) this.#focusElement(this.#selectedCell());
            return;
          }
          this.#model = startModel;
          this.#tier = startTier;
          this.#renderState();
          this.#setLabelImmediate();
          if (keepGridFocus && this.#connected) this.#focusElement(this.#selectedCell());
        };

        this.#cancelGesture = cancel;
        // window-level so an off-element release always cleans up
        win.addEventListener('pointermove', move);
        win.addEventListener('pointerup', up);
        win.addEventListener('pointercancel', cancel);
      });

      // roving radio-grid keyboard: arrows move+select, matching pointer's immediate feedback
      track.addEventListener('keydown', (e) => {
        const keys = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'];
        if (!keys.includes(e.key)) return;
        e.preventDefault();
        let mi = this.#models.findIndex(m => m.id === this.#model);
        let ti = this.#tier;
        if (e.key === 'ArrowLeft') ti = Math.max(0, ti - 1);
        if (e.key === 'ArrowRight') ti = Math.min(TIERS.length - 1, ti + 1);
        if (e.key === 'ArrowUp') mi = Math.max(0, mi - 1);
        if (e.key === 'ArrowDown') mi = Math.min(this.#models.length - 1, mi + 1);
        this.#select(this.#models[mi].id, ti);
        this.#focusElement(this.#selectedCell());
      });
    }
  }

  /* ── ULTRA lever: drag, click, keyboard ── */
  #wireLever() {
    const { lever } = this.#els;

    lever.addEventListener('click', (event) => {
      if (this.#consumeSuppressedClick(lever, event)) return;
      this.#toggleUltra();
    });

    lever.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 || !e.isPrimary) return;
      this.#clearSuppressedClick();
      this.#focusElement(lever);
      if (!this.#connected || !this.#open) {
        this.#armSuppress(lever, e.pointerId);
        return;
      }
      this.#cancelActiveGesture();
      const win = this.ownerDocument.defaultView;
      if (!win) return;
      const rect = lever.getBoundingClientRect();
      const travel = rect.height - 56;
      if (travel <= 0) return;
      const startY = e.clientY;
      const startP = this.#ultra ? 1 : 0;
      const startUltra = this.#ultra;
      let dragging = false;

      const move = (ev) => {
        if (ev.pointerId !== e.pointerId) return;
        if (!dragging && Math.abs(ev.clientY - startY) < 3) return;
        if (!dragging) {
          dragging = true;
          lever.classList.add('dragging');
          try { lever.setPointerCapture(e.pointerId); } catch (_) {}
          lever.classList.remove('settled');
        }
        const p = Math.min(1, Math.max(0, startP + (ev.clientY - startY) / travel));
        lever.style.setProperty('--p', p);
        lever.classList.toggle('lit', p > 0.08);
      };

      let cancel;
      const detach = () => {
        win.removeEventListener('pointermove', move);
        win.removeEventListener('pointerup', up);
        win.removeEventListener('pointercancel', cancel);
        try {
          if (lever.hasPointerCapture(e.pointerId)) lever.releasePointerCapture(e.pointerId);
        } catch (_) {}
        if (this.#cancelGesture === cancel) this.#cancelGesture = null;
      };

      const up = (ev) => {
        if (ev.pointerId !== e.pointerId) return;
        detach();
        if (!dragging) return; // plain press: the ensuing click toggles
        this.#armSuppress(lever, e.pointerId);
        lever.classList.remove('dragging');
        const p = Math.min(1, Math.max(0, startP + (ev.clientY - startY) / travel));
        this.#setUltra(p > 0.55);
        if (p >= 1 && this.#ultra) this.#celebrate(); // already at rest — no transitionend will fire
      };

      cancel = (ev, suppressClick = false) => {
        if (ev && ev.pointerId !== e.pointerId) return;
        detach();
        lever.classList.remove('dragging', 'dropping');
        if (suppressClick) this.#armSuppress(lever, e.pointerId);
        if (!dragging) return;
        this.#ultra = startUltra;
        this.#renderState();
      };

      this.#cancelGesture = cancel;
      win.addEventListener('pointermove', move);
      win.addEventListener('pointerup', up);
      win.addEventListener('pointercancel', cancel);
    });

    // Keep activation deterministic across browsers and role="switch" mappings.
    lever.addEventListener('keydown', (e) => {
      if (e.key !== ' ' && e.key !== 'Enter') return;
      e.preventDefault();
      if (!e.repeat) this.#toggleUltra();
    });

    this.#els.ball.addEventListener('transitionend', (e) => {
      if (e.propertyName !== 'top') return;
      lever.classList.remove('dropping');
      if (this.#ultra) this.#celebrate();
    });

    // guarded: descendant label-pop/twinkle animationends bubble through here
    this.#els.panelInner.addEventListener('animationend', (e) => {
      if (e.target === this.#els.panelInner && e.animationName === 'panel-pulse') {
        this.#els.panelInner.classList.remove('pulse');
      }
    });
  }

  #celebrateTimer = 0;

  #toggleUltra() { this.#setUltra(!this.#ultra); }

  #setUltra(on) {
    this.#cancelActiveGesture();
    on = Boolean(on);
    if (on === this.#ultra) { this.#renderState(); return; }
    this.#ultra = on;
    const { lever, root } = this.#els;
    root.classList.toggle('ultra', on);
    lever.setAttribute('aria-checked', String(on));
    lever.classList.toggle('dropping', on);
    lever.classList.toggle('lit', on);
    lever.classList.remove('settled');
    lever.style.setProperty('--p', on ? 1 : 0);
    clearTimeout(this.#celebrateTimer);
    if (on) {
      if (this.#prefersReducedMotion()) {
        lever.classList.remove('dropping');
        lever.classList.add('settled');
        this.#els.panelInner.classList.remove('pulse');
      } else {
        // transitionend is the fast path; the timer guarantees the settled
        // state even if the transition is cancelled or never reports back
        this.#celebrateTimer = setTimeout(() => this.#celebrate(), 300);
      }
    }
    this.#emit();
  }

  /* landing crescendo: bulb bloom + label pop + panel pulse (ticks key off .ultra) */
  #celebrate() {
    const { lever, panelInner } = this.#els;
    if (!this.#ultra || lever.classList.contains('settled')) return;
    clearTimeout(this.#celebrateTimer);
    lever.classList.remove('dropping');
    lever.classList.add('settled');
    panelInner.classList.remove('pulse');
    if (this.#prefersReducedMotion()) return;
    void panelInner.offsetWidth;
    panelInner.classList.add('pulse');
  }
}

if (typeof customElements !== 'undefined' && !customElements.get('model-picker')) {
  customElements.define('model-picker', ModelPicker);
}

export {}; // side-effect module: importing it registers <model-picker>
