"use client";

// FE-14: the corrections of the override file, with undo. A chip on the map toolbar opens the list, and it shows only
// while the file holds an entry. An entry whose link has gone (`stale`, the list `cartograph check` fails on) says so.
// Keys and reasons are the file's text: shown as text.
import { useEffect, useRef, useState } from "react";
import { useGenerationTick } from "@/lib/api/cache";
import { RequestError } from "@/lib/api/client";
import { CORRECTIONS_CHANGED, getCorrections, undoCorrection, type Correction } from "@/lib/api/corrections";
import { toast } from "../shell/toast";
import "./correction.css";

const VERB = { remove: "Removed", retarget: "Retargeted", set: "Level set", add: "Added" } as const;

export default function CorrectionList({ ws }: { ws: string }) {
  // Read again when the index changes (a sync applied the file) and when this tab wrote the file.
  const tick = useGenerationTick();
  const [wrote, setWrote] = useState(0);
  const [items, setItems] = useState<Correction[]>([]);
  useEffect(() => {
    const f = () => setWrote((n) => n + 1);
    window.addEventListener(CORRECTIONS_CHANGED, f);
    return () => window.removeEventListener(CORRECTIONS_CHANGED, f);
  }, []);
  useEffect(() => {
    let live = true;
    getCorrections(ws).then((r) => live && setItems(r), () => live && setItems([]));
    return () => { live = false; };
  }, [ws, tick, wrote]);
  const [open, setOpen] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const box = useRef<HTMLDialogElement>(null);
  useEffect(() => { if (open) box.current?.showModal(); else box.current?.close(); }, [open]);
  if (items.length === 0 && !open) return null;
  const stale = items.filter((c) => c.stale).length;

  async function undo(id: string) {
    try {
      await undoCorrection(ws, id);
      setErr(null);
      toast("Removed from cartograph.overrides.yaml. The link comes back when the index has applied it.");
    } catch (e) {
      setErr(e instanceof RequestError ? `${e.what}. ${e.why}. ${e.try_}.` : "Something went wrong. Try again.");
    }
  }

  return (
    <>
      <button type="button" className={stale ? "lm-chip is-acc" : "lm-chip"} onClick={() => setOpen(true)}>
        Corrections · {items.length}{stale > 0 && <> · {stale} stale</>}
      </button>
      <dialog ref={box} className="cd cl" aria-labelledby="cl-title" onCancel={(e) => { e.preventDefault(); setOpen(false); }} onClose={() => setOpen(false)}>
        <div className="cd-body">
          <h2 id="cl-title">Corrections</h2>
          <p className="cd-sub">From cartograph.overrides.yaml. Undo removes the entry from the file.</p>
          {err && <p className="cd-err" role="alert">{err}</p>}
          {items.length === 0 ? <p className="cd-none">The file holds no corrections.</p> : (
            <ul>
              {items.map((c) => (
                <li key={c.id}>
                  <div>
                    <b>{VERB[c.action]}</b>
                    <code>{c.key.from} to {c.key.to}{c.key.op ? ` · ${c.key.op}` : ""}</code>
                    {c.stale && <span className="map-chip">Stale: matches no link</span>}
                    <button type="button" className="btn" onClick={() => void undo(c.id)} aria-label={`Undo: ${c.key.from} to ${c.key.to}`}>Undo</button>
                  </div>
                  <p className="cl-reason">{c.reason}</p>
                  <span className="cl-by">{c.by ? `${c.by}${c.at ? `, ${c.at.slice(0, 10)}` : ""}` : "Not committed yet"}</span>
                </li>
              ))}
            </ul>
          )}
          <div className="cd-actions"><button type="button" className="btn" onClick={() => setOpen(false)}>Close</button></div>
        </div>
      </dialog>
    </>
  );
}
