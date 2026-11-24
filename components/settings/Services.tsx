"use client";

import { useState } from "react";
import { useLive, useRepos, useWorkspaces, wsRoute } from "@/lib/api/live";
import { useResource } from "@/lib/api/cache";
import { setService, type ServiceEntry } from "@/lib/api/settings";
import type { Service } from "@/lib/api";
import { foundVia } from "@/lib/settings";
import { getRepos, getServices } from "@/lib/source";
import { FileState, SettingsPage, useFileDraft, type YamlMap } from "./ui";
import "./services.css";

/** Splits waiting to be written, by new service name; each is a services: entry in cartograph.yaml (C7). */
type Splits = Record<string, ServiceEntry>;
const SPLITS: YamlMap<Splits> = { read: () => ({}), write: (y, v) => Object.values(v).reduce(setService, y) };

/**
 * FE-13: the services the index found (IDX-20), each with the C6 rule that found it, and a split written to
 * cartograph.yaml as a services: entry. Merges and domains are override-file edits (phase 4), so they aren't offered
 * here yet. Without the binary the same page reads the demo workspace's services.
 */
export default function Services() {
  const live = useLive();
  const { current } = useWorkspaces();
  const ws = live ? current?.id : undefined;
  const found = useResource<Service[]>(ws ?? "", wsRoute(ws, "services"));
  const liveRepos = useRepos(ws).data;
  const services = (live ? found.data : getServices()) ?? [];
  const repos: { id: string; name: string }[] = (live ? liveRepos : getRepos()) ?? [];
  const f = useFileDraft<Splits>("", {}, SPLITS);
  const [split, setSplit] = useState<{ name: string; path: string; repo: string } | null>(null);
  const pending = Object.values(f.value);
  const start = (s: Service) => {
    const root = s.paths[0] ?? "";
    setSplit({ name: `${root.split("/").pop() || s.name}-part`, path: root ? `${root}/` : "", repo: s.repo });
  };

  return (
    <SettingsPage title="Services and domains" saves="file"
      file={{ count: f.count, onSave: f.save, onDiscard: f.discard, ...f.file }}
      scope={<>A split is written to <span className="set-mono">cartograph.yaml</span> when you press Save. Merging services and grouping them into domains arrive with the override file.</>}
      intro="See which services Cartograph found and how it found each one, and split one by path.">
      <FileState f={f.file} />
      <div className="set-svc-layout">
        <section className="set-sec" aria-labelledby="set-svc-h">
          <h2 id="set-svc-h">{services.length} {services.length === 1 ? "service" : "services"}</h2>
          {live && found.error && <p className="set-note" role="alert">{found.error.what}. {found.error.try_}.</p>}
          {split && (
            <form className="set-svc-split" onSubmit={(e) => { e.preventDefault(); f.edit({ [split.name]: { name: split.name, repo: split.repo, path: split.path.replace(/\/$/, "") || undefined } }); setSplit(null); }}>
              <label className="set-label">New service name<input className="set-input" required value={split.name} onChange={(e) => setSplit({ ...split, name: e.target.value })} /></label>
              <label className="set-label">Path in the repository<input className="set-input is-mono" required value={split.path} onChange={(e) => setSplit({ ...split, path: e.target.value })} /></label>
              {repos.length > 1 && (
                <label className="set-label">Repository
                  <select className="set-select" value={split.repo} onChange={(e) => setSplit({ ...split, repo: e.target.value })}>{repos.map((r) => <option key={r.id}>{r.name}</option>)}</select>
                </label>
              )}
              <div className="set-actions"><button type="submit" className="set-btn is-primary">Add split</button><button type="button" className="set-btn" onClick={() => setSplit(null)}>Cancel</button></div>
            </form>
          )}
          <ul className="set-svc-list">
            <li className="set-svc-row is-found" aria-hidden><span className="set-k">Service</span><span className="set-k set-svc-how">Found from</span><span /></li>
            {services.map((s) => (
              <li key={s.id} className="set-svc-row is-found">
                <span className="set-svc-name"><b>{s.name}</b><small className="set-mono">{s.paths[0] ? `${s.repo}/${s.paths[0]}` : `${s.repo} (repository root)`}</small></span>
                <span className="set-svc-how">{foundVia(s.detected_by, s.library)}</span>
                <button type="button" className="set-btn is-sm" disabled={f.file.readOnly} aria-label={`Split ${s.name}`} onClick={() => start(s)}>Split</button>
              </li>
            ))}
          </ul>
        </section>
        <aside className="set-col">
          <section className="set-sec" aria-labelledby="set-pend-h">
            <h2 id="set-pend-h">Unsaved splits</h2>
            {pending.length ? <ul className="set-svc-doms">{pending.map((p) => <li key={p.name}><i aria-hidden /><span>{p.name}</span><span className="set-mono set-muted">{p.path ?? p.repo}</span></li>)}</ul>
              : <p className="set-note">Nothing to save. Split a service to preview the file edit in the bar below.</p>}
          </section>
        </aside>
      </div>
    </SettingsPage>
  );
}
