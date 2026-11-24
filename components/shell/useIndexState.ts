"use client";

import { useEffect, useMemo, useState } from "react";
import type { Freshness } from "@/lib/model";
import { getDemoFreshness } from "@/lib/source";
import { useFreshness, useLive, useUp, useWorkspaces } from "@/lib/api/live";

export type DemoIndexState = "loading" | "failed" | "stale" | null;
const read = (): { state: DemoIndexState; why: string | null } => {
  const p = new URLSearchParams(location.hash.slice(1));
  const s = p.get("state");
  return { state: s === "loading" || s === "failed" || s === "stale" ? s : null, why: p.get("why") };
};

const failed = (what: string, action: string): Freshness => ({ state: "failed", indexVersion: 0, builtAt: "", behind: 0, error: { what, action } });

/**
 * The single Freshness every status surface reads (chip, popover, band, Activity), plus the overlay state.
 * With the binary (FE-04) it is the current workspace's /freshness, then whatever the event stream pushes; nothing
 * reads the URL. In the demo, #state=loading|failed|stale (and #why=schema) still shows each G02 state.
 */
// ponytail: the demo's hash switches stay until the demo build gets its own fixtures (FE-04 Done when, phase 3).
export function useIndexState() {
  const live = useLive();
  const up = useUp();
  const { current } = useWorkspaces();
  const fr = useFreshness(live ? current?.id : undefined);
  const [dismissed, setDismissed] = useState(false);

  const [v, setV] = useState<{ state: DemoIndexState; why: string | null }>({ state: null, why: null });
  useEffect(() => {
    if (live) return;
    const sync = () => setV(read());
    sync();
    window.addEventListener("hashchange", sync);
    return () => window.removeEventListener("hashchange", sync);
  }, [live]);
  /** Clears the state from the hash and keeps the rest of it. */
  const clear = () => {
    if (live) { setDismissed(true); return; }
    const p = new URLSearchParams(location.hash.slice(1));
    p.delete("state"); p.delete("why");
    const rest = p.toString();
    history.replaceState(null, "", location.pathname + location.search + (rest ? `#${rest}` : ""));
    setV({ state: null, why: null });
  };
  const demoFreshness = useMemo<Freshness | undefined>(() => (v.state === "stale" || v.state === "failed" ? getDemoFreshness(v.state) : undefined), [v.state]);

  if (!live) return { ...v, freshness: demoFreshness, clear, loading: v.state === "loading", offline: false };
  const freshness = !up ? failed("Cartograph isn't running", "Start it again from the terminal.")
    : fr.data ?? (fr.error ? failed(fr.error.what, fr.error.try_) : undefined);
  return {
    state: (freshness?.state === "behind" && !dismissed ? "stale" : null) as DemoIndexState,
    why: null, freshness, clear, loading: !freshness, offline: !up,
  };
}
