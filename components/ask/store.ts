"use client";

import { useSyncExternalStore } from "react";
import { ask, checkCitations, type Answer, type Ref, type ToolId } from "@/lib/ask";
import { buildContext, canAsk, loadAi, modelName, narrate, type AiSettings } from "@/lib/ai";
import { COMMITS, dayMs } from "@/lib/workspace";

/**
 * One chat, three places to show it: docked in the map's sidebar, floating over any route, or the /ask page.
 * The thread, the draft and the stream live here, outside React, so a question asked in one place keeps
 * streaming and carries on in the others. Threads and placement persist to localStorage; the draft and the
 * stream do not.
 */

/** Where the panel lives off the /ask page. The page itself is a route, not a mode. */
export type AskMode = "docked" | "floating";
/** The floating panel: open, minimised to the launcher, or closed until Ask is pressed again. */
export type FloatState = "open" | "min" | "closed";

export interface ChatTurn { id: string; a: Answer; at: number; model?: string }
export interface Thread { id: string; title: string; turns: ChatTurn[]; at: number }

interface State {
  threads: Thread[]; // newest first
  active: string | null; // null: a new chat
  mode: AskMode;
  float: FloatState;
  returnTo: string | null; // where the page's "floating" goes back to
  draft: string;
  refs: Ref[];
  live: string | null; // the turn streaming now
}

const KEY = "cartograph:ask";
const MAX_THREADS = 30;
const MAX_UNSUPPORTED = 0.3; // decision G5
/** "Now" off the map, where there is no timeline: the newest commit. */
export const LATEST = dayMs(COMMITS[COMMITS.length - 1].date);

const EMPTY: State = { threads: [], active: null, mode: "docked", float: "open", returnTo: null, draft: "", refs: [], live: null };
let state: State = EMPTY;
let loaded = false;
const subs = new Set<() => void>();

function load() {
  loaded = true;
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null") as Partial<State> | null;
    if (!v) return;
    state = {
      ...state,
      threads: Array.isArray(v.threads) ? v.threads.filter((t) => t && Array.isArray(t.turns)) : [],
      active: typeof v.active === "string" ? v.active : null,
      mode: v.mode === "floating" ? "floating" : "docked",
      float: v.float === "min" || v.float === "closed" ? v.float : "open",
      returnTo: typeof v.returnTo === "string" ? v.returnTo : null,
    };
  } catch { /* storage unavailable or unreadable */ }
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;
function persist() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    let threads = state.threads.slice(0, MAX_THREADS);
    const { active, mode, float, returnTo } = state;
    // a full quota drops the oldest chats rather than losing the save
    while (true) {
      try { localStorage.setItem(KEY, JSON.stringify({ threads, active, mode, float, returnTo })); return; }
      catch { if (!threads.length) return; threads = threads.slice(0, -1); }
    }
  }, 300);
}

function set(patch: Partial<State>, save = true) {
  state = { ...state, ...patch };
  if (save) persist();
  subs.forEach((f) => f());
}

const get = () => { if (!loaded && typeof window !== "undefined") load(); return state; };
const subscribe = (f: () => void) => { subs.add(f); return () => { subs.delete(f); }; };

export function useAsk(): State {
  return useSyncExternalStore(subscribe, get, () => EMPTY);
}

export const askStore = {
  get,
  setMode: (mode: AskMode) => set({ mode, ...(mode === "floating" ? { float: "open" as const } : {}) }),
  setFloat: (float: FloatState) => set({ float }),
  setReturnTo: (returnTo: string | null) => set({ returnTo }),
  setDraft: (draft: string, refs: Ref[] = get().refs) => set({ draft, refs }, false),
  newChat: () => { stop(); set({ active: null, draft: "", refs: [] }); },
  openThread: (id: string) => set({ active: id }),
  deleteThread: (id: string) => {
    if (get().live && get().active === id) stop();
    set({ threads: get().threads.filter((t) => t.id !== id), active: get().active === id ? null : get().active });
  },
};

export const activeThread = (s: State) => s.threads.find((t) => t.id === s.active) ?? null;
export const findTurn = (turnId: string) => {
  for (const th of get().threads) { const x = th.turns.find((t) => t.id === turnId); if (x) return { thread: th, turn: x }; }
  return null;
};

function patchTurn(threadId: string, turnId: string, a: Answer) {
  set({ threads: get().threads.map((th) => (th.id !== threadId ? th : { ...th, turns: th.turns.map((x) => (x.id === turnId ? { ...x, a } : x)) })) });
}

let ctl: AbortController | null = null;
export function stop() {
  ctl?.abort();
  ctl = null;
  if (get().live) set({ live: null }, false);
}

/**
 * Asks in the active thread, starting one if needed. The graph engine answers first (instant, and it is the
 * evidence the model words); with a model set up, the wording streams in and is checked citation by citation.
 * `onShow` hears every version of the answer, so the map can light its evidence.
 */
export async function run(text: string, asOf: number, opts: { ids?: string[]; tool?: ToolId } = {}, ai: AiSettings = loadAi(), onShow?: (a: Answer) => void) {
  if (!text.trim()) return;
  stop();
  const base = ask(text, asOf, opts);
  const context = buildContext(base, asOf, ai.ultra);
  const first: Answer = { ...base, asOf, context };
  const ready = canAsk(ai);
  const now = Date.now();
  const turn: ChatTurn = { id: `s${now.toString(36)}`, a: first, at: now, model: ready ? modelName(ai.model) : undefined };

  const s = get();
  let th = activeThread(s);
  if (!th) th = { id: `c${now.toString(36)}`, title: text.trim().slice(0, 90), turns: [], at: now };
  const tid = th.id;
  const next = { ...th, turns: [...th.turns, turn], at: now };
  set({ threads: [next, ...s.threads.filter((x) => x.id !== tid)], active: tid, draft: "", refs: [], live: ready ? turn.id : null });
  onShow?.(first);
  if (!ready) return;

  const c = (ctl = new AbortController());
  let shown = first;
  const show = (a: Answer) => { shown = a; patchTurn(tid, turn.id, a); onShow?.(a); };
  try {
    let out = "";
    for await (const chunk of narrate(ai, context, c.signal)) {
      out += chunk;
      const checked = checkCitations(out, context.handles, asOf);
      show({ ...first, by: modelName(ai.model), checked, lines: checked.paragraphs.map((p) => [p.map((x) => x.text).join(" ")]) });
    }
    const st = shown.checked?.stats;
    if (st && st.sentences && st.unsupported / st.sentences > MAX_UNSUPPORTED) {
      show({ ...first, checked: shown.checked, notice: `The model's wording was set aside: ${st.unsupported} of ${st.sentences} sentences had no checked source. This is the map's own answer.` });
    }
  } catch (e) {
    if (!c.signal.aborted) show({ ...first, notice: `The model call failed, so this is the map's own answer. ${e instanceof Error ? e.message : e}` });
  } finally {
    if (ctl === c) { ctl = null; set({ live: null }); }
  }
}

/** Puts the cursor in whichever composer is showing, once it has rendered. */
export const focusComposer = () => requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById("ask-input")?.focus()));
