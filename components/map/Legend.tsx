"use client";

import { DOMAINS, domainHue, hueText } from "./domains";
import "./map.css";

const Line = ({ cls, w = 28 }: { cls: string; w?: number }) => (
  <svg width={w} height="12" aria-hidden><line className={`map-ln ${cls}`} x1="0" y1="6" x2={w} y2="6" />{cls === "fixed" && <rect className="map-dia" x="9" y="1" width="9" height="9" rx="1.5" transform="rotate(45 14 6)" />}</svg>
);

/** Board 00 link encoding and the domain colours. Confidence is line style; time is colour. */
export default function Legend({ compare, weak, weakCount, onWeak }: { compare: boolean; weak: boolean; weakCount: number; onWeak: (v: boolean) => void }) {
  return (
    <details className="map-legend">
      <summary>Legend</summary>
      <div role="group" aria-label="Link style">
        <span><Line cls="exact" />Exact</span>
        <span><Line cls="likely" />Likely</span>
        <span><Line cls="guessed" />Guessed</span>
        <span><Line cls="fixed" />Overridden</span>
        <span><Line cls="cross" />Cross-service</span>
        {compare ? (<>
          <span><Line cls="cmp-added" />Added</span>
          <span><Line cls="cmp-removed" />Removed</span>
          <span><Line cls="cmp-changed" />Changed</span>
        </>) : (<>
          <span><Line cls="lg-added" />Added in last 30 days</span>
          <span><Line cls="lg-removed" />Removed in last 30 days</span>
        </>)}
      </div>
      <div className="map-doms" role="group" aria-label="Domains">
        {DOMAINS.map((d, i) => <span key={d.id}><i style={{ background: domainHue(i) }} aria-hidden /><b style={{ color: hueText(domainHue(i)) }}>{d.id}</b></span>)}
      </div>
      <label className="map-weak">
        <input type="checkbox" checked={weak} onChange={(e) => onWeak(e.target.checked)} />
        Show weak links (below 0.30) · {weakCount}
      </label>
    </details>
  );
}
