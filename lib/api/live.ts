// FE-04: the shell's reads from the local API, as hooks. Components still speak the interim lib/model.ts shapes;
// to* below map the generated wire types onto them (lib/model.ts header) until FE-02's switch-over.
import { useSyncExternalStore } from "react";
import type { Freshness as ApiFreshness, GraphSlice, Repo as ApiRepo, WorkspaceSummary as ApiWorkspace } from "./types.gen.ts";
import type { Freshness, WorkspaceSummary } from "../model.ts";
import { toFreshness, toSummary } from "./freshness.ts";
import { bump, useResource, type Resource } from "./cache.ts";
import { pushedFreshness, pushedVersion, subscribePushed } from "./events.ts";
import { getMode, isUp, subscribe, type Mode } from "./session.ts";

export const useMode = (): Mode => useSyncExternalStore(subscribe, getMode, () => "checking");
export const useLive = () => useMode() === "live";
/** False while the binary doesn't answer. */
export const useUp = () => useSyncExternalStore(subscribe, isUp, () => true);

export { toFreshness, toSummary };

// ---------- the current workspace: the user's last pick, else the first in the registry ----------

const WS_KEY = "cartograph.workspace"; // an id, not a secret
const wsSubs = new Set<() => void>();
const readPick = () => { try { return localStorage.getItem(WS_KEY); } catch { return null; } };
export function pickWorkspace(id: string) {
  try { localStorage.setItem(WS_KEY, id); } catch { /* storage blocked: the pick lasts this page */ }
  picked = id;
  wsSubs.forEach((f) => f());
}
let picked: string | null = null;
/** Opens workspace `id` on the map. The router's navigations use pushState, which fires no hashchange, so the
 * wizard and the first-run list call this before they push /#added=<id> or /#ws=<id>. */
export function openWorkspace(id: string) {
  if (id !== picked) { pickWorkspace(id); bump(""); } // a new workspace isn't in the list read before it
}
// A /#added=<id> or /#ws=<id> typed, reloaded or opened in a new tab: open that workspace.
const fromHash = () => {
  const p = new URLSearchParams(location.hash.slice(1));
  const id = p.get("ws") ?? p.get("added");
  if (id) openWorkspace(id);
};
if (typeof window !== "undefined") { fromHash(); window.addEventListener("hashchange", fromHash); }
const usePick = () => useSyncExternalStore((f) => { wsSubs.add(f); return () => { wsSubs.delete(f); }; }, () => picked ?? (picked = readPick()), () => null);

export function useWorkspaces(): Resource<WorkspaceSummary[]> & { current?: WorkspaceSummary } {
  const live = useLive();
  const r = useResource<ApiWorkspace[]>("", live ? "/api/v1/workspaces" : null);
  const pick = usePick();
  const list = r.data?.map(toSummary);
  return { ...r, data: list, current: list?.find((w) => w.id === pick) ?? list?.[0] };
}

const wsRoute = (ws: string | undefined, tail: string) => (ws ? `/api/v1/w/${encodeURIComponent(ws)}/${tail}` : null);

/** The workspace's Freshness: fetched once, then whatever the event stream pushes. */
export function useFreshness(ws: string | undefined): Resource<Freshness> {
  const r = useResource<ApiFreshness>(ws ?? "", wsRoute(ws, "freshness"));
  useSyncExternalStore(subscribePushed, pushedVersion, () => 0);
  const f = (ws && pushedFreshness(ws)) || r.data;
  return { ...r, data: f ? toFreshness(f) : undefined };
}

export const useRepos = (ws: string | undefined) => useResource<ApiRepo[]>(ws ?? "", wsRoute(ws, "repos"));

/** One file-level slice at `at` (now, a date or a sha). */
export const useGraph = (ws: string | undefined, at: string) => useResource<GraphSlice>(ws ?? "", wsRoute(ws, "graph"), { at: at || "now" });
