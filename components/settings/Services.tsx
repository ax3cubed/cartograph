"use client";

import { useState } from "react";
import { DOMAINS, SERVICE_DETECT } from "@/lib/demo/settings";
import { SERVICES } from "@/lib/workspace";
import { useLive, useRepos, useWorkspaces } from "@/lib/api/live";
import { useResource } from "@/lib/api/cache";
import { setService, type ServiceEntry } from "@/lib/api/settings";
import type { Node } from "@/lib/api";
import { FileState, SettingsPage, useFileDraft, type YamlMap } from "./ui";
import "./services.css";

interface Svc { id: string; name: string; path: string; how: string; domain: string; owner: string; edited: boolean }
type Change = { kind: "domain"; id: string; to: string } | { kind: "merge"; ids: string[] } | { kind: "split"; id: string; name: string; path: string };

const START: Svc[] = SERVICES.map((s) => ({ id: s.id, name: s.name, path: s.name, ...SERVICE_DETECT[s.id], edited: false }));

/** The first six domains take the six fixed hues in order; later ones are neutral. */
const hue = (domains: string[], d: string) => SERVICES[domains.indexOf(d)]?.color ?? "var(--faint)";

export default function Services() {
  return useLive() ? <LiveServices /> : <DemoServices />;
}

/** Splits waiting to be written, by new service name; each is a services: entry in cartograph.yaml (C7). */
type Splits = Record<string, ServiceEntry>;
const SPLITS: YamlMap<Splits> = { read: () => ({}), write: (y, v) => Object.values(v).reduce(setService, y) };

/**
 * FE-13 with the binary: the services the index found, and a split written to cartograph.yaml as a services: entry.
 * Merges and domains are override-file edits (phase 4), so they aren't offered here yet.
 */
