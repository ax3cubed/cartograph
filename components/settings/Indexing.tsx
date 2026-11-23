"use client";

import { useEffect, useState } from "react";
import { Check, CircleCheck } from "lucide-react";
import { DEFAULT_AI, loadAi } from "@/lib/ai";
import { DEFAULT_EXCLUDES, EXTRACTORS, RUNS } from "@/lib/demo/settings";
import { NODES } from "@/lib/workspace";
import { getCurrentWorkspace, getFreshness } from "@/lib/source";
import { useFreshness, useLive, useWorkspaces } from "@/lib/api/live";
import { RequestError, send } from "@/lib/api/client";
import { readExcludes, setExcludes } from "@/lib/api/settings";
import { fmtDate } from "@/lib/date";
import { Advanced, Chip, FileState, SettingsPage, Switch, useFileDraft, type YamlMap } from "./ui";
import "./indexing.css";

interface Summaries { on: boolean; model: string; scope: "modules" | "files" }
interface Prefs { hooks: boolean; fetch: boolean; excludes: string; off: string[]; summ: Summaries }
const DEFAULTS: Prefs = { hooks: false, fetch: false, excludes: DEFAULT_EXCLUDES, off: EXTRACTORS.filter((x) => !x.on).map((x) => x.id), summ: { on: false, model: "", scope: "modules" } };

/** FE-13: "Left out" is options.exclude in cartograph.yaml. The other controls have no manifest key yet. */
const YAML: YamlMap<Prefs> = {
  read: (y) => ({ excludes: readExcludes(y).join("\n") }),
  write: (y, v) => setExcludes(y, v.excludes.split("\n").map((l) => l.trim()).filter(Boolean)),
};

