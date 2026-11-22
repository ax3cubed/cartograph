"use client";

import { useEffect, useState } from "react";
import { X } from "lucide-react";
import "./addedHint.css";

const SEEN = "cartograph.start-hint-seen";

/**
 * One-time "Start here" hint on the map after the add-workspace flow ends at /#added=northwind.
 * Mount it once inside the map page (any element; it positions itself). It removes #added from the URL and never shows twice.
 */
export default function AddedHint() {
  const [show, setShow] = useState(false);
  useEffect(() => {
    const check = () => {
      const p = new URLSearchParams(location.hash.slice(1));
      if (!p.has("added")) return;
      p.delete("added");
      const rest = p.toString();
      history.replaceState(null, "", location.pathname + location.search + (rest ? `#${rest}` : ""));
      try { if (localStorage.getItem(SEEN)) return; localStorage.setItem(SEEN, "1"); } catch { /* storage can be blocked: show it anyway */ }
      setShow(true);
    };
    check();
    window.addEventListener("hashchange", check);
    return () => window.removeEventListener("hashchange", check);
  }, []);
  if (!show) return null;
  return (
    <aside className="ah" role="note" aria-label="Start here">
      <div>
        <b>Start here</b>
        <p>Click a service to see what it depends on and what depends on it. Press <kbd>⌘K</kbd> to search.</p>
      </div>
      <button type="button" className="ah-x" aria-label="Dismiss hint" onClick={() => setShow(false)}><X size={14} aria-hidden /></button>
    </aside>
  );
}
