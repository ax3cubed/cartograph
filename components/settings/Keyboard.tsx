"use client";

import { useEffect, useRef, useState } from "react";
import { BROWSER_KEYS, DEFAULT_SHORTCUTS, loadKeymap, saveKeymap, type Shortcut } from "@/lib/settings";
import { SettingsPage, useFlash } from "./ui";
import "./keyboard.css";

const NAMES: Record<string, string> = { Escape: "Esc", Enter: "↵", " ": "Space", ArrowLeft: "←", ArrowRight: "→", ArrowUp: "↑", ArrowDown: "↓" };
/** The chord a key event makes, in the same notation as the list: ["⌘", "K"]. Null for a bare modifier. */
function chordOf(e: KeyboardEvent): string[] | null {
  if (["Meta", "Control", "Alt", "Shift"].includes(e.key)) return null;
  const k = NAMES[e.key] ?? (e.key.length === 1 ? e.key.toUpperCase() : e.key);
  const printable = e.key.length === 1 && !/[a-z0-9]/i.test(e.key); // shift is already in "?" or "+"
  return [e.metaKey && "⌘", e.ctrlKey && "Ctrl", e.altKey && "⌥", e.shiftKey && !printable && "⇧", k].filter(Boolean) as string[];
}
const conflictOf = (keys: string[]) => BROWSER_KEYS[keys.join(" ")];

export default function Keyboard() {
  const [list, setList] = useState<Shortcut[]>(DEFAULT_SHORTCUTS);
  const [rec, setRec] = useState<string | null>(null);
  const [msg, setMsg] = useState("");
  const btns = useRef<Record<string, HTMLButtonElement | null>>({});
  useEffect(() => setList(loadKeymap()), []);

  const [saved, flash] = useFlash();
  const commit = (next: Shortcut[]) => { setList(next); saveKeymap(next); flash(); };
  useEffect(() => {
    if (!rec) return;
    const on = (e: KeyboardEvent) => {
      e.preventDefault(); e.stopPropagation();
      if (e.key === "Escape") { setRec(null); setMsg("Cancelled. The shortcut is unchanged."); return; }
      const keys = chordOf(e);
      if (!keys) return;
      const used = list.find((s) => s.id !== rec && s.keys.join(" ") === keys.join(" "));
      if (used) { setMsg(`${keys.join(" ")} is already used for "${used.what}". Press other keys, or Esc to cancel.`); return; }
      const row = list.find((s) => s.id === rec)!;
      commit(list.map((s) => (s.id === rec ? { ...s, keys } : s)));
      setMsg(`${row.what} is now ${keys.join(" ")}.${conflictOf(keys) ? ` This is also ${conflictOf(keys)} in your browser.` : ""}`);
      setRec(null);
      requestAnimationFrame(() => btns.current[row.id]?.focus());
    };
    window.addEventListener("keydown", on, true);
    return () => window.removeEventListener("keydown", on, true);
  }, [rec, list]);

  const conflicts = list.filter((s) => conflictOf(s.keys));
  const groups = [...new Set(list.map((s) => s.group))];
  const start = (id: string) => { setRec(id); setMsg("Press the new keys."); btns.current[id]?.focus(); };

  return (
    <SettingsPage
      title="Keyboard" saves="app" saved={saved}
      intro={<>Every action on the map has a key. Press <kbd>?</kbd> anywhere to see this list, or select a shortcut here to change it.</>}
      actions={<>
        {conflicts.length > 0 && <>
          <span className="set-kb-warn">{conflicts.length} conflict{conflicts.length > 1 ? "s" : ""}: {conflicts[0].keys.map((k) => <kbd key={k}>{k}</kbd>)} is also a browser shortcut for {conflictOf(conflicts[0].keys)}</span>
          <button type="button" className="set-btn is-sm is-line set-kb-warn-btn" aria-label={`Resolve conflict: change shortcut for ${conflicts[0].what}`} onClick={() => start(conflicts[0].id)}>Change shortcut</button>
        </>}
        <button type="button" className="set-btn is-sm is-line" aria-label="Reset to defaults: all shortcuts" onClick={() => { commit(DEFAULT_SHORTCUTS); setRec(null); setMsg("All shortcuts are back to their defaults."); }}>Reset to defaults</button>
      </>}
    >
      <p className="set-status" role="status" aria-live="polite">{msg}</p>
      <div className="set-grid">
        {groups.map((g) => (
          <section key={g} className="set-sec" aria-label={g}>
            <h2>{g}</h2>
            <ul className="set-kb-list">
              {list.filter((s) => s.group === g).map((s) => {
                const c = conflictOf(s.keys);
                const on = rec === s.id;
                return (
                  <li key={s.id} className={c ? "is-conflict" : undefined}>
                    <span><span>{s.what}</span>{c && <small>Conflicts with {c}</small>}</span>
                    <span className="set-kb-keys">{on ? <span className="set-kb-rec">Press new keys…</span> : s.keys.map((k, i) => <kbd key={i}>{k}</kbd>)}</span>
                    <button type="button" ref={(el) => { btns.current[s.id] = el; }} className="set-btn is-sm is-line" disabled={s.set}
                      title={s.set ? "These keys work as a set, so they can't be changed one by one" : undefined}
                      aria-label={`${on ? "Cancel changing" : "Change"} shortcut for ${s.what}`} onClick={() => (on ? (setRec(null), setMsg("")) : start(s.id))}>{on ? "Cancel" : "Change"}</button>
                  </li>
                );
              })}
            </ul>
          </section>
        ))}
      </div>
    </SettingsPage>
  );
}