export default function Indexing() {
  const live = useLive();
  const { current } = useWorkspaces();
  const liveFresh = useFreshness(live ? current?.id : undefined).data;
  const ws = live ? current : getCurrentWorkspace();
  const fresh = (live ? liveFresh : undefined) ?? getFreshness();
  const f = useFileDraft<Prefs>("cartograph:indexing", DEFAULTS, YAML);
  /** Controls with no cartograph.yaml key or route yet: shown, not editable, with the binary. */
  const later = live;
  const p: Prefs = { ...DEFAULTS, ...f.value };
  const edit = (patch: Partial<Prefs>) => f.edit({ ...p, ...patch });
  const [confirm, setConfirm] = useState(false);
  const [msg, setMsg] = useState("");
  const [models, setModels] = useState<string[]>([]);
  useEffect(() => { const a = loadAi(); setModels([...new Set([a.model, ...a.models].filter(Boolean))]); }, []);
  const id = ws?.id ?? "workspace";
  const built = fmtDate(fresh.builtAt, "dmy-hm", "local");
  const units = NODES.filter((n) => (p.summ.scope === "files" ? n.kind !== "workspace" : n.kind === "service" || n.kind === "module")).length;

  return (
    <SettingsPage title="Indexing" saves="file" file={{ count: f.count, onSave: f.save, onDiscard: f.discard, ...f.file }}
      intro="Choose what Cartograph reads and when it brings the index up to date.">
      <section className="set-sec set-idx-state" aria-labelledby="set-idx-state-h">
        <CircleCheck size={22} aria-hidden />
        <div>
          <h2 id="set-idx-state-h">The index is {fresh.state === "current" ? "up to date" : fresh.state}</h2>
          <p className="set-note">Built {built}. {fresh.behind === 0 ? "No commits waiting." : `${fresh.behind} commits behind HEAD.`}</p>
          <Advanced>
            <p className="set-note">Version {fresh.indexVersion}. Stored at <span className="set-mono">~/.cartograph/workspaces/{id}/index.cartograph</span>, outside the workspace folder so a sync client can&apos;t damage it.</p>
          </Advanced>
        </div>
        <button type="button" className="set-btn" aria-expanded={confirm} aria-controls="set-idx-confirm" onClick={() => setConfirm(!confirm)}>Rebuild index</button>
      </section>
      {confirm && (
        <div id="set-idx-confirm" className="set-confirm" role="alertdialog" aria-labelledby="set-idx-ct" aria-describedby="set-idx-cd">
          <b id="set-idx-ct">Rebuild the index from scratch?</b>
          <p id="set-idx-cd">This reads every repository again and takes about 41 seconds, based on the last build. You can keep reading the current index while it runs. If you cancel, the last complete index stays and the next run picks up where this one stopped.</p>
          <div className="set-actions">
            <button type="button" className="set-btn is-primary" onClick={async () => {
              setConfirm(false);
              if (!live || !ws) { setMsg("Rebuild confirmed. In this demo nothing is rebuilt until the local server runs it."); return; }
              try { await send("POST", `/api/v1/w/${encodeURIComponent(ws.id)}/jobs`, { kind: "rebuild" }); setMsg("Rebuild started. The index status in the header shows its progress."); }
              catch (e) { setMsg(e instanceof RequestError ? `${e.what}. ${e.try_}.` : "The rebuild couldn't start."); }
            }}>Rebuild index</button>
            <button type="button" className="set-btn" onClick={() => setConfirm(false)}>Keep current index</button>
          </div>
        </div>
      )}
      {msg && <p className="set-note" role="status">{msg}</p>}
      <FileState f={f.file} />

      <div className="set-grid set-idx-grid">
        <section className="set-sec" aria-labelledby="set-idx-ex">
          <h2 id="set-idx-ex">File types</h2>
          <p className="set-note">Choose which kinds of files Cartograph reads. Turn off the ones your repositories don&apos;t have.{later && " Readers can't be turned off from here yet."}</p>
          <ul className="set-idx-ext">
            <li className="set-idx-th" aria-hidden><span className="set-k">Reads</span><span className="set-k">Files found</span><span className="set-k">On</span></li>
            {EXTRACTORS.map((x) => {
              const on = !p.off.includes(x.id);
              return (
                <li key={x.id} className={on ? undefined : "is-off"}>
                  <span id={`set-idx-x-${x.id}`}><b>{x.name} {!on && <Chip tone="off">Off</Chip>}</b><small>{x.note}</small></span>
                  <span className="set-mono">{x.files}</span>
                  <Switch on={on} disabled={later} labelledBy={`set-idx-x-${x.id}`} onChange={(v) => edit({ off: v ? p.off.filter((o) => o !== x.id) : [...p.off, x.id] })} />
                </li>
              );
            })}
          </ul>
          <Advanced>
            <p className="set-note">Each reader&apos;s results are cached by file content, so unchanged files are never read twice.</p>
            <ul className="set-idx-ver">{EXTRACTORS.map((x) => <li key={x.id}><span>{x.name}</span><span className="set-mono">{x.v}</span></li>)}</ul>
          </Advanced>
        </section>

        <div className="set-col">
          <section className="set-sec" aria-labelledby="set-idx-up">
            <h2 id="set-idx-up">When to update</h2>
            <p className="set-note">Choose how soon changes in your repositories show up in the map.{later && " These choices aren't stored in cartograph.yaml yet; run cartograph hooks and cartograph schedule from the terminal."}</p>
            <label className="set-radio"><input type="checkbox" checked disabled /><span>Keep it current as you work<small>Always on. Answers come straight away and the index catches up in the background.</small></span></label>
            <label className="set-radio"><input type="checkbox" disabled={later} checked={p.hooks} onChange={(e) => edit({ hooks: e.target.checked })} /><span>Also update after each commit or pull<small>Adds a git hook to each repository on this machine.</small></span></label>
            <label className="set-radio"><input type="checkbox" disabled={later} checked={p.fetch} onChange={(e) => edit({ fetch: e.target.checked })} /><span>Also fetch every hour<small>Runs git fetch, the only time Cartograph uses the network.</small></span></label>
          </section>
          <section className="set-sec" aria-labelledby="set-idx-sc">
            <h2 id="set-idx-sc">Left out</h2>
            <p className="set-note">Paths Cartograph skips. Binary files and files over 2 MB are always skipped. History starts at the first commit.</p>
            <label className="set-label">Paths to leave out, one per line
              <textarea className="set-area" rows={5} readOnly={f.file.readOnly} value={p.excludes} onChange={(e) => edit({ excludes: e.target.value })} />
            </label>
          </section>
          <section className="set-sec" aria-labelledby="set-md-s">
            <div className="set-sec-head"><h2 id="set-md-s">Written summaries</h2><Switch on={p.summ.on} disabled={later} labelledBy="set-md-s" onChange={(on) => edit({ summ: { ...p.summ, on } })} /></div>
            <p className="set-note">A model writes a short description of each service and module. Every one is marked as model-written where it shows. Off by default.</p>
            {p.summ.on && <>
              <div className="set-idx-two">
                <label className="set-label">Model
                  <select className="set-select" value={p.summ.model} onChange={(e) => edit({ summ: { ...p.summ, model: e.target.value } })}>
                    <option value="">Choose a model</option>{models.map((m) => <option key={m} value={m}>{m}</option>)}
                  </select>
                </label>
                <label className="set-label">Write for
                  <select className="set-select" value={p.summ.scope} onChange={(e) => edit({ summ: { ...p.summ, scope: e.target.value as Summaries["scope"] } })}>
                    <option value="modules">Services and modules</option><option value="files">Services, modules and files</option>
                  </select>
                </label>
              </div>
              <p className="set-note">{units} summaries, about {Math.max(1, Math.ceil(units / 60))} min with a local model. A hosted model receives the code it summarises.{models.length === 0 && " Set up a model in Models first."}</p>
              <p className="set-note">Writing starts once the local server is running.</p>
            </>}
          </section>
          <section className="set-sec" aria-labelledby="set-idx-rl">
            <div className="set-sec-head"><h2 id="set-idx-rl">Recent runs</h2><span className="set-note">Full figures are on the Coverage view.</span></div>
            <ul className="set-idx-runs">
              <li aria-hidden><span className="set-k">Run</span><span className="set-k">Duration</span><span className="set-k">Files read</span></li>
              {RUNS.map((r) => <li key={r.id}><span>{r.what}</span><span className="set-mono">{r.took}</span><span className="set-mono"><Check size={12} aria-hidden /> {r.parsed}</span></li>)}
            </ul>
          </section>
        </div>
      </div>
    </SettingsPage>
  );
}
