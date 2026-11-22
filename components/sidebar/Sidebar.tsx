"use client";

import { useCallback, useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { ChevronsLeft, ChevronsRight, Info } from "lucide-react";
import { AiMark } from "@/components/brand/AiMark";
import { ASK_EVENT } from "../shell/AppShell";
import { askStore, focusComposer } from "../ask/store";
import { SIDEBAR_CLOSE_EVENT } from "../ask/ModeSwitch";
import "./sidebar.css";

export type SidebarTab = "details" | "ask";
type Mode = "dock" | "drawer" | "sheet";
type Snap = "peek" | "half" | "full";
interface Saved { open: boolean; tab: SidebarTab; width: number }

const KEY = "cartograph:sidebar";
const MIN = 340, MAX = 560, DEFAULT: Saved = { open: true, tab: "details", width: 400 };
const clampW = (w: number) => Math.round(Math.min(MAX, Math.max(MIN, w)));

/** Window event that opens the sidebar on a tab: `showSidebar("details")`. Ask uses ASK_EVENT, which also focuses the composer. */
export const SIDEBAR_EVENT = "cartograph:sidebar";
export const showSidebar = (tab: SidebarTab) => window.dispatchEvent(new CustomEvent(SIDEBAR_EVENT, { detail: tab }));

/** The Ask composer input. Ask owns its markup, so the sidebar finds it in the DOM. */
const askInput = (root: HTMLElement | null) =>
  root?.querySelector<HTMLInputElement | HTMLTextAreaElement>("#ask-input, .ask-mention input, .ask-mention textarea, .ask-bar input");

function useMode(): Mode {
  const [mode, setMode] = useState<Mode>("dock");
  useEffect(() => {
    const sheet = window.matchMedia("(max-width: 640px)"), drawer = window.matchMedia("(max-width: 1180px)");
    const on = () => setMode(sheet.matches ? "sheet" : drawer.matches ? "drawer" : "dock");
    on();
    sheet.addEventListener("change", on); drawer.addEventListener("change", on);
    return () => { sheet.removeEventListener("change", on); drawer.removeEventListener("change", on); };
  }, []);
  return mode;
}

/**
 * The map's right sidebar: Details and Ask as tabs. Docked and resizable (340–560px) on wide screens,
 * an overlay drawer below 1180px, a bottom sheet (peek, half, full) below 640px.
 * Collapsed, it's a 44px strip of tab icons. `]` toggles it; Ask (the header button, `/`, #ask) opens Ask and focuses the composer.
 * Both tabs stay mounted, so a thread or a scroll position survives switching.
 */
export default function Sidebar({ details, ask, askUnread = false }: { details: ReactNode; ask: ReactNode; askUnread?: boolean }) {
  const [s, setS] = useState<Saved>(DEFAULT);
  const [snap, setSnap] = useState<Snap>("peek");
  const [dragging, setDragging] = useState(false);
  const mode = useMode();
  const root = useRef<HTMLElement>(null);
  const id = useId();
  const loaded = useRef(false);

  useEffect(() => {
    if (loaded.current) return; // dev strict mode runs effects twice; read storage once, before anything is saved
    try {
      const v = JSON.parse(localStorage.getItem(KEY) ?? "null") as Partial<Saved> | null;
      if (v) setS({ open: v.open ?? true, tab: v.tab === "ask" ? "ask" : "details", width: clampW(v.width ?? DEFAULT.width) });
      else if (window.matchMedia("(max-width: 1180px)").matches) setS((x) => ({ ...x, open: false })); // a drawer starts closed so the map shows
    } catch { /* storage unavailable */ }
    loaded.current = true;
  }, []);
  useEffect(() => {
    if (!loaded.current) return;
    try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* storage unavailable */ }
  }, [s]);

  const open = useCallback((tab?: SidebarTab) => {
    setS((x) => ({ ...x, open: true, tab: tab ?? x.tab }));
    setSnap((p) => (p === "peek" ? "half" : p));
  }, []);
  const toggle = useCallback(() => {
    setS((x) => ({ ...x, open: !x.open }));
    setSnap((p) => (p === "peek" ? "half" : "peek"));
  }, []);
  const focusAsk = useCallback(() => {
    open("ask");
    // wait for the pane to unhide before focusing
    requestAnimationFrame(() => requestAnimationFrame(() => askInput(root.current)?.focus()));
  }, [open]);

  // through the window event, so the map hears it too (it clears the unread dot)
  const askNow = () => window.dispatchEvent(new Event(ASK_EVENT));
  useEffect(() => {
    // while the chat floats, Ask (header, "/", #ask) brings the floating panel up instead of this tab
    const onAsk = () => {
      if (askStore.get().mode === "floating") { askStore.setFloat("open"); focusComposer(); }
      else focusAsk();
    };
    const onShow = (e: Event) => open((e as CustomEvent<SidebarTab>).detail);
    // the chat moved out (floating or its page): close, and come back on Details next time
    const onClose = () => { setS((x) => ({ ...x, open: false, tab: "details" })); setSnap("peek"); };
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (e.key !== "]" || e.metaKey || e.ctrlKey || e.altKey || el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) return;
      e.preventDefault();
      toggle();
    };
    window.addEventListener(ASK_EVENT, onAsk);
    window.addEventListener(SIDEBAR_EVENT, onShow);
    window.addEventListener(SIDEBAR_CLOSE_EVENT, onClose);
    window.addEventListener("keydown", onKey);
    return () => { window.removeEventListener(ASK_EVENT, onAsk); window.removeEventListener(SIDEBAR_EVENT, onShow); window.removeEventListener(SIDEBAR_CLOSE_EVENT, onClose); window.removeEventListener("keydown", onKey); };
  }, [focusAsk, open, toggle]);

  // ---- width: drag the handle, or use arrow keys on it ----
  const startResize = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const x0 = e.clientX, w0 = s.width;
    setDragging(true);
    const move = (ev: PointerEvent) => setS((x) => ({ ...x, width: clampW(w0 + (x0 - ev.clientX)) }));
    const up = () => { setDragging(false); window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  const resizeKey = (e: ReactKeyboardEvent) => {
    const step = e.shiftKey ? 64 : 16;
    const next = { ArrowLeft: s.width + step, ArrowRight: s.width - step, Home: MIN, End: MAX }[e.key];
    if (next === undefined) return;
    e.preventDefault();
    setS((x) => ({ ...x, width: clampW(next) }));
  };

  // ---- bottom sheet: tap the grip to cycle, or drag it ----
  const cycle = () => setSnap((p) => (p === "peek" ? "half" : p === "half" ? "full" : "peek"));
  const startSheet = (e: ReactPointerEvent<HTMLButtonElement>) => {
    const y0 = e.clientY;
    let moved = false;
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointerup", up);
      const dy = ev.clientY - y0;
      if (Math.abs(dy) < 24) return;
      moved = true;
      setSnap((p) => (dy < 0 ? (p === "peek" ? "half" : "full") : (p === "full" ? "half" : "peek")));
    };
    window.addEventListener("pointerup", up);
    e.currentTarget.addEventListener("click", (ce) => { if (moved) { ce.preventDefault(); ce.stopImmediatePropagation(); } }, { once: true, capture: true });
  };

  const tabKey = (e: ReactKeyboardEvent) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const tab = s.tab === "ask" ? "details" : "ask";
    setS((x) => ({ ...x, tab }));
    document.getElementById(`${id}-${tab}-tab`)?.focus();
  };
  // Esc in the composer hands focus back to the map
  const paneKey = (e: ReactKeyboardEvent) => {
    if (e.key !== "Escape" || e.defaultPrevented) return;
    const el = e.target as HTMLElement;
    if (!/^(INPUT|TEXTAREA)$/.test(el.tagName)) return;
    document.getElementById("map")?.focus();
  };

  const expanded = mode === "sheet" || s.open;
  const cls = ["sb", `is-${mode}`, expanded ? "is-open" : "is-collapsed", mode === "sheet" ? `is-${snap}` : "", dragging ? "is-dragging" : ""].join(" ");
  const tabBtn = (tab: SidebarTab, label: string, icon: ReactNode, dot: boolean) => (
    <button key={tab} type="button" role="tab" id={`${id}-${tab}-tab`} aria-controls={`${id}-${tab}`} aria-selected={s.tab === tab}
      tabIndex={s.tab === tab ? 0 : -1} className="sb-tab" onClick={() => { if (tab !== "ask" || askStore.get().mode === "floating") open(tab); if (tab === "ask") askNow(); }}>
      {icon}<span>{label}</span>{dot && <i className="sb-dot" aria-label="New answer" />}
    </button>
  );
  const unread = askUnread && s.tab !== "ask";

  return (
    <aside ref={root} className={cls} data-tab={s.tab} aria-label="Sidebar" style={mode !== "sheet" ? { ["--sb-cur" as string]: `${s.width}px` } : undefined}>
      {mode === "dock" && expanded && (
        <div className="sb-handle" role="separator" aria-orientation="vertical" aria-label="Resize sidebar" tabIndex={0}
          aria-valuemin={MIN} aria-valuemax={MAX} aria-valuenow={s.width} aria-valuetext={`${s.width} pixels wide`}
          onPointerDown={startResize} onKeyDown={resizeKey} onDoubleClick={() => setS((x) => ({ ...x, width: DEFAULT.width }))} />
      )}

      {!expanded ? (
        <div className="sb-strip">
          <button type="button" className="sb-icon" onClick={() => open()} aria-label="Expand sidebar" aria-keyshortcuts="]" title="Expand sidebar (])">
            <ChevronsLeft size={16} aria-hidden />
          </button>
          <button type="button" className={`sb-icon ${s.tab === "details" ? "is-on" : ""}`} onClick={() => open("details")} aria-label="Show details">
            <Info size={17} aria-hidden />
          </button>
          <button type="button" className={`sb-icon ${s.tab === "ask" ? "is-on" : ""}`} onClick={() => { if (askStore.get().mode === "floating") open("ask"); askNow(); }} aria-label={unread ? "Ask, new answer" : "Ask"}>
            <AiMark size={17} />{unread && <i className="sb-dot" aria-hidden />}
          </button>
        </div>
      ) : (
        <>
          {mode === "sheet" && (
            <button type="button" className="sb-grip" onPointerDown={startSheet} onClick={cycle}
              aria-label={snap === "full" ? "Shrink the panel" : "Expand the panel"}><span aria-hidden /></button>
          )}
          <div className="sb-head">
            <div className="sb-tabs" role="tablist" aria-label="Sidebar" onKeyDown={tabKey}>
              {tabBtn("details", "Details", <Info size={15} aria-hidden />, false)}
              {tabBtn("ask", "Ask", <AiMark size={16} />, unread)}
            </div>
            {mode !== "sheet" && (
              <button type="button" className="sb-icon" onClick={toggle} aria-label="Collapse sidebar" aria-keyshortcuts="]" title="Collapse sidebar (])">
                <ChevronsRight size={16} aria-hidden />
              </button>
            )}
          </div>
        </>
      )}

      <div className="sb-body" hidden={!expanded} onKeyDown={paneKey}>
        <div className="sb-pane" role="tabpanel" id={`${id}-details`} aria-labelledby={`${id}-details-tab`} hidden={s.tab !== "details"}>{details}</div>
        <div className="sb-pane" role="tabpanel" id={`${id}-ask`} aria-labelledby={`${id}-ask-tab`} hidden={s.tab !== "ask"}>{ask}</div>
      </div>
    </aside>
  );
}
