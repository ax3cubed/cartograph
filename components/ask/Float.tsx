"use client";

import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { usePathname, useRouter } from "next/navigation";
import { Minus, X } from "lucide-react";
import { AiMark } from "@/components/brand/AiMark";
import Ask, { type AskProps } from "../Ask";
import ModeSwitch, { useMoveAsk } from "./ModeSwitch";
import { LATEST, askStore, focusComposer, useAsk } from "./store";

interface Box { x: number; y: number; w: number; h: number }
const KEY = "cartograph:ask-float";
const MIN_W = 340, MIN_H = 380, PAD = 8;

function fit(b: Box): Box {
  const vw = window.innerWidth, vh = window.innerHeight;
  const w = Math.min(Math.max(b.w, MIN_W), vw - PAD * 2), h = Math.min(Math.max(b.h, MIN_H), vh - PAD * 2);
  return { w, h, x: Math.min(Math.max(b.x, PAD), vw - w - PAD), y: Math.min(Math.max(b.y, PAD), vh - h - PAD) };
}
function initial(): Box {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null") as Box | null;
    if (v && [v.x, v.y, v.w, v.h].every(Number.isFinite)) return fit(v);
  } catch { /* storage unavailable */ }
  const w = 420, h = Math.min(640, window.innerHeight - 120);
  return fit({ w, h, x: window.innerWidth - w - 16, y: window.innerHeight - h - 16 });
}
const save = (b: Box) => { try { localStorage.setItem(KEY, JSON.stringify(b)); } catch { /* storage unavailable */ } };

/**
 * The chat floating over the app: a non-modal panel the person can drag by its header and resize from its
 * top-left corner. Position and size are remembered. Esc minimises it to the launcher in the bottom-right.
 * Below 640px it becomes a full-width sheet and stops moving.
 */
function Panel(props: AskProps & { onMap: boolean }) {
  const s = useAsk();
  const move = useMoveAsk();
  const [box, setBox] = useState<Box>(initial);
  const boxRef = useRef(box);
  boxRef.current = box;

  useEffect(() => {
    const on = () => setBox((b) => fit(b));
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  const commit = (b: Box) => { const f = fit(b); setBox(f); save(f); };

  const drag = (e: ReactPointerEvent, mode: "move" | "size") => {
    if (e.button !== 0 || (mode === "move" && (e.target as HTMLElement).closest("button, a"))) return;
    e.preventDefault();
    const x0 = e.clientX, y0 = e.clientY, b0 = boxRef.current;
    const at = (ev: PointerEvent): Box => {
      const dx = ev.clientX - x0, dy = ev.clientY - y0;
      if (mode === "move") return { ...b0, x: b0.x + dx, y: b0.y + dy };
      const w = Math.max(MIN_W, b0.w - dx), h = Math.max(MIN_H, b0.h - dy);
      return { w, h, x: b0.x + b0.w - w, y: b0.y + b0.h - h };
    };
    const mv = (ev: PointerEvent) => setBox(fit(at(ev)));
    const up = (ev: PointerEvent) => { commit(at(ev)); window.removeEventListener("pointermove", mv); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", mv);
    window.addEventListener("pointerup", up);
  };
  const keyMove = (e: ReactKeyboardEvent, mode: "move" | "size") => {
    const d = e.shiftKey ? 64 : 16;
    const v = { ArrowLeft: [-d, 0], ArrowRight: [d, 0], ArrowUp: [0, -d], ArrowDown: [0, d] }[e.key];
    if (!v) return;
    e.preventDefault();
    const b = boxRef.current;
    commit(mode === "move" ? { ...b, x: b.x + v[0], y: b.y + v[1] } : { x: b.x + v[0], y: b.y + v[1], w: b.w - v[0], h: b.h - v[1] });
  };

  const minimise = () => {
    askStore.setFloat("min");
    requestAnimationFrame(() => document.querySelector<HTMLElement>(".ask-launch")?.focus());
  };
  const onKey = (e: ReactKeyboardEvent) => {
    // the @ menu, the thread menu and the inspector dialog handle their own Esc
    if (e.key !== "Escape" || e.defaultPrevented || (e.target as HTMLElement).closest("dialog")) return;
    e.preventDefault();
    e.stopPropagation(); // the map's own Esc (back out of focus) stays out of it
    minimise();
  };
  const lit = props.onMap && !!props.answer;

  return (
    <section className="ask-float" role="dialog" aria-label="Cartograph AI" onKeyDown={onKey}
      style={{ left: box.x, top: box.y, width: box.w, height: box.h }}>
      <header className="ask-float-head" onPointerDown={(e) => drag(e, "move")}>
        <div className="ask-float-grab" tabIndex={0} role="button" aria-roledescription="movable panel"
          aria-label="Cartograph AI. Use the arrow keys to move the panel." onKeyDown={(e) => keyMove(e, "move")}>
          <AiMark size={18} active={!!s.live} />
          <h2>Cartograph AI</h2>
        </div>
        <ModeSwitch current="floating" />
        <button type="button" className="ask-icon" aria-label="Minimise" title="Minimise (Esc)" aria-keyshortcuts="Escape" onClick={minimise}><Minus size={15} aria-hidden /></button>
        <button type="button" className="ask-icon" aria-label="Close" title="Close" onClick={() => askStore.setFloat("closed")}><X size={15} aria-hidden /></button>
      </header>
      {lit && (
        <p className="ask-float-note">
          The evidence is lit on the map. Move this panel, or
          <button type="button" className="ask-link" onClick={() => move("docked")}>dock it beside the map</button>
        </p>
      )}
      <Ask {...props} variant="floating" />
      <div className="ask-float-grip" role="button" tabIndex={0} aria-roledescription="resize handle"
        aria-label={`Resize the panel, now ${Math.round(box.w)} by ${Math.round(box.h)} pixels. Use the arrow keys.`}
        onPointerDown={(e) => drag(e, "size")} onKeyDown={(e) => keyMove(e, "size")} />
    </section>
  );
}

/** The floating chat, or its launcher when minimised. Renders nothing unless the chat is set to float. */
export default function AskFloat(props: AskProps & { onMap?: boolean }) {
  const s = useAsk();
  if (s.mode !== "floating" || s.float === "closed") return null;
  if (s.float === "min") {
    return (
      <button type="button" className="ask-launch" aria-label="Open Cartograph AI" title="Cartograph AI"
        onClick={() => { askStore.setFloat("open"); focusComposer(); }}>
        <AiMark size={22} active={!!s.live} style={{ "--aimark-accent": "currentColor" } as React.CSSProperties} />
      </button>
    );
  }
  return <Panel {...props} onMap={!!props.onMap} />;
}

/** The floating chat on every route but the map (which mounts its own, wired to the map) and the chat's page. */
export function AskFloatHost() {
  const raw = usePathname();
  const path = raw.length > 1 ? raw.replace(/\/$/, "") : raw;
  const router = useRouter();
  if (path === "/" || path === "/ask" || path.startsWith("/welcome")) return null;
  const back = `&back=${encodeURIComponent(path)}`;
  const hrefFor = (id: string) => `/#focus=${encodeURIComponent(id)}${back}`;
  return (
    <AskFloat t={LATEST} answer={null} onAnswer={() => {}} onOpen={(id) => router.push(hrefFor(id))}
      hrefFor={hrefFor} answerHref={(turn) => `/#answer=${encodeURIComponent(turn)}${back}`} />
  );
}
