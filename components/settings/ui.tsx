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
import { useLive, useWorkspaces } from "@/lib/api/live";
import { onEvent } from "@/lib/api/events";
import { RequestError } from "@/lib/api/client";
import { getConfig, previewConfig, putConfig, type ConfigFile } from "@/lib/api/settings";
import "./settings.css";

export interface FileBar {
  count: number; onSave: () => void; onDiscard: () => void; target?: string;
  /** FE-13: the unified diff the save would write (text only), a 412 conflict and its two ways out, a failed save. */
  diff?: string; conflict?: boolean; onReload?: () => void; onOverwrite?: () => void; error?: string;
}

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
          file.conflict ? (
            <div className="set-bar is-leave" role="alert">
              <span><b>cartograph.yaml changed on disk</b> <span className="set-muted">· it was edited after this page read it. Reload to see it, or overwrite it with your {count} {count === 1 ? "change" : "changes"}.</span></span>
              <span className="set-actions">
                <button type="button" className="set-btn is-sm is-line" onClick={file.onReload}>Reload</button>
                <button type="button" className="set-btn is-sm is-primary" onClick={file.onOverwrite}>Overwrite</button>
              </span>
            </div>
          ) : (
          <div className="set-bar" role="region" aria-label="Unsaved changes">
            <span><b>{count} unsaved {count === 1 ? "change" : "changes"}</b> <span className="set-muted">· writes {file.target ?? "cartograph.yaml"}</span></span>
            <span className="set-actions">
              <button type="button" className="set-btn is-sm is-line" onClick={file.onDiscard}>Discard</button>
              <button type="button" className="set-btn is-sm is-primary" onClick={file.onSave}>Save</button>
            </span>
            {file.error && <p className="set-bar-err" role="alert">{file.error}</p>}
            {file.diff && <details className="set-bar-diff"><summary>Preview the change to cartograph.yaml</summary><pre className="set-pre" aria-label="Diff of cartograph.yaml">{file.diff}</pre></details>}
          </div>
          )
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

export function Switch({ on, onChange, label, labelledBy, disabled }: { on: boolean; onChange: (v: boolean) => void; label?: string; labelledBy?: string; disabled?: boolean }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} aria-labelledby={labelledBy} className="set-sw" disabled={disabled} onClick={() => onChange(!on)}><span /></button>
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

/** State that lives in this browser only. Reads after mount so the first paint matches the server. "" keeps it in memory. */
export function useStored<T>(key: string, initial: T): [T, (v: T) => void] {
  const [v, setV] = useState(initial);
  useEffect(() => {
    if (!key) return;
    try { const raw = localStorage.getItem(key); if (raw !== null) setV(JSON.parse(raw)); } catch { /* storage unavailable */ }
  }, [key]);
  return [v, (n) => { setV(n); if (key) try { localStorage.setItem(key, JSON.stringify(n)); } catch { /* storage unavailable */ } }];
}

/** How a page's draft maps onto cartograph.yaml: the fields it reads from the text, and the text with them written. */
export interface YamlMap<T> { read: (yaml: string) => Partial<T>; write: (yaml: string, value: T) => string }
const errText = (e: unknown) => (e instanceof RequestError ? `${e.what}. ${e.try_}.` : "The change couldn't be saved.");

/**
 * A draft over a saved object. `count` is how many top-level fields differ from what was saved.
 * Without `yaml` the saved object lives in localStorage under `key` (this app only). With `yaml` and the binary (FE-13)
 * it is read from the workspace's cartograph.yaml: GET once per visit, a preview diff 300 ms after each edit, PUT with
 * If-Match on save, and on 412 the bar offers Reload or Overwrite. A config.changed event reloads a page with no draft.
 */
