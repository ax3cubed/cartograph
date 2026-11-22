"use client";

// How settings save. Two rules, used on every page. Each page's intro says which one applies.
//
// 1. This app only (appearance, keyboard, models): the change applies at once and a small "Saved"
//    status appears (`useFlash` + the `saved` prop). Typing fields that hold a secret or a URL commit
//    with their own button ("Save key"), then show the same status.
// 2. Written to cartograph.yaml (indexing, services, call matching, retrieval): edits stay a draft
//    (`useFileDraft`). A sticky bar at the bottom says "N unsaved changes · writes cartograph.yaml"
//    with Discard and Save. Leaving the page with a draft asks first (`beforeunload` plus an in-page bar).

import { useCallback, useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { usePath } from "../shell/usePath";
import { Check } from "lucide-react";
import { SETTINGS_GROUPS, SETTINGS_SECTIONS } from "@/lib/settings";
import "./settings.css";

export interface FileBar { count: number; onSave: () => void; onDiscard: () => void; target?: string }

const SCOPE = {
  app: "Changes here apply as you make them and stay on this machine.",
  file: <>Changes are written to <span className="set-mono">cartograph.yaml</span> when you press Save.</>,
  readonly: "This page only shows what Cartograph found. Nothing here changes.",
} as const;

export function SettingsPage({ title, intro, saves, scope, saved, actions, file, children }: {
  title: string; intro: React.ReactNode; saves: keyof typeof SCOPE; scope?: React.ReactNode; saved?: boolean;
  actions?: React.ReactNode; file?: FileBar; children: React.ReactNode;
}) {
  const router = useRouter();
  const path = usePath().replace(/\/$/, "");
  const count = file?.count ?? 0;
  const [leave, setLeave] = useState<string | null>(null);
  const keepRef = useRef<HTMLButtonElement>(null);
  const fileRef = useRef(file);
  fileRef.current = file;

  const go = useCallback((href: string) => { if (count > 0) setLeave(href); else router.push(href); }, [count, router]);

  useEffect(() => {
    if (count === 0) return;
    const warn = (e: BeforeUnloadEvent) => { e.preventDefault(); e.returnValue = ""; };
    const click = (e: MouseEvent) => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      const a = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
      if (!a || a.target === "_blank" || a.hasAttribute("download") || a.origin !== location.origin) return;
      if (a.pathname.replace(/\/$/, "") === location.pathname.replace(/\/$/, "")) return;
      e.preventDefault(); e.stopPropagation();
      setLeave(a.pathname + a.search + a.hash);
    };
    window.addEventListener("beforeunload", warn);
    document.addEventListener("click", click, true);
    return () => { window.removeEventListener("beforeunload", warn); document.removeEventListener("click", click, true); };
  }, [count]);

  useEffect(() => { if (leave) keepRef.current?.focus(); }, [leave]);
  useEffect(() => { if (count === 0) setLeave(null); }, [count]);

  return (
    <div className="set-page">
      <label className="set-mobnav">
        <span className="set-k">Settings</span>
        <select className="set-select" aria-label="Settings section" value={path} onChange={(e) => go(e.target.value)}>
          {(Object.keys(SETTINGS_GROUPS) as (keyof typeof SETTINGS_GROUPS)[]).map((g) => (
            <optgroup key={g} label={SETTINGS_GROUPS[g]}>
              {SETTINGS_SECTIONS.filter((s) => s.group === g).map((s) => <option key={s.slug} value={`/settings/${s.slug}`}>{s.label}</option>)}
            </optgroup>
          ))}
        </select>
      </label>
      <header className="set-head">
        <div><h1>{title}</h1><p>{intro} <span className="set-scope">{scope ?? SCOPE[saves]}</span></p></div>
        <div className="set-head-actions">
          {actions}
          <p className="set-saved" role="status">{saved && <><Check size={14} aria-hidden />Saved</>}</p>
        </div>
      </header>
      {children}
      {file && count > 0 && (
        leave ? (
          <div className="set-bar is-leave" role="alertdialog" aria-labelledby="set-bar-t" onKeyDown={(e) => e.key === "Escape" && setLeave(null)}>
            <b id="set-bar-t">Leave with {count} unsaved {count === 1 ? "change" : "changes"}?</b>
            <span className="set-actions">
              <button ref={keepRef} type="button" className="set-btn is-sm" onClick={() => setLeave(null)}>Keep editing</button>
              <button type="button" className="set-btn is-sm is-line" onClick={() => { const to = leave; file.onDiscard(); setLeave(null); router.push(to); }}>Discard and leave</button>
              <button type="button" className="set-btn is-sm is-primary" onClick={() => { const to = leave; file.onSave(); setLeave(null); router.push(to); }}>Save and leave</button>
            </span>
          </div>
        ) : (
          <div className="set-bar" role="region" aria-label="Unsaved changes">
            <span><b>{count} unsaved {count === 1 ? "change" : "changes"}</b> <span className="set-muted">· writes {file.target ?? "cartograph.yaml"}</span></span>
            <span className="set-actions">
              <button type="button" className="set-btn is-sm is-line" onClick={file.onDiscard}>Discard</button>
              <button type="button" className="set-btn is-sm is-primary" onClick={file.onSave}>Save</button>
            </span>
          </div>
        )
      )}
    </div>
  );
}

