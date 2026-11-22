"use client";

import { useEffect, useState } from "react";
import { Server, X } from "lucide-react";
import { ClaudeIcon, GeminiIcon, GrokIcon, OpenAIIcon } from "@/components/ui/model-picker";

const MARK: Record<string, (p: { className?: string }) => React.ReactNode> = { anthropic: ClaudeIcon, openai: OpenAIIcon, google: GeminiIcon, xai: GrokIcon };
import { DEFAULT_AI, MAX_MODELS, PRESETS, PROVIDER_NAMES, PROVIDER_URLS, canAsk, listModels, loadAi, providerFor, saveAi, type AiSettings } from "@/lib/ai";
import { Chip, SettingsPage, useFlash } from "./ui";
import "./models.css";

const BUILTIN = Object.keys(PROVIDER_URLS);
const CUSTOM = "custom";
/** Custom presets: the endpoints that are not one of the built-in providers. */
const CUSTOM_PRESETS = PRESETS.filter(([, u]) => providerFor(u) === CUSTOM);
const nameOf = (id: string) => (id === CUSTOM ? "Custom endpoint" : PROVIDER_NAMES[id] ?? id);
const hostOf = (url: string) => { try { return new URL(url).host; } catch { return url; } };

export default function Models() {
  const [ai, setAi] = useState<AiSettings>(DEFAULT_AI);
  const [sel, setSel] = useState<string>(DEFAULT_AI.provider);
  const [key, setKey] = useState("");
  const [url, setUrl] = useState("");
  const [cKey, setCKey] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [found, setFound] = useState<string[]>([]);
  const [add, setAdd] = useState("");
  const [msg, setMsg] = useState("");
  const [loading, setLoading] = useState(false);
  const [saved, flash] = useFlash();

  useEffect(() => {
    const s = loadAi();
    setAi(s); setSel(s.provider === CUSTOM || PROVIDER_URLS[s.provider] ? s.provider : CUSTOM);
    setUrl(s.baseUrl); setCKey(s.apiKey); setModels(s.models);
  }, []);

  const commit = (next: AiSettings) => { saveAi(next); setAi(next); flash(); setMsg(""); };
  const keyOf = (id: string) => (id === CUSTOM ? ai.apiKey : ai.keys[id] ?? "");
  const ready = canAsk(ai);

  const useForAsk = (id: string) => {
    const model = id === CUSTOM && !ai.models.includes(ai.model) ? ai.models[0] ?? ai.model : ai.model;
    commit({ ...ai, provider: id, model });
  };
  const saveKey = () => { commit({ ...ai, keys: { ...ai.keys, [sel]: key.trim() } }); setKey(""); };
  const removeKey = () => commit({ ...ai, keys: { ...ai.keys, [sel]: "" } });

  const addModel = () => {
    const m = add.trim();
    if (!m || models.includes(m)) return setAdd("");
    if (models.length >= MAX_MODELS) return setMsg(`The picker holds ${MAX_MODELS} models. Remove one first.`);
    setModels([...models, m]); setAdd("");
  };
  const load = async () => {
    setLoading(true); setMsg("");
    try {
      const list = await listModels({ baseUrl: url.trim(), apiKey: cKey });
      setFound(list); setMsg(`Found ${list.length} models. Type or pick up to ${MAX_MODELS} for the picker.`);
    } catch (e) {
      setMsg(`Couldn't load models: ${e instanceof Error ? e.message : "no answer"}. Check the URL and key, or type model ids yourself.`);
    } finally { setLoading(false); }
  };
  const saveCustom = () => {
    const baseUrl = url.trim();
    const owner = providerFor(baseUrl);
    if (!baseUrl) return setMsg("Enter the endpoint's URL first.");
    if (owner !== CUSTOM) return setMsg(`That address belongs to ${nameOf(owner)}. Choose it from the list above and add a key there.`);
    commit({ ...ai, baseUrl, apiKey: cKey.trim(), models, model: models.includes(ai.model) || ai.provider !== CUSTOM ? ai.model : models[0] ?? ai.model });
  };

  const status = ready ? `Ask is using ${nameOf(ai.provider)}.` : `Ask is set to ${nameOf(ai.provider)} but has no ${ai.provider === CUSTOM && !ai.baseUrl ? "endpoint" : "key"} yet, so answers come from the map alone.`;

  return (
    <SettingsPage title="Models" saves="app" saved={saved}
      intro={<>Choose the AI service that words Ask&apos;s answers. With none set up, answers come from the map alone and nothing leaves this machine. {status}</>}>
      <div className="set-grid set-md-grid">
        <section className="set-sec" aria-labelledby="set-md-p">
          <h2 id="set-md-p">Services</h2>
          <p className="set-note">Ask uses one service at a time. Claude, OpenAI, Gemini and Grok need a key. A custom endpoint covers OpenRouter, Ollama, LM Studio or your team&apos;s own.</p>
          <ul className="set-md-provs">
            {[...BUILTIN, CUSTOM].map((id) => {
              const active = ai.provider === id;
              const isCustom = id === CUSTOM;
              const hasKey = !!keyOf(id);
              const usable = isCustom ? !!ai.baseUrl : hasKey;
              return (
                <li key={id} className={sel === id ? "is-sel" : undefined}>
                  <span className="set-md-ic" aria-hidden>{(() => { const M = MARK[id]; return M ? <M className="set-md-mark" /> : <Server size={16} />; })()}</span>
                  <span><b>{nameOf(id)}</b><small>{isCustom ? (ai.baseUrl ? hostOf(ai.baseUrl) : "OpenRouter, Ollama, LM Studio or your own") : hostOf(PROVIDER_URLS[id])}</small></span>
                  <span className="set-md-chips">
                    {isCustom && !ai.baseUrl ? <Chip tone="off">Not set up</Chip> : <Chip tone={hasKey ? "ok" : "off"} dot>{hasKey ? "Key saved" : isCustom ? "No key (optional)" : "No key"}</Chip>}
                    {active && <Chip tone="acc">Used by Ask</Chip>}
                  </span>
                  <span className="set-actions">
                    <button type="button" className="set-btn is-sm is-line" aria-pressed={sel === id} aria-label={`Edit ${nameOf(id)}`} onClick={() => { setSel(id); setMsg(""); }}>Edit</button>
                    {!active && <button type="button" className="set-btn is-sm" disabled={!usable} title={usable ? undefined : isCustom ? "Save an endpoint first" : "Add a key first"} aria-label={`Use ${nameOf(id)} for Ask`} onClick={() => useForAsk(id)}>Use for Ask</button>}
                  </span>
                </li>
              );
            })}
          </ul>
        </section>

        <section className="set-sec" aria-labelledby="set-md-f">
          <h2 id="set-md-f">{nameOf(sel)}</h2>
          {sel !== CUSTOM ? (
            <form className="set-md-form" onSubmit={(e) => { e.preventDefault(); if (key.trim()) saveKey(); }}>
              <label className="set-label" htmlFor="set-md-key">{nameOf(sel)} API key</label>
              <input id="set-md-key" key={sel} className="set-input is-mono" type="password" autoComplete="off" placeholder={keyOf(sel) ? "Type to replace the saved key" : "Paste your key"} value={key} onChange={(e) => setKey(e.target.value)} />
              <p className="set-note">Sent only to <span className="set-mono">{hostOf(PROVIDER_URLS[sel])}</span>. It is never shown again. Until the local server can use your keychain, it stays in this browser.</p>
              <div className="set-actions">
                <button type="submit" className="set-btn is-primary" disabled={!key.trim()}>Save key</button>
                {keyOf(sel) && <button type="button" className="set-btn is-line" onClick={removeKey}>Remove key</button>}
              </div>
            </form>
          ) : (
            <form className="set-md-form" onSubmit={(e) => { e.preventDefault(); saveCustom(); }}>
              <label className="set-label" htmlFor="set-md-url">Endpoint URL</label>
              <input id="set-md-url" className="set-input is-mono" type="url" list="set-md-presets" placeholder="http://localhost:11434/v1" value={url} onChange={(e) => setUrl(e.target.value)} />
              <datalist id="set-md-presets">{CUSTOM_PRESETS.map(([n, u]) => <option key={u} value={u} label={n} />)}</datalist>
              <p className="set-note">It must speak the OpenAI chat format. Start typing to see Ollama, LM Studio and OpenRouter.</p>
              <label className="set-label" htmlFor="set-md-ckey">API key, if it needs one</label>
              <input id="set-md-ckey" className="set-input is-mono" type="password" autoComplete="off" placeholder="Not needed for local models" value={cKey} onChange={(e) => setCKey(e.target.value)} />
              <label className="set-label" htmlFor="set-md-add">Models in the picker ({models.length} of {MAX_MODELS})</label>
              <div className="set-actions">
                <input id="set-md-add" className="set-input is-mono set-md-grow" list="set-md-found" placeholder="Model id, e.g. llama3.1:8b" value={add} onChange={(e) => setAdd(e.target.value)} onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), addModel())} />
                <datalist id="set-md-found">{found.map((m) => <option key={m} value={m} />)}</datalist>
                <button type="button" className="set-btn" onClick={addModel} disabled={!add.trim()}>Add</button>
                <button type="button" className="set-btn is-line" onClick={load} disabled={!url.trim() || loading}>{loading ? "Loading…" : "Load models"}</button>
              </div>
              {models.length > 0 && <ul className="set-md-models">{models.map((m) => <li key={m}><code>{m}</code><button type="button" className="icon-btn sm" aria-label={`Remove ${m}`} onClick={() => setModels(models.filter((x) => x !== m))}><X size={12} /></button></li>)}</ul>}
              <div className="set-actions"><button type="submit" className="set-btn is-primary">Save endpoint</button></div>
            </form>
          )}
          {msg && <p className="set-note" role="status">{msg}</p>}
        </section>
      </div>
    </SettingsPage>
  );
}
