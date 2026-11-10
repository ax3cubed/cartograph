"use client";

import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { MAX_MODELS, PRESETS, listModels, type AiSettings } from "@/lib/ai";

interface Props {
  open: boolean;
  settings: AiSettings;
  onSave: (s: AiSettings) => void;
  onClose: () => void;
}

/** Model settings: which provider to talk to and which of its models appear in the composer picker. */
export default function Settings({ open, settings, onSave, onClose }: Props) {
  const ref = useRef<HTMLDialogElement>(null);
  const [draft, setDraft] = useState(settings);
  const [add, setAdd] = useState("");
  const [available, setAvailable] = useState<string[]>([]);
  const [status, setStatus] = useState("");

  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) { setDraft(settings); setStatus(""); d.showModal(); }
    if (!open && d.open) d.close();
  }, [open, settings]);

  const addModel = (id: string) => {
    id = id.trim();
    if (!id || draft.models.includes(id)) return setAdd("");
    if (draft.models.length >= MAX_MODELS) return setStatus(`The picker holds ${MAX_MODELS} models. Remove one first.`);
    setDraft({ ...draft, models: [...draft.models, id] });
    setAdd("");
    setStatus("");
  };
  const load = async () => {
    setStatus("Loading models…");
    try {
      const ids = await listModels(draft);
      setAvailable(ids);
      setStatus(`${ids.length} models available. Type to search, then Add.`);
    } catch (e) {
      setStatus(e instanceof Error ? e.message : String(e));
    }
  };
  const save = () => {
    const model = draft.models.includes(draft.model) ? draft.model : draft.models[0] ?? "";
    onSave({ ...draft, baseUrl: draft.baseUrl.trim(), model });
    onClose();
  };

  return (
    <dialog ref={ref} className="settings" onClose={onClose} aria-labelledby="settings-title">
      <form method="dialog" onSubmit={(e) => (e.preventDefault(), save())}>
        <header>
          <h2 id="settings-title">Model settings</h2>
          <button type="button" className="icon-btn sm" onClick={onClose} aria-label="Close settings"><X size={14} /></button>
        </header>
        <p className="settings-note">
          Works with any provider that has an OpenAI-compatible endpoint, hosted or on this machine.
          With no model set up, answers come from the map itself and nothing leaves your browser.
        </p>

        <label htmlFor="ai-preset">Provider</label>
        <select id="ai-preset" value={PRESETS.find(([, u]) => u === draft.baseUrl)?.[1] ?? ""} onChange={(e) => e.target.value && setDraft({ ...draft, baseUrl: e.target.value })}>
          <option value="">Custom</option>
          {PRESETS.map(([name, url]) => <option key={url} value={url}>{name}</option>)}
        </select>

        <label htmlFor="ai-base">Base URL</label>
        <input id="ai-base" type="url" inputMode="url" placeholder="http://localhost:11434/v1" value={draft.baseUrl} onChange={(e) => setDraft({ ...draft, baseUrl: e.target.value })} />

        <label htmlFor="ai-key">API key</label>
        <input id="ai-key" type="password" autoComplete="off" placeholder="Not needed for local models" value={draft.apiKey} onChange={(e) => setDraft({ ...draft, apiKey: e.target.value })} />
        <p className="settings-note">Kept in this browser. Sent to the Cartograph server only so it can call your provider.</p>

        <label htmlFor="ai-add">Models in the picker ({draft.models.length}/{MAX_MODELS})</label>
        <div className="settings-row">
          <input id="ai-add" list="ai-available" placeholder="Model id, e.g. llama3.3 or claude-opus-5-5" value={add}
            onChange={(e) => setAdd(e.target.value)} onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), addModel(add))} />
          <datalist id="ai-available">{available.map((id) => <option key={id} value={id} />)}</datalist>
          <button type="button" className="btn" onClick={() => addModel(add)} disabled={!add.trim()}>Add</button>
          <button type="button" className="btn" onClick={load} disabled={!draft.baseUrl.trim()}>Load list</button>
        </div>
        {draft.models.length > 0 && (
          <ul className="settings-models">
            {draft.models.map((id) => (
              <li key={id}>
                <code>{id}</code>
                <button type="button" className="icon-btn sm" onClick={() => setDraft({ ...draft, models: draft.models.filter((m) => m !== id) })} aria-label={`Remove ${id}`}><X size={12} /></button>
              </li>
            ))}
          </ul>
        )}
        <p className="settings-status" role="status">{status}</p>
        <p className="settings-note">
          In the picker, the columns set reasoning effort (ignored by models that have none) and ULTRA sends the whole map as context instead of only the evidence for your question.
        </p>

        <footer>
          <button type="button" className="btn" onClick={onClose}>Cancel</button>
          <button type="submit" className="btn primary">Save</button>
        </footer>
      </form>
    </dialog>
  );
}
