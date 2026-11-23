"use client";

import { useEffect, useRef } from "react";
import Link from "next/link";
import { AlertTriangle, Check, Copy, X } from "lucide-react";
import { getCurrentWorkspace } from "@/lib/source";
import type { Freshness } from "@/lib/model";
import { behindText, copy } from "./IndexStatus";
import { toast } from "./toast";
import { ago } from "./ago";
import type { DemoIndexState } from "./useIndexState";
import "./indexOverlay.css";

/**
 * Board G02: full-screen loading and "can't read the index". The stale band is IndexBand, below, because it sits in the page flow.
 * Driven by the shell from #state=loading|failed|stale. `onClose` clears the state.
 */
export default function IndexOverlay({ state, why, onClose }: { state: DemoIndexState; why: string | null; onClose: () => void }) {
  if (state === "loading") return <Loading onClose={onClose} />;
  if (state === "failed") return <Failed why={why} onClose={onClose} />;
  return null;
}

function useEsc(onClose: () => void) {
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [onClose]);
}

function Loading({ onClose }: { onClose: () => void }) {
  const ws = getCurrentWorkspace();
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => cancel.current?.focus(), []);
  useEsc(() => cancel.current?.click());
  const total = ws?.modules ?? 0, done = Math.round(total * 0.64), pct = total ? Math.round((done / total) * 100) : 0;
  return (
    <div className="io-full" role="dialog" aria-modal="true" aria-label={`Opening ${ws?.name ?? "index"}`}>
      <div className="io-rings" aria-hidden />
      <div className="io-card" aria-live="polite">
        <div className="io-head"><b>Opening {ws?.name}</b></div>
        <Step label="Read index header" note="schema v3" />
        <Step label="Load services" note={String(ws?.services ?? 0)} />
        <div className="io-step is-now">
          <span className="io-spin" aria-hidden /><span>Lay out modules</span><span className="io-n">{done} / {total}</span>
          <div className="io-bar" role="progressbar" aria-label="Lay out modules" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}><span style={{ width: `${pct}%` }} /></div>
        </div>
        <div className="io-step is-wait"><span className="io-ring" aria-hidden /><span>Load history</span><span className="io-n">{ws?.commits} commits</span></div>
        <p>The first open saves the layout in the index. Later opens skip Lay out modules.</p>
        <p>Cancel stops opening and keeps the last complete index. Open the workspace again to restart.</p>
        <div className="io-actions">
          <button ref={cancel} type="button" className="btn" aria-label={`Cancel opening ${ws?.name}`} onClick={() => { onClose(); toast("Cancelled. The last complete index is still in use."); }}>Cancel</button>
        </div>
      </div>
    </div>
  );
}
const Step = ({ label, note }: { label: string; note: string }) => (
  <div className="io-step"><Check size={18} aria-hidden className="io-ok" /><span>{label}</span><span className="io-n">{note}</span></div>
);

function Failed({ why, onClose }: { why: string | null; onClose: () => void }) {
  const ws = getCurrentWorkspace();
  const schema = why === "schema";
  const file = `${ws?.name ?? "index"}.cartograph`;
  const demo = !!ws?.demo;
  const cmd = schema ? "cargo install cartograph-cli --version 0.4.0" : `cartograph index ~/code/${ws?.name} --rebuild`;
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => first.current?.focus(), []);
  useEsc(onClose);
  return (
    <div className="io-full" role="alertdialog" aria-modal="true" aria-labelledby="io-ft" aria-describedby="io-fb">
      <div className="io-card io-fail">
        <span className={`sh-chip ${schema ? "sh-chip-warn" : "sh-chip-down"}`}>{schema ? "Schema newer than this app" : "File damaged"}</span>
        <h2 id="io-ft">{schema ? `${file} was built by a newer CLI` : `${file} can't be read`}</h2>
        <p id="io-fb">{schema
          ? "The index uses schema v4. This version of Cartograph reads up to v3. Update the app, or install the CLI version this app ships with and rebuild the index."
          : "The file ends before the index is complete. The CLI was probably stopped while writing. Your repositories are unaffected. To rebuild the index, run the command below."}</p>
        <dl>
          {schema ? <><dt>Index</dt><dd>schema v4 · CLI 0.5.1</dd><dt>This app</dt><dd>schema v3 · v0.4.0</dd></>
            : <><dt>Last good write</dt><dd>services, modules (history missing)</dd><dt>Path</dt><dd>~/.cartograph/workspaces/a41c7e90/index.cartograph</dd></>}
        </dl>
        <div className="io-code"><code><span aria-hidden>$ </span>{cmd}</code></div>
        <div className="io-actions">
          <button ref={first} type="button" className="btn primary" aria-label="Copy command" onClick={() => copy(cmd)}><Copy size={14} aria-hidden />Copy command</button>
          <Link className="btn" href="/welcome">Open another index</Link>
          <button type="button" className="btn" onClick={onClose}>{demo ? "Dismiss" : "Back to the map"}</button>
        </div>
      </div>
    </div>
  );
}

/**
 * The stale band: a slim row in the page flow under the header, so it pushes the page down instead of covering it.
 * Reads the same Freshness as the chip and the popover. At narrow widths the command hides and a compact Copy command stays.
 */
export function IndexBand({ state, freshness, onClose }: { state: DemoIndexState; freshness?: Freshness; onClose: () => void }) {
  if (state !== "stale" || !freshness) return null;
  const cmd = "cartograph sync";
  const behind = (freshness.repos ?? []).filter((r) => r.behind > 0);
  return (
    <div className="io-stale" role="status">
      <AlertTriangle size={18} aria-hidden className="io-stale-ic" />
      <p className="io-stale-t">
        <b>{behindText(freshness)} commits behind HEAD</b>
        <span> · {behind.map((r) => `${r.name} ${r.behind}${r.capped ? "+" : ""}`).join(", ")} · built {ago(freshness.builtAt)}. Links added since then aren&apos;t on the map. Search and Ask use the older index.</span>
      </p>
      <div className="io-stale-cmd">
        <code><span aria-hidden>$ </span>{cmd}</code>
        <button type="button" className="io-copy" aria-label={`Copy command: ${cmd}`} onClick={() => copy(cmd)}><Copy size={13} aria-hidden /><span className="io-copy-long">Copy</span><span className="io-copy-short">Copy command</span></button>
      </div>
      <button type="button" className="io-x" aria-label="Dismiss" title="Hide this until you open the workspace again" onClick={onClose}><X size={16} aria-hidden /></button>
    </div>
  );
}
