"use client";

import { useLayoutEffect, useRef, type ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import "./map.css";

/** What a context menu is about: one node, or the line between two nodes on the map. */
export type MenuTarget = { type: "node"; id: string } | { type: "link"; a: string; b: string };
/** Open request: `at` is the pointer, or null for the keyboard (the menu then sits at the element's centre). */
export type MenuReq = { target: MenuTarget; at: { x: number; y: number } | null; from: Element | null };
export type MenuItem = { label: string; icon: LucideIcon; run: () => void; hint?: string; key?: string; hidden?: boolean };

// ContextMenu and Shift+F10 also fire a native `contextmenu` right after the keydown; this lets the handler skip it.
let lastKey = 0;
/** True for the keys that open a context menu (ContextMenu, Shift+F10). */
export const isMenuKey = (e: { key: string; shiftKey: boolean }) => {
  const hit = e.key === "ContextMenu" || (e.shiftKey && e.key === "F10");
  if (hit) lastKey = Date.now();
  return hit;
};

/**
 * Handler for `onContextMenu`: suppresses the browser's menu on this element only and reports where to open.
 * A contextmenu that follows a menu key (or comes with no pointer position) opens at the element instead.
 */
export const onMenuEvent = (e: React.MouseEvent, open: (at: { x: number; y: number } | null) => void) => {
  e.preventDefault();
  e.stopPropagation();
  if (Date.now() - lastKey < 600) return; // the keydown already opened it
  open(e.clientX || e.clientY ? { x: e.clientX, y: e.clientY } : null);
};

/**
 * Touch: a press held ~500ms without moving opens the menu. Call from `onPointerDown`; mouse and pen are ignored.
 * The click that ends the press is swallowed so it doesn't also select or open.
 */
export function longPress(e: React.PointerEvent, open: (at: { x: number; y: number }) => void) {
  if (e.pointerType !== "touch") return;
  const x = e.clientX, y = e.clientY;
  const done = () => {
    clearTimeout(timer);
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", done);
    window.removeEventListener("pointercancel", done);
  };
  const move = (ev: PointerEvent) => { if (Math.abs(ev.clientX - x) + Math.abs(ev.clientY - y) > 8) done(); };
  const timer = setTimeout(() => {
    done();
    lastKey = 0;
    // the compatibility mousedown and click that follow the lift land wherever the finger is, often on the menu
    // that just opened under it: they must not move focus, select a node or activate an item
    const swallow = (ev: Event) => { ev.stopPropagation(); ev.preventDefault(); };
    for (const t of ["mousedown", "click"]) window.addEventListener(t, swallow, { capture: true, once: true });
    setTimeout(() => { for (const t of ["mousedown", "click"]) window.removeEventListener(t, swallow, { capture: true }); }, 800);
    open({ x, y });
  }, 500);
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", done);
  window.addEventListener("pointercancel", done);
}

/** The context menu for map nodes, tree rows and map links. Items are built by the owner of the handlers (Atlas). */
export default function NodeMenu({ req, title, sub, items, onClose }: {
  req: MenuReq; title: ReactNode; sub: ReactNode; items: MenuItem[]; onClose: () => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const shown = items.filter((i) => !i.hidden);
  const buttons = () => [...(root.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [])];

  // place at the pointer (or the element's centre), clamped inside the viewport, then focus the first item
  useLayoutEffect(() => {
    const el = root.current;
    if (!el) return;
    let { x, y } = req.at ?? { x: 0, y: 0 };
    if (!req.at && req.from) {
      const r = req.from.getBoundingClientRect();
      x = r.left + r.width / 2; y = r.top + r.height / 2;
    }
    const m = 8, w = el.offsetWidth, h = el.offsetHeight;
    const vw = window.innerWidth, vh = window.innerHeight;
    if (x + w > vw - m) x = Math.max(m, req.at ? x - w : vw - m - w);
    if (y + h > vh - m) y = Math.max(m, req.at ? y - h : vh - m - h);
    el.style.left = `${Math.max(m, x)}px`;
    el.style.top = `${Math.max(m, y)}px`;
    el.style.visibility = "visible";
    buttons()[0]?.focus();
  }, [req]);

  // Esc, an outside press, scrolling or resizing closes it; focus goes back to the node or row
  const close = (restore = true) => {
    onClose();
    const from = req.from as HTMLElement | SVGElement | null;
    if (restore && from?.isConnected) from.focus({ preventScroll: true });
  };
  const closeRef = useRef(close);
  closeRef.current = close;
  useLayoutEffect(() => {
    const out = (e: Event) => { if (!root.current?.contains(e.target as Node)) closeRef.current(e.type !== "pointerdown"); };
    const resize = () => closeRef.current(false);
    window.addEventListener("pointerdown", out, true);
    window.addEventListener("wheel", out, { capture: true, passive: true });
    window.addEventListener("resize", resize);
    window.addEventListener("blur", resize);
    return () => {
      window.removeEventListener("pointerdown", out, true);
      window.removeEventListener("wheel", out, { capture: true });
      window.removeEventListener("resize", resize);
      window.removeEventListener("blur", resize);
    };
  }, []);

  const activate = (i: MenuItem) => { close(); i.run(); };

  const onKey = (e: React.KeyboardEvent) => {
    e.stopPropagation(); // the map's own single-key shortcuts must not fire behind the menu
    const all = buttons();
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const go = (i: number) => { e.preventDefault(); all[(i + all.length) % all.length]?.focus(); };
    if (e.key === "ArrowDown") return go(at + 1);
    if (e.key === "ArrowUp") return go(at - 1);
    if (e.key === "Home") return go(0);
    if (e.key === "End") return go(all.length - 1);
    if (e.key === "Escape" || isMenuKey(e)) { e.preventDefault(); return close(); }
    if (e.key === "Tab") { e.preventDefault(); return close(); }
    if (e.key === "Enter" || e.key === " ") { e.preventDefault(); const i = shown[at]; if (i) activate(i); return; }
    const k = shown.find((i) => i.key && i.key.toLowerCase() === e.key.toLowerCase());
    if (k && !e.metaKey && !e.ctrlKey && !e.altKey) { e.preventDefault(); activate(k); }
  };

  return (
    <div ref={root} className="map-menu" role="menu" aria-label={typeof title === "string" ? title : "Actions"}
      style={{ visibility: "hidden" }} onKeyDown={onKey} onContextMenu={(e) => e.preventDefault()}>
      <div className="map-menu-head" role="presentation">
        <b>{title}</b>
        <span>{sub}</span>
      </div>
      {shown.map((i) => {
        const Icon = i.icon;
        return (
          <button key={i.label} type="button" role="menuitem" tabIndex={-1} className="map-menu-item"
            aria-keyshortcuts={i.hint} onClick={() => activate(i)}
            onPointerMove={(e) => e.currentTarget.focus({ preventScroll: true })}>
            <Icon size={15} aria-hidden />
            <span>{i.label}</span>
            {i.hint && <kbd aria-hidden>{i.hint}</kbd>}
          </button>
        );
      })}
    </div>
  );
}
