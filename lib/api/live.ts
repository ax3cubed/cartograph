// FE-04: the shell's reads from the local API, as hooks. Components still speak the interim lib/model.ts shapes;
// to* below map the generated wire types onto them (lib/model.ts header) until FE-02's switch-over.
import { useEffect, useState, useSyncExternalStore } from "react";
import type { Commit as ApiCommit, Freshness as ApiFreshness, GraphSlice, Lod, Node as ApiNode, Repo as ApiRepo, SearchResults, WorkspaceSummary as ApiWorkspace } from "./types.gen.ts";
import type { Holder, Structure } from "../../components/map/levels.ts";
import { get, type Params, type RequestError } from "./client.ts";
import type { Freshness, WorkspaceSummary } from "../model.ts";
import { toFreshness, toSummary } from "./freshness.ts";
import { bump, generationOf, load, setGeneration, useGenerationTick, useResource, type Resource } from "./cache.ts";
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

export const wsRoute = (ws: string | undefined, tail: string) => (ws ? `/api/v1/w/${encodeURIComponent(ws)}/${tail}` : null);

/** The workspace's Freshness: fetched once, then whatever the event stream pushes. */
export function useFreshness(ws: string | undefined): Resource<Freshness> {
  const r = useResource<ApiFreshness>(ws ?? "", wsRoute(ws, "freshness"));
  useSyncExternalStore(subscribePushed, pushedVersion, () => 0);
  const f = (ws && pushedFreshness(ws)) || r.data;
  return { ...r, data: f ? toFreshness(f) : undefined };
}

export const useRepos = (ws: string | undefined) => useResource<ApiRepo[]>(ws ?? "", wsRoute(ws, "repos"));

/** One file-level slice at `at` (now, a date or a sha). */
/** One level's slice at `at` ("" = now); the file level unless `lod` says otherwise (SRV-04). */
export const useGraph = (ws: string | undefined, at: string, lod: Lod = "file") =>
  useResource<GraphSlice>(ws ?? "", wsRoute(ws, "graph"), lod === "file" ? { at: at || "now" } : { at: at || "now", lod });

// ---------- FE-08 to FE-10: map, time and panel reads ----------

export const nodeRoute = (ws: string | undefined, id: string | null | undefined, tail = "") =>
  (ws && id ? `/api/v1/w/${encodeURIComponent(ws)}/nodes/${encodeURIComponent(id)}${tail}` : null);

/** Any per-workspace read; `tail` null waits. */
export const useWsRead = <T,>(ws: string | undefined, tail: string | null, params?: Params) =>
  useResource<T>(ws ?? "", tail === null ? null : wsRoute(ws, tail), params);

/** An async read keyed by (workspace, generation, key): fetched again when the index changes. */
export function useLoaded<T>(ws: string | undefined, key: string | null, fn: () => Promise<T>): Resource<T> {
  const t = useGenerationTick();
  const k = ws && key ? `${ws}\u0000${generationOf(ws)}\u0000${key}` : "";
  const [state, setState] = useState<Resource<T> & { of?: string }>({ loading: !!k });
  useEffect(() => {
    if (!k) { setState({ loading: false }); return; }
    let live = true;
    setState((s) => ({ data: s.of === key ? s.data : undefined, loading: true, of: key ?? undefined }));
    fn().then(
      (data) => live && setState({ data, loading: false, of: key ?? undefined }),
      (error: RequestError) => live && setState((s) => ({ ...s, error, loading: false })),
    );
    return () => { live = false; };
    // k covers the workspace, the key and the generation
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [k, t]);
  return state;
}

/** Every page of a paged read, up to `max` rows. */
export async function loadAll<T>(ws: string, route: string, params: Params = {}, max = 200_000): Promise<T[]> {
  const out: T[] = [];
  let cursor: string | null | undefined;
  do {
    const r = await get<T[]>(route, { ...params, limit: 1000, cursor });
    if (r.generation > generationOf(ws)) setGeneration(ws, r.generation);
    out.push(...r.data);
    cursor = r.next_cursor;
  } while (cursor && out.length < max);
  return out;
}

/** Services and the modules inside each (FE-08 roll-ups), from …/nodes/{id}/children. */
export function useStructure(ws: string | undefined): Resource<Structure> {
  return useLoaded(ws, "structure", async () => {
    const top = await load<ApiNode[]>(ws!, nodeRoute(ws, "workspace", "/children")!, { limit: 1000 });
    const services = top.filter((n) => n.kind === "service");
    const inner = await Promise.all(services.map((s) => load<ApiNode[]>(ws!, nodeRoute(ws, s.id, "/children")!, { limit: 1000 })));
    const holder = (n: ApiNode, service: string): Holder => ({ id: n.id, kind: n.kind, name: n.name, path: n.path, service });
    return {
      services: services.map((s) => holder(s, s.id)),
      modules: inner.flatMap((list, k) => list.filter((n) => n.kind === "module").map((n) => holder(n, services[k].id))),
    };
  });
}

/** The merged first-parent timeline, oldest first (FE-09 snaps the knob to these). */
export const useCommits = (ws: string | undefined) =>
  useLoaded(ws, "commits", async () => (await loadAll<ApiCommit>(ws!, wsRoute(ws, "commits")!)).reverse());

/**
 * FE-10: …/search as you type, 120 ms after the last key, aborting the read it replaces. `results` is best first;
 * `near` is the "did you mean" when nothing matched.
 */
export function useSearch(ws: string | undefined, q: string, limit = 60): { data?: SearchResults; loading: boolean } {
  const [state, setState] = useState<{ data?: SearchResults; loading: boolean; of?: string }>({ loading: false });
  const term = q.trim();
  useEffect(() => {
    if (!ws || !term) { setState({ loading: false }); return; }
    const ac = new AbortController();
    setState((s) => ({ ...s, loading: true }));
    const t = setTimeout(() => {
      get<SearchResults>(wsRoute(ws, "search")!, { q: term, limit }, { signal: ac.signal }).then(
        (r) => setState({ data: r.data, loading: false, of: term }),
        (e: Error) => { if (e.name !== "AbortError") setState({ data: { results: [] }, loading: false, of: term }); },
      );
    }, 120);
    return () => { clearTimeout(t); ac.abort(); };
  }, [ws, term, limit]);
  return state;
}
