"use client";

import { useEffect, useMemo, useState } from "react";
import type { Freshness } from "@/lib/model";
import { freshnessFor } from "@/lib/demo/shell";

export type DemoIndexState = "loading" | "failed" | "stale" | null;
const read = (): { state: DemoIndexState; why: string | null } => {
  const p = new URLSearchParams(location.hash.slice(1));
  const s = p.get("state");
  return { state: s === "loading" || s === "failed" || s === "stale" ? s : null, why: p.get("why") };
};

/** Reads #state=loading|failed|stale (and #why=schema for the failed state) so each G02 state can be shown in the demo. */
export function useIndexState() {
  const [v, setV] = useState<{ state: DemoIndexState; why: string | null }>({ state: null, why: null });
  useEffect(() => {
    const sync = () => setV(read());
    sync();
    window.addEventListener("hashchange", sync);
    return () => window.removeEventListener("hashchange", sync);
  }, []);
  /** Clears the state from the hash and keeps the rest of it. */
  const clear = () => {
    const p = new URLSearchParams(location.hash.slice(1));
    p.delete("state"); p.delete("why");
    const rest = p.toString();
    history.replaceState(null, "", location.pathname + location.search + (rest ? `#${rest}` : ""));
    setV({ state: null, why: null });
  };
  /** The single Freshness every status surface reads; undefined means the workspace's own. */
  const freshness = useMemo<Freshness | undefined>(() => (v.state === "stale" || v.state === "failed" ? freshnessFor(v.state) : undefined), [v.state]);
  return { ...v, freshness, clear };
}