/** "Saved" status for settings that apply at once. Call `flash()` after a change. */
export function useFlash(): [boolean, () => void] {
  const [on, setOn] = useState(false);
  const t = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(t.current), []);
  return [on, () => { setOn(true); clearTimeout(t.current); t.current = setTimeout(() => setOn(false), 2500); }];
}

export function Switch({ on, onChange, label, labelledBy }: { on: boolean; onChange: (v: boolean) => void; label?: string; labelledBy?: string }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} aria-labelledby={labelledBy} className="set-sw" onClick={() => onChange(!on)}><span /></button>
  );
}

export function Seg<T extends string | number>({ label, value, options, onChange }: { label: string; value: T; options: [T, string][]; onChange: (v: T) => void }) {
  return (
    <div className="set-seg" role="radiogroup" aria-label={label}>
      {options.map(([v, text]) => (
        <button key={String(v)} type="button" role="radio" aria-checked={value === v} onClick={() => onChange(v)}>{text}</button>
      ))}
    </div>
  );
}

export const Chip = ({ tone, dot, children }: { tone?: "ok" | "warn" | "bad" | "off" | "acc"; dot?: boolean; children: React.ReactNode }) => (
  <span className={`set-chip${tone ? ` is-${tone}` : ""}`}>{dot && <i aria-hidden />}{children}</span>
);

/** Optional detail, closed by default. Jargon and tuning knobs live here. */
export const Advanced = ({ label = "Advanced", children }: { label?: string; children: React.ReactNode }) => (
  <details className="set-adv"><summary>{label}</summary><div>{children}</div></details>
);

/** State that lives in this browser only. Reads after mount so the first paint matches the server. */
export function useStored<T>(key: string, initial: T): [T, (v: T) => void] {
  const [v, setV] = useState(initial);
  useEffect(() => {
    try { const raw = localStorage.getItem(key); if (raw !== null) setV(JSON.parse(raw)); } catch { /* storage unavailable */ }
  }, [key]);
  return [v, (n) => { setV(n); try { localStorage.setItem(key, JSON.stringify(n)); } catch { /* storage unavailable */ } }];
}

/** A draft over a saved object. `count` is how many top-level fields differ from what was saved. */
export function useFileDraft<T extends object>(key: string, initial: T) {
  const [saved, setSaved] = useStored<T>(key, initial);
  const [draft, setDraft] = useState<T | null>(null);
  const value = draft ?? saved;
  const keys = new Set([...Object.keys(saved), ...Object.keys(value)]);
  let count = 0;
  for (const k of keys) if (JSON.stringify((saved as Record<string, unknown>)[k]) !== JSON.stringify((value as Record<string, unknown>)[k])) count++;
  return {
    value, count,
    edit: (patch: Partial<T>) => setDraft({ ...value, ...patch }),
    set: (next: T) => setDraft(next),
    save: () => { if (draft) setSaved(draft); setDraft(null); },
    discard: () => setDraft(null),
  };
}
