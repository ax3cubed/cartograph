"use client";

import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { TOAST_EVENT, takeQueue } from "./toast";
import "./toasts.css";

interface T { id: number; text: string }
let seq = 0;

/** Decision D5: toasts are announced to screen readers and Esc dismisses them (the bell keeps the history). */
export default function Toasts() {
  const [items, setItems] = useState<T[]>([]);
  const drop = (id: number) => setItems((l) => l.filter((t) => t.id !== id));
  useEffect(() => {
    const show = (text: string) => {
      const id = ++seq;
      setItems((l) => [...l.slice(-3), { id, text }]);
      setTimeout(() => drop(id), 6000);
    };
    const add = (e: Event) => show(String((e as CustomEvent).detail));
    takeQueue().forEach(show); // toasts queued before a navigation
    // Esc closes the newest toast, unless a dialog or menu is already using the key
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented && !document.querySelector("dialog[open], [role=menu], .sh-pop")) setItems((l) => l.slice(0, -1));
    };
    window.addEventListener(TOAST_EVENT, add);
    window.addEventListener("keydown", esc);
    return () => { window.removeEventListener(TOAST_EVENT, add); window.removeEventListener("keydown", esc); };
  }, []);
  return (
    <div className="sh-toasts" role="status" aria-live="polite" aria-relevant="additions">
      {items.map((t) => (
        <div key={t.id} className="sh-toast">
          <span>{t.text}</span>
          <button type="button" className="sh-toast-x" aria-label="Dismiss notification" onClick={() => drop(t.id)}><X size={14} aria-hidden /></button>
        </div>
      ))}
    </div>
  );
}