export function useFileDraft<T extends object>(key: string, initial: T, yaml?: YamlMap<T>) {
  const live = useLive();
  const { current } = useWorkspaces();
  const ws = yaml && live ? current?.id : undefined;
  const [stored, setStored] = useStored<T>(yaml ? "" : key, initial);
  const [draft, setDraft] = useState<T | null>(null);
  const [cfg, setCfg] = useState<{ file?: ConfigFile; missing?: boolean; error?: string; loading: boolean }>({ loading: !!ws });
  const [diff, setDiff] = useState("");
  const [conflict, setConflict] = useState(false);
  const [error, setError] = useState("");
  const yamlRef = useRef(yaml);
  yamlRef.current = yaml;
  const drafting = useRef(false);
  drafting.current = draft !== null;

  const reload = useCallback(async () => {
    if (!ws) return;
    try { setCfg({ file: await getConfig(ws), loading: false }); }
    catch (e) { setCfg(e instanceof RequestError && e.code === "not_found" ? { missing: true, loading: false } : { error: errText(e), loading: false }); }
  }, [ws]);
  useEffect(() => { void reload(); }, [reload]);
  useEffect(() => (ws ? onEvent((e) => { if (e.event === "config.changed" && e.data.ws === ws && !drafting.current) void reload(); }) : undefined), [ws, reload]);

  const fromFile = ws && cfg.file && yaml ? { ...initial, ...yaml.read(cfg.file.yaml) } : null;
  const saved = (fromFile ?? stored) as T;
  const value = draft ?? saved;
  const keys = new Set([...Object.keys(saved), ...Object.keys(value)]);
  let count = 0;
  for (const k of keys) if (JSON.stringify((saved as Record<string, unknown>)[k]) !== JSON.stringify((value as Record<string, unknown>)[k])) count++;

  // the preview diff, 300 ms after the last edit; rendered as text
  const text = ws && cfg.file && draft && count ? yamlRef.current!.write(cfg.file.yaml, draft) : null;
  useEffect(() => {
    if (!ws || text === null) { setDiff(""); return; }
    let live = true;
    const t = setTimeout(() => { previewConfig(ws, text).then((d) => live && setDiff(d.diff), () => live && setDiff("")); }, 300);
    return () => { live = false; clearTimeout(t); };
  }, [ws, text]);

  const write = async (file: ConfigFile) => {
    if (!ws || !draft) return;
    setError("");
    try {
      await putConfig(ws, yamlRef.current!.write(file.yaml, draft), file.etag);
      setDraft(null); setConflict(false);
      await reload();
    } catch (e) {
      if (e instanceof RequestError && e.status === 412) setConflict(true);
      else setError(errText(e));
    }
  };
  return {
    value, count,
    edit: (patch: Partial<T>) => setDraft({ ...value, ...patch }),
    set: (next: T) => setDraft(next),
    save: () => {
      if (ws) { if (cfg.file) void write(cfg.file); return; }
      if (draft) setStored(draft);
      setDraft(null);
    },
    discard: () => { setDraft(null); setConflict(false); setError(""); },
    /** For SettingsPage's `file` bar. `readOnly` while the workspace's cartograph.yaml can't be read. */
    file: {
      live: !!ws, readOnly: !!ws && !cfg.file, missing: !!cfg.missing, loading: cfg.loading, loadError: cfg.error,
      diff, conflict, error,
      onReload: () => { setDraft(null); setConflict(false); void reload(); },
      onOverwrite: async () => { try { await write(await getConfig(ws!)); } catch (e) { setError(errText(e)); } },
    },
  };
}

/** What a file-backed page says when it can't read the workspace's cartograph.yaml; its controls are read-only then. */
export function FileState({ f }: { f: { live: boolean; missing: boolean; loading: boolean; loadError?: string } }) {
  if (!f.live || (!f.missing && !f.loadError && !f.loading)) return null;
  return (
    <p className="set-note set-file-state" role={f.loading ? undefined : "status"}>
      {f.loading ? "Reading cartograph.yaml…" : f.missing
        ? <>This workspace has no <span className="set-mono">cartograph.yaml</span>, so these settings can&apos;t be saved. Run <span className="set-mono">cartograph init</span> in the workspace folder to add one.</>
        : f.loadError}
    </p>
  );
}
