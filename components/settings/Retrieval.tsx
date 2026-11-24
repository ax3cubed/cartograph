"use client";

import { useState } from "react";
import { CHUNKS, SECRET_HITS } from "@/lib/demo/settings";
import { readRetrieval, setRetrieval } from "@/lib/api/settings";
import { Advanced, Chip, FileState, SettingsPage, Switch, useFileDraft, type YamlMap } from "./ui";
import "./retrieval.css";
import { fmtNum } from "@/lib/date";

const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
interface Prefs { on: string[]; scan: boolean }
const DEFAULTS: Prefs = { on: CHUNKS.filter((c) => c.on).map((c) => c.id), scan: true };
const same = (a: string[], b: string[]) => a.length === b.length && a.every((x) => b.includes(x));

/** FE-13: `retrieval:` in cartograph.yaml. A key is written only while it differs from the default, so the file stays
 * as short as the choices. No embedding model (G7): search matches words. */
const YAML: YamlMap<Prefs> = {
  read: (y) => { const r = readRetrieval(y); return { ...(r.include && { on: r.include }), ...(r.secret_scan !== undefined && { scan: r.secret_scan }) }; },
  write: (y, v) => setRetrieval(y, {
    include: same(v.on, DEFAULTS.on) ? undefined : CHUNKS.map((c) => c.id).filter((id) => v.on.includes(id)),
    secret_scan: v.scan ? undefined : false,
  }),
};

export default function Retrieval() {
  const f = useFileDraft<Prefs>("", DEFAULTS, YAML);
  const p: Prefs = { ...DEFAULTS, ...f.value };
  const edit = (patch: Partial<Prefs>) => f.edit({ ...p, ...patch });
  const [confirm, setConfirm] = useState(false);
  const [msg, setMsg] = useState("");
  const entries = CHUNKS.filter((c) => p.on.includes(c.id)).reduce((n, c) => n + c.n, 0);

  return (
    <SettingsPage title="Retrieval and privacy" saves="file" file={{ count: f.count, onSave: f.save, onDiscard: f.discard, ...f.file }}
      intro="Choose what Ask and your editor can search, and what stays out.">
      <FileState f={f.file} />
      <div className="set-grid">
        <section className="set-sec" aria-labelledby="set-ret-t">
          <h2 id="set-ret-t">What can be searched</h2>
          <p className="set-note">Ask and your editor look things up here. Search matches words, names and paths; nothing is sent to a model to build it.</p>
          <div>
            <div className="set-sec-head"><span className="set-k">Include these</span><span className="set-k">Entries</span></div>
            {CHUNKS.map((c) => (
              <label key={c.id} className="set-check">
                <input type="checkbox" checked={p.on.includes(c.id)} disabled={f.file.readOnly} onChange={(e) => edit({ on: e.target.checked ? [...p.on, c.id] : p.on.filter((x) => x !== c.id) })} />
                {c.label}<span className="set-mono set-muted">{fmt(c.n)}</span>
              </label>
            ))}
          </div>
          <div className="set-ret-sum">
            <span>{fmtNum(entries)} entries. Nothing is embedded, so a rebuild is quick.</span>
            <button type="button" className="set-btn" aria-expanded={confirm} aria-controls="set-ret-confirm" onClick={() => setConfirm(!confirm)}>Rebuild</button>
          </div>
          {confirm && (
            <div id="set-ret-confirm" className="set-confirm" role="alertdialog" aria-labelledby="set-ret-ct" aria-describedby="set-ret-cd">
              <b id="set-ret-ct">Rebuild search?</b>
              <p id="set-ret-cd">This processes all {fmtNum(entries)} entries again. Rebuild uses your saved choices, so save first if you changed any.</p>
              <div className="set-actions">
                <button type="button" className="set-btn is-primary" onClick={() => { setConfirm(false); setMsg("Rebuild confirmed. In this demo nothing is rebuilt until the local server runs it."); }}>Rebuild search</button>
                <button type="button" className="set-btn" onClick={() => setConfirm(false)}>Keep current search</button>
              </div>
            </div>
          )}
          {msg && <p className="set-note" role="status">{msg}</p>}
          <Advanced>
            <p className="set-note">Each entry is a small piece of text, such as a node card, a function body or one code change. Search matches the words in it; no embedding model is used.</p>
          </Advanced>
        </section>

        <div className="set-col">
          <section className="set-sec" aria-labelledby="set-ret-s">
            <div className="set-sec-head"><h2 id="set-ret-s">Scan for secrets</h2><Switch on={p.scan} disabled={f.file.readOnly} onChange={(scan) => edit({ scan })} labelledBy="set-ret-s" /></div>
            <p className="set-note">Anything that looks like a key or password is left out of search, summaries and every model prompt.</p>
            {p.scan && (
              <ul className="set-ret-hits">
                {SECRET_HITS.map((h) => (
                  <li key={h.id}><span><span className="set-mono">{h.mask}</span> · {h.what} <span className="set-mono">{h.file}</span>{h.year && ` · ${h.year}`}</span><Chip tone="ok">Left out</Chip></li>
                ))}
              </ul>
            )}
            <span className="set-note">Uses the gitleaks rules plus 2 custom patterns. Rotate any key found here.</span>
          </section>
          <section className="set-sec" aria-labelledby="set-ret-a">
            <h2 id="set-ret-a">Who can read this index</h2>
            <div className="set-ret-who">
              <span><b>Only this machine</b><small>The index file stays local. The editor server only accepts connections from this machine.</small></span><Chip tone="ok">Active</Chip>
            </div>
            <div className="set-ret-who">
              <span><b>Sharing with a teammate</b><small>Cartograph has no server. Commit <span className="set-mono">cartograph.yaml</span> and each person builds the same index on their own machine.</small></span>
            </div>
          </section>
        </div>
      </div>
    </SettingsPage>
  );
}
