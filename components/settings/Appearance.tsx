"use client";

import { useEffect, useState } from "react";
import type { AppearanceSettings } from "@/lib/model";
import { DEFAULT_APPEARANCE, loadAppearance, saveAppearance } from "@/lib/theme";
import { serviceColor } from "@/lib/workspace";
import { getCurrentWorkspace } from "@/lib/source";
import { Seg, SettingsPage, useFlash } from "./ui";
import "./appearance.css";

const THEMES: [AppearanceSettings["theme"], string, string][] = [
  ["system", "System", "Follows your OS and switches live"],
  ["light", "Light", "For bright rooms"],
  ["dark", "Dark", "Default. For low light"],
];

export default function Appearance() {
  const [a, setA] = useState<AppearanceSettings>(DEFAULT_APPEARANCE);
  useEffect(() => setA(loadAppearance()), []);
  const [saved, flash] = useFlash();
  const set = <K extends keyof AppearanceSettings>(k: K, v: AppearanceSettings[K]) => { const n = { ...a, [k]: v }; setA(n); saveAppearance(n); flash(); };
  const ws = getCurrentWorkspace();
  const label = [a.theme, a.contrast === "more" && "more contrast", a.motion !== "follow" && `motion ${a.motion}`, a.hues === "safe" && "colour-blind safe", a.size !== 100 && `${a.size}%`, a.density === "compact" && "compact"].filter(Boolean).join(" · ");
  const [g, b, au] = ["gateway", "billing", "auth"].map(serviceColor);

  return (
    <SettingsPage title="Appearance" saves="app" saved={saved} intro="Choose how the map looks and moves. Changes show in the preview and across the app straight away.">
      <div className="set-grid set-ap-grid">
        <div className="set-col">
          <section className="set-sec" aria-labelledby="set-ap-th">
            <h2 id="set-ap-th">Theme</h2>
            <div className="set-ap-themes" role="radiogroup" aria-labelledby="set-ap-th">
              {THEMES.map(([id, name, desc]) => (
                <button key={id} type="button" role="radio" aria-checked={a.theme === id} className="set-ap-opt" onClick={() => set("theme", id)}>
                  <span className={`set-ap-sw is-${id}`} aria-hidden />
                  <b>{name}</b><small>{desc}</small>
                </button>
              ))}
            </div>
          </section>
          <section className="set-sec" aria-labelledby="set-ap-ac">
            <h2 id="set-ap-ac">Accessibility</h2>
            <div className="set-ap-set"><b>Contrast</b><Seg label="Contrast" value={a.contrast} onChange={(v) => set("contrast", v)} options={[["standard", "Standard"], ["more", "More contrast"]]} /><span>More darkens lines and muted text, and thickens links.</span></div>
            <div className="set-ap-set"><b>Motion</b><Seg label="Motion" value={a.motion} onChange={(v) => set("motion", v)} options={[["follow", "Follow system"], ["reduce", "Reduce"], ["full", "Full"]]} /><span>Reduce turns off the layout morph and link flashes.</span></div>
            <div className="set-ap-set"><b>Service colours</b><Seg label="Service colours" value={a.hues} onChange={(v) => set("hues", v)} options={[["default", "Default"], ["safe", "Colour-blind safe"]]} /><span>The safe set keeps the 6 domains distinct for protan, deutan and tritan vision.</span></div>
            <div className="set-ap-set"><b>Text size</b><Seg label="Text size" value={a.size} onChange={(v) => set("size", v)} options={[[90, "90%"], [100, "100%"], [115, "115%"], [130, "130%"]]} /><span>Map labels scale too. Panels reflow instead of truncating.</span></div>
            <div className="set-ap-set"><b>Density</b><Seg label="Density" value={a.density} onChange={(v) => set("density", v)} options={[["comfortable", "Comfortable"], ["compact", "Compact"]]} /><span>Compact tightens the tree and tables, and keeps 32 px minimum targets.</span></div>
          </section>
        </div>

        <section className="set-sec set-ap-prev" aria-labelledby="set-ap-pr">
          <div className="set-sec-head"><h2 id="set-ap-pr">Preview</h2><span className="set-mono set-muted">{label}</span></div>
          <div className="set-ap-frame">
            <div className="set-ap-bar"><i aria-hidden />cartograph<span>{ws?.name ?? "workspace"} · {ws?.services ?? 0} services</span></div>
            <svg viewBox="0 0 400 250" role="img" aria-label="Preview of the map in the chosen theme">
              <g stroke="var(--edge)" style={{ strokeWidth: "calc(1.2px * var(--link-w))" }} strokeOpacity=".7"><line x1="110" y1="80" x2="280" y2="100" /><line x1="110" y1="80" x2="170" y2="190" /><line x1="280" y1="100" x2="170" y2="190" /></g>
              <line x1="280" y1="100" x2="340" y2="190" stroke="var(--up)" strokeWidth="2" />
              <line x1="110" y1="80" x2="50" y2="170" stroke="var(--down)" strokeWidth="1.6" strokeDasharray="6 4" />
              <circle cx="110" cy="80" r="22" fill={b} fillOpacity=".35" stroke={b} strokeWidth="3" />
              <circle cx="280" cy="100" r="18" fill={g} fillOpacity=".35" stroke="var(--accent)" strokeWidth="3" />
              <circle cx="170" cy="190" r="15" fill={au} fillOpacity=".35" stroke={au} strokeWidth="3" />
              <circle cx="340" cy="190" r="8" fill="none" stroke={g} strokeWidth="2.5" />
              <circle cx="50" cy="170" r="8" fill="none" stroke={b} strokeWidth="2.5" />
              {[["billing", 110, 122], ["gateway", 280, 138], ["auth", 170, 224]].map(([t, x, y]) => <text key={t} x={x} y={y} textAnchor="middle">{t}</text>)}
            </svg>
            <div className="set-ap-foot"><span className="is-up">↗ +2 links</span><span className="is-down">↙ −1 link</span><span>Nov 7 · <span className="set-mono">3e0d7a2</span></span></div>
          </div>
          <p className="set-note">Added and removed links keep their arrows and line styles in every theme, so they read the same in greyscale.</p>
        </section>
      </div>
    </SettingsPage>
  );
}
