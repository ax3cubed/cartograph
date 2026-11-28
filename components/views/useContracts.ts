"use client";

// The specs of a workspace and each one's versions: what the Contracts view lists and the Coverage view's risk card
// counts. Versions are read once per spec and kept by the generation cache.
import { useMemo } from "react";
import type { Contract, ContractVersion } from "@/lib/api";
import { load } from "@/lib/api/cache";
import { useLoaded, useWsRead, wsRoute } from "@/lib/api/live";

export function useContracts(ws: string | undefined) {
  const res = useWsRead<Contract[]>(ws, "contracts");
  const specs = useMemo(() => res.data ?? [], [res.data]);
  const versions = useLoaded<Record<string, ContractVersion[]>>(ws, specs.length ? `versions:${specs.map((s) => s.id).join(",")}` : null, async () =>
    Object.fromEntries(await Promise.all(specs.map(async (s) => [s.id, await load<ContractVersion[]>(ws!, wsRoute(ws, `contracts/${encodeURIComponent(s.id)}/versions`)!)] as const))));
  return { specs, versions: versions.data, loading: res.loading, error: res.error };
}
