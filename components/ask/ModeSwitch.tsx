"use client";

import { useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { useRouter } from "next/navigation";
import { usePath } from "../shell/usePath";
import { Check, ChevronDown, Maximize2, PanelRight, PictureInPicture2 } from "lucide-react";
import { ASK_EVENT } from "../shell/AppShell";
import { askStore, focusComposer } from "./store";

export type AskPlace = "docked" | "floating" | "page";

const PLACES = [
  { id: "docked", label: "Sidebar", hint: "Beside the map", icon: PanelRight },
  { id: "floating", label: "Floating", hint: "A panel over any view", icon: PictureInPicture2 },
  { id: "page", label: "Full page", hint: "Room for long chats", icon: Maximize2 },
] as const;

/** Window event that closes the map's sidebar: the chat left it, so the map gets the room back. */
export const SIDEBAR_CLOSE_EVENT = "cartograph:sidebar-close";

/** Moves the chat. The thread stays the same; only where it shows changes. */
export function useMoveAsk() {
  const router = useRouter();
  const raw = usePath();
  const path = raw.length > 1 ? raw.replace(/\/$/, "") : raw;
  return (to: AskPlace) => {
    const fromSidebar = path === "/" && askStore.get().mode === "docked";
    if (to !== "docked" && fromSidebar) window.dispatchEvent(new Event(SIDEBAR_CLOSE_EVENT));
    if (to === "page") {
      if (path !== "/ask") askStore.setReturnTo(location.pathname + location.hash);
      router.push("/ask");
      return;
    }
    askStore.setMode(to);
    if (to === "docked") {
      // the sidebar exists only on the map; #ask opens it on Ask and focuses the composer
      if (path === "/") window.dispatchEvent(new Event(ASK_EVENT));
      else router.push("/#ask");
      return;
    }
    if (path === "/ask") router.push(askStore.get().returnTo ?? "/");
    focusComposer();
  };
}

/** "Open in": a menu of the places the chat can show, the current one checked. Shown in every place. */
export default function ModeSwitch({ current }: { current: AskPlace }) {
  const move = useMoveAsk();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const id = useId();
  const cur = PLACES.find((p) => p.id === current)!;

  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false); };
    document.addEventListener("mousedown", away);
    // focus the checked item when the menu opens
    requestAnimationFrame(() => ref.current?.querySelector<HTMLElement>('[role="menuitemradio"][aria-checked="true"]')?.focus());
    return () => document.removeEventListener("mousedown", away);
  }, [open]);

  const close = () => { setOpen(false); ref.current?.querySelector<HTMLElement>("[aria-haspopup]")?.focus(); };
  const onKey = (e: ReactKeyboardEvent) => {
    if (!open) return;
    const items = [...(ref.current?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? [])];
    const at = items.indexOf(document.activeElement as HTMLElement);
    const step = { ArrowDown: 1, ArrowUp: -1 }[e.key];
    if (step) { e.preventDefault(); items[(at + step + items.length) % items.length]?.focus(); }
    else if (e.key === "Home" || e.key === "End") { e.preventDefault(); items[e.key === "Home" ? 0 : items.length - 1]?.focus(); }
    else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); } // close the menu, not the floating panel
    else if (e.key === "Tab") setOpen(false);
  };

  return (
    <div className="ask-modes" ref={ref} onKeyDown={onKey}>
      <button type="button" className="ask-modes-btn" aria-haspopup="menu" aria-expanded={open} aria-controls={open ? id : undefined}
        aria-label={`Open in: ${cur.label}`} title="Open in…" onClick={() => setOpen((o) => !o)}>
        <cur.icon size={15} aria-hidden /><ChevronDown size={12} aria-hidden />
      </button>
      {open && (
        <div className="ask-modes-pop" role="menu" id={id} aria-label="Open the chat in">
          <p className="ask-modes-k" aria-hidden>Open in</p>
          {PLACES.map(({ id: place, label, hint, icon: Icon }) => (
            <button key={place} type="button" role="menuitemradio" aria-checked={current === place} tabIndex={-1} className="ask-modes-item"
              onClick={() => { setOpen(false); if (current !== place) move(place); }}>
              <Icon size={15} aria-hidden />
              <span><b>{label}</b><span>{hint}</span></span>
              {current === place && <Check size={14} aria-hidden className="ask-modes-check" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