function LiveServices() {
  const { current } = useWorkspaces();
  const ws = current?.id;
  const found = useResource<Node[]>(ws ?? "", ws ? `/api/v1/w/${encodeURIComponent(ws)}/nodes/workspace/children` : null, { limit: 1000 });
  const repos = useRepos(ws).data ?? [];
  const services = (found.data ?? []).filter((n) => n.kind === "service");
  const f = useFileDraft<Splits>("", {}, SPLITS);
  const [split, setSplit] = useState<{ from: string; name: string; path: string; repo: string } | null>(null);
  const pending = Object.values(f.value);
  const start = (n: Node) => setSplit({ from: n.id, name: `${n.path.split("/").pop() || n.name}-part`, path: n.path ? `${n.path}/` : "", repo: repos[0]?.name ?? "" });

  return (
    <SettingsPage title="Services and domains" saves="file"
      file={{ count: f.count, onSave: f.save, onDiscard: f.discard, ...f.file }}
      scope={<>A split is written to <span className="set-mono">cartograph.yaml</span> when you press Save. Merging services and grouping them into domains arrive with the override file.</>}
      intro="See which services Cartograph found, and split one by path.">
      <FileState f={f.file} />
      <div className="set-svc-layout">
        <section className="set-sec" aria-labelledby="set-svc-h">
          <h2 id="set-svc-h">{services.length} services</h2>
          {found.error && <p className="set-note" role="alert">{found.error.what}. {found.error.try_}.</p>}
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
            {services.map((n) => (
              <li key={n.id} className="set-svc-row is-live">
                <span className="set-svc-name"><b>{n.name}</b><small className="set-mono">{n.path || "(repository root)"}</small></span>
                <button type="button" className="set-btn is-sm" disabled={f.file.readOnly} aria-label={`Split ${n.name}`} onClick={() => start(n)}>Split</button>
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

function DemoServices() {
  const [rows, setRows] = useState(START);
  const [domains, setDomains] = useState(DOMAINS);
  const [sel, setSel] = useState<string[]>([]);
  const [pending, setPending] = useState<Change[]>([]);
  const [split, setSplit] = useState<{ name: string; path: string } | null>(null);
  const [newDomain, setNewDomain] = useState<string | null>(null);
  const [msg, setMsg] = useState("");
  const one = sel.length === 1 ? rows.find((r) => r.id === sel[0])! : undefined;
  const nameOf = (id: string) => rows.find((r) => r.id === id)?.name ?? id;

  const yaml = pending.flatMap((c) =>
    c.kind === "split" ? ["+ services:", `+   - name: ${c.name}`, `+     path: ${c.path}`]
    : c.kind === "merge" ? [`+ - merge: [${c.ids.map(nameOf).join(", ")}]`, `+   into: ${rows.find((r) => r.id === c.ids[0])!.path}`]
    : [`+ - domain: ${c.to}`, `+   services: [${nameOf(c.id)}]`]);

  const setDomain = (id: string, to: string) => {
    const cur = rows.find((r) => r.id === id)!.domain;
    setPending([...pending.filter((c) => !(c.kind === "domain" && c.id === id)), ...(to === cur ? [] : [{ kind: "domain" as const, id, to }])]);
  };
  const shown = (r: Svc) => { const c = pending.find((x) => x.kind === "domain" && x.id === r.id); return c?.kind === "domain" ? c.to : r.domain; };

  const save = () => {
    let next = rows;
    for (const c of pending) {
      if (c.kind === "domain") next = next.map((r) => (r.id === c.id ? { ...r, domain: c.to, edited: true } : r));
      if (c.kind === "merge") next = next.filter((r) => r.id === c.ids[0] || !c.ids.includes(r.id)).map((r) => (r.id === c.ids[0] ? { ...r, how: `merged from ${c.ids.slice(1).map(nameOf).join(", ")}`, edited: true } : r));
      if (c.kind === "split") {
        const from = next.find((r) => r.id === c.id)!;
        next = [...next, { ...from, id: c.name, name: c.name, path: c.path, how: "cartograph.yaml services entry", edited: true }];
      }
    }
    setRows(next); setPending([]); setSel([]);
    setMsg("Saved. Splits went to cartograph.yaml, merges and domain changes to cartograph.overrides.yaml. In this demo they stay on this page.");
  };
  const counts = (d: string) => rows.filter((r) => shown(r) === d).length;

  return (
    <SettingsPage title="Services and domains" saves="file"
      file={{ count: pending.length, onSave: save, onDiscard: () => setPending([]), target: "cartograph.yaml and cartograph.overrides.yaml" }}
      scope={<>A split is written to <span className="set-mono">cartograph.yaml</span>. Merges and domain changes go to <span className="set-mono">cartograph.overrides.yaml</span>. Nothing is written until you press Save.</>}
      intro="See how each service was found, and group services into domains. Split a service by path, or merge several into one.">
      <div className="set-svc-layout">
        <section className="set-sec" aria-labelledby="set-svc-h">
          <div className="set-sec-head">
            <h2 id="set-svc-h">{rows.length} services · {sel.length} selected</h2>
            <div className="set-actions">
              <button type="button" className="set-btn" disabled={!one} title={one ? undefined : "Select one service to split it by path"} onClick={() => setSplit({ name: `${one!.name}-routes`, path: `${one!.path}/routes` })}>Split</button>
              <button type="button" className="set-btn" disabled={sel.length < 2} onClick={() => { setPending([...pending, { kind: "merge", ids: sel }]); setSel([]); }}>Merge {sel.length > 1 ? `${sel.length} services` : ""}</button>
            </div>
          </div>
          {split && one && (
            <form className="set-svc-split" onSubmit={(e) => { e.preventDefault(); setPending([...pending, { kind: "split", id: one.id, ...split }]); setSplit(null); setSel([]); }}>
              <label className="set-label">New service name<input className="set-input" required value={split.name} onChange={(e) => setSplit({ ...split, name: e.target.value })} /></label>
              <label className="set-label">Path inside {one.name}<input className="set-input is-mono" required value={split.path} onChange={(e) => setSplit({ ...split, path: e.target.value })} /></label>
              <div className="set-actions"><button type="submit" className="set-btn is-primary">Add split</button><button type="button" className="set-btn" onClick={() => setSplit(null)}>Cancel</button></div>
            </form>
          )}
          <ul className="set-svc-list">
            <li className="set-svc-row" aria-hidden><span /><span className="set-k">Service</span><span className="set-k">Detected from</span><span className="set-k">Domain</span><span className="set-k">Owner</span></li>
            {rows.map((r) => (
              <li key={r.id} className="set-svc-row">
                <input type="checkbox" aria-label={`Select ${r.name}`} checked={sel.includes(r.id)} onChange={(e) => setSel(e.target.checked ? [...sel, r.id] : sel.filter((s) => s !== r.id))} />
                <span className="set-svc-name"><b>{r.name}{r.edited && <i className="set-svc-edit" role="img" aria-label="Edited since detection" title="Edited since detection" />}</b><small className="set-mono">{r.path}</small></span>
                <span className="set-svc-how">{r.how}</span>
                <span className="set-svc-dom"><i style={{ background: hue(domains, shown(r)) }} aria-hidden />
                  <select className="set-select" aria-label={`Domain for ${r.name}`} value={shown(r)} onChange={(e) => setDomain(r.id, e.target.value)}>{domains.map((d) => <option key={d}>{d}</option>)}</select>
                </span>
                <span className="set-svc-own" title={r.owner}>{r.owner}</span>
              </li>
            ))}
          </ul>
        </section>

        <aside className="set-col">
          <section className="set-sec" aria-labelledby="set-dom-h">
            <div className="set-sec-head"><h2 id="set-dom-h">Domains</h2><button type="button" className="set-btn is-sm" onClick={() => setNewDomain(newDomain === null ? "" : null)}>Add domain</button></div>
            {newDomain !== null && (
              <form className="set-actions" onSubmit={(e) => { e.preventDefault(); const n = newDomain.trim(); if (n && !domains.includes(n)) setDomains([...domains, n]); setNewDomain(null); }}>
                <input className="set-input" aria-label="Domain name" value={newDomain} onChange={(e) => setNewDomain(e.target.value)} />
                <button type="submit" className="set-btn is-primary is-sm">Add</button>
              </form>
            )}
            <ul className="set-svc-doms">
              {domains.map((d) => <li key={d}><i style={{ background: hue(domains, d) }} aria-hidden /><span>{d}</span><span className="set-mono set-muted" aria-label={`${counts(d)} services`}>{counts(d)}</span></li>)}
            </ul>
            <p className="set-note">The first six domains use the six fixed colours, in order.</p>
          </section>
          <section className="set-sec" aria-labelledby="set-pend-h">
            <h2 id="set-pend-h">Unsaved edits</h2>
            {yaml.length ? <pre className="set-pre set-svc-diff" aria-label="YAML diff">{yaml.join("\n")}</pre> : <p className="set-note">Nothing to save. Split, merge or change a domain to preview the file edit here.</p>}
            <p className="set-note" role="status">{msg}</p>
          </section>
        </aside>
      </div>
    </SettingsPage>
  );
}
