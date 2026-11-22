"use client";

import { useState } from "react";
import { CHUNKS, SECRET_HITS } from "@/lib/demo/settings";
import { Advanced, Chip, SettingsPage, Switch, useFileDraft } from "./ui";
import "./retrieval.css";

const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
interface Prefs { on: string[]; scan: boolean; embed: "nomic" | "none" }
const DEFAULTS: Prefs = { on: CHUNKS.filter((c) => c.on).map((c) => c.id), scan: true, embed: "nomic" };

export default function Retrieval() {
  const f = useFileDraft<Prefs>("cartograph:retrieval-v2", DEFAULTS);
  const p: Prefs = { ...DEFAULTS, ...f.value };
  const edit = (patch: Partial<Prefs>) => f.edit({ ...p, ...patch });
  const [confirm, setConfirm] = useState(false);
  const [msg, setMsg] = useState("");
  const entries = CHUNKS.filter((c) => p.on.includes(c.id)).reduce((n, c) => n + c.n, 0);
  const mins = Math.max(1, Math.ceil(entries / 4000));
  const embeds = p.embed === "nomic";

  return (
    <SettingsPage title="Retrieval and privacy" saves="file" file={{ count: f.count, onSave: f.save, onDiscard: f.discard }}
      intro="Choose what Ask and your editor can search, and what stays out.">
      <div className="set-grid">
        <section className="set-sec" aria-labelledby="set-ret-t">
          <h2 id="set-ret-t">What can be searched</h2>
          <p className="set-note">Ask and your editor look things up here. A search by meaning needs an embedding model. Without one, search matches words only.</p>
          <label className="set-label">Search by meaning
            <select className="set-select" value={p.embed} onChange={(e) => edit({ embed: e.target.value as Prefs["embed"] })}>
              <option value="nomic">nomic-embed-text, runs on this machine</option>
              <option value="none">Off, match words only</option>
            </select>
          </label>
          <div>
            <div className="set-sec-head"><span className="set-k">Include these</span><span className="set-k">Entries</span></div>
            {CHUNKS.map((c) => (
              <label key={c.id} className="set-check">
                <input type="checkbox" checked={p.on.includes(c.id)} onChange={(e) => edit({ on: e.target.checked ? [...p.on, c.id] : p.on.filter((x) => x !== c.id) })} />
                {c.label}<span className="set-mono set-muted">{fmt(c.n)}</span>
              </label>
            ))}
          </div>
          <div className="set-ret-sum">
            <span>{entries.toLocaleString("en")} entries. {embeds ? `A full build takes about ${mins} ${mins === 1 ? "minute" : "minutes"}. After that, only changed files are processed.` : "Nothing is embedded, so a rebuild is quick."}</span>
            <button type="button" className="set-btn" aria-expanded={confirm} aria-controls="set-ret-confirm" onClick={() => setConfirm(!confirm)}>Rebuild</button>
          </div>
          {confirm && (
            <div id="set-ret-confirm" className="set-confirm" role="alertdialog" aria-labelledby="set-ret-ct" aria-describedby="set-ret-cd">
              <b id="set-ret-ct">Rebuild search?</b>
              <p id="set-ret-cd">This processes all {entries.toLocaleString("en")} entries again{embeds ? ` and takes about ${mins} ${mins === 1 ? "minute" : "minutes"}` : ""}. Rebuild uses your saved choices, so save first if you changed any.</p>
              <div className="set-actions">
                <button type="button" className="set-btn is-primary" onClick={() => { setConfirm(false); setMsg("Rebuild confirmed. In this demo nothing is rebuilt until the local server runs it."); }}>Rebuild search</button>
                <button type="button" className="set-btn" onClick={() => setConfirm(false)}>Keep current search</button>
              </div>
            </div>
          )}
          {msg && <p className="set-note" role="status">{msg}</p>}
          <Advanced>
            <p className="set-note">Each entry is a small piece of text, such as a node card, a function body or one code change. Embedding model nomic-embed-text produces 768 dimensions per entry.</p>
          </Advanced>
        </section>

        <div className="set-col">
          <section className="set-sec" aria-labelledby="set-ret-s">
            <div className="set-sec-head"><h2 id="set-ret-s">Scan for secrets</h2><Switch on={p.scan} onChange={(scan) => edit({ scan })} labelledBy="set-ret-s" /></div>
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
