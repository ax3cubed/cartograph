"use client";

import { useEffect, useId, useRef, useState } from "react";
import { NODES, byId } from "@/lib/workspace";
import { now } from "@/lib/source";
import type { Correction, CorrectionAction, LinkType } from "@/lib/model";
import { KINDS, toCorrection, yamlOf, type Draft, type EffLink } from "./links";
import "./map.css";

const FILES = NODES.filter((n) => n.kind === "file");
const WRONG: { action: CorrectionAction; label: string }[] = [
  { action: "remove", label: "These two don't talk to each other" },
  { action: "retarget", label: "The link goes somewhere else" },
  { action: "set", label: "I know how sure this link is" },
];

/** Demo only: a saved correction stays in local state. A real save appends to cartograph.overrides.yaml. */
export default function CorrectionDialog({ link, from, onSave, onClose }: {
  link: EffLink | null; from?: string; onSave: (c: Correction) => void; onClose: () => void;
}) {
  const [tab, setTab] = useState<"fix" | "add">(link ? "fix" : "add");
  const [action, setAction] = useState<CorrectionAction>("remove");
  const [newTo, setNewTo] = useState("");
  const [level, setLevel] = useState("exact");
  const [reason, setReason] = useState("");
  const [add, setAdd] = useState({ from: from ?? FILES[0].id, to: FILES[1].id, type: "calls" as LinkType });
  const id = useId();
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    box.current?.querySelector<HTMLElement>("input,select,textarea")?.focus();
    return () => prev?.focus();
  }, [tab]);

  const draft: Draft = tab === "fix" && link
    ? { action, link: { from: link.from, to: link.to, type: link.type }, newTo, level, reason }
    : { action: "add", link: add, reason };
  const ready = reason.trim() && (tab === "add" ? add.from !== add.to : action !== "retarget" || newTo);
  const nm = (i: string) => byId.get(i)?.name ?? i;
  const save = () => { onSave(toCorrection(draft, new Date(now()).toISOString().slice(0, 10))); onClose(); };

  return (
    <div className="map-scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={box} className="map-dlg" role="dialog" aria-modal="true" aria-labelledby={`${id}t`}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          if (e.key === "Tab") { // keep focus inside
            const els = [...box.current!.querySelectorAll<HTMLElement>("button,input,select,textarea")].filter((x) => !x.hasAttribute("disabled"));
            const first = els[0], last = els[els.length - 1];
            if (e.shiftKey && document.activeElement === first) (e.preventDefault(), last.focus());
            else if (!e.shiftKey && document.activeElement === last) (e.preventDefault(), first.focus());
          }
        }}>
        <div className="map-tabs" role="tablist" aria-label="Correction">
          <button role="tab" aria-selected={tab === "fix"} disabled={!link} onClick={() => setTab("fix")}>Mark link as wrong</button>
          <button role="tab" aria-selected={tab === "add"} onClick={() => setTab("add")}>Add a missing link</button>
        </div>
        <h2 id={`${id}t`}>{tab === "fix" ? "Mark this link as wrong" : "Add a missing link"}</h2>
        {tab === "fix" && link && (
          <>
            <p className="map-sub">{nm(link.from)} → {nm(link.to)} · {KINDS[link.type]}{link.confidence ? ` · ${link.confidence.level} · ${link.confidence.score.toFixed(2)}` : ""}</p>
            <fieldset className="map-opts">
              <legend className="map-k">What is wrong</legend>
              {WRONG.map((w) => (
                <label key={w.action} className="map-opt">
                  <input type="radio" name={id} checked={action === w.action} onChange={() => setAction(w.action)} />{w.label}<small>{w.action}:</small>
                </label>
              ))}
            </fieldset>
            {action === "retarget" && (
              <label className="map-lab">Correct target
                <select className="map-inp" value={newTo} onChange={(e) => setNewTo(e.target.value)}>
                  <option value="">Choose a file</option>
                  {FILES.filter((f) => f.id !== link.to && f.id !== link.from).map((f) => <option key={f.id} value={f.id}>{f.id}</option>)}
                </select>
              </label>
            )}
            {action === "set" && (
              <label className="map-lab">Confidence
                <select className="map-inp" value={level} onChange={(e) => setLevel(e.target.value)}>
                  <option value="exact">Exact</option><option value="likely">Likely</option><option value="guessed">Guessed</option>
                </select>
              </label>
            )}
          </>
        )}
        {tab === "add" && (
          <div className="map-two">
            <label className="map-lab">From
              <select className="map-inp" value={add.from} onChange={(e) => setAdd({ ...add, from: e.target.value })}>{FILES.map((f) => <option key={f.id} value={f.id}>{f.id}</option>)}</select>
            </label>
            <label className="map-lab">To
              <select className="map-inp" value={add.to} onChange={(e) => setAdd({ ...add, to: e.target.value })}>{FILES.map((f) => <option key={f.id} value={f.id}>{f.id}</option>)}</select>
            </label>
            <label className="map-lab">Link kind
              <select className="map-inp" value={add.type} onChange={(e) => setAdd({ ...add, type: e.target.value as LinkType })}>
                {(Object.keys(KINDS) as LinkType[]).map((k) => <option key={k} value={k}>{KINDS[k]}</option>)}
              </select>
            </label>
          </div>
        )}
        <label className="map-lab">Reason, for whoever reads the override file next (required)
          <textarea className="map-inp" rows={2} value={reason} onChange={(e) => setReason(e.target.value)} required />
        </label>
        <div>
          <span className="map-k">Adds to cartograph.overrides.yaml</span>
          <pre className="map-diff">{yamlOf(draft).map((l, i) => <span key={i}>{"+ " + l}{"\n"}</span>)}</pre>
        </div>
        <div className="map-dlg-foot">
          <span>Demo: this correction stays in this session and is not written to disk. In the app, commit the file so the next index and everyone else's map include it.</span>
          <button type="button" className="btn" onClick={onClose}>Cancel</button>
          <button type="button" className="btn primary" disabled={!ready} onClick={save}>Save correction</button>
        </div>
      </div>
    </div>
  );
}
