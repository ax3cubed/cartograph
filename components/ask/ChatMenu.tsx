"use client";

import { useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { ChevronDown, History as HistoryIcon, RotateCcw, Search, Trash2 } from "lucide-react";
import { AiMark } from "@/components/brand/AiMark";
import { staleReason, type HistTurn } from "./History";
import { fmtDay } from "./format";
import type { Thread } from "./store";

const DAY = 86_400_000;
const age = (ms: number) => {
  const d = Math.floor((Date.now() - ms) / DAY);
  return d < 1 ? "today" : d < 7 ? `${d}d` : d < 60 ? `${Math.floor(d / 7)}w` : `${Math.floor(d / 30)}mo`;
};
function groups(threads: Thread[]) {
  const today = new Date().setHours(0, 0, 0, 0);
  const out: [string, Thread[]][] = [["Today", []], ["Last 7 days", []], ["Earlier", []]];
  for (const th of threads) out[th.at >= today ? 0 : th.at >= today - 6 * DAY ? 1 : 2][1].push(th);
  return out.filter(([, list]) => list.length);
}

/**
 * The chat history menu in the /ask page header (Claire's `cl_full` pattern): the open chat's title is the
 * trigger, and the popover holds a search field, this browser's chats grouped by age, and the questions from
 * earlier sessions, which re-run as of the date they were asked.
 */
export default function ChatMenu({ threads, active, live, onOpen, onDelete, past = [], onRerun }: {
  threads: Thread[];
  active: Thread | null;
  /** A question is streaming: the mark pulses. */
  live?: boolean;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
  past?: HistTurn[];
  onRerun?: (question: string, asOf?: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const id = useId();
  const query = q.trim().toLowerCase();
  const shown = groups(threads.filter((th) => th.title.toLowerCase().includes(query)));
  const earlier = onRerun ? past.filter((x) => x.question.toLowerCase().includes(query)) : [];

  const close = (refocus = true) => {
    setOpen(false);
    setQ("");
    if (refocus) ref.current?.querySelector<HTMLElement>("[aria-haspopup]")?.focus();
  };

  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) { setOpen(false); setQ(""); } };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);

  // Arrows move between the search field and the rows; Esc closes and returns focus to the trigger.
  const onKey = (e: ReactKeyboardEvent) => {
    if (!open) return;
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); return; }
    const step = { ArrowDown: 1, ArrowUp: -1 }[e.key];
    if (!step) return;
    const items = [...(ref.current?.querySelectorAll<HTMLElement>(".ask-cm-search input, .ask-cm-open") ?? [])];
    const at = items.indexOf(document.activeElement as HTMLElement);
    e.preventDefault();
    items[(at + step + items.length) % items.length]?.focus();
  };

  const none = shown.length === 0 && earlier.length === 0;

  return (
    <div className="ask-cm" ref={ref} onKeyDown={onKey}>
      <button type="button" className="ask-cm-btn" aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? id : undefined}
        title="Chat history" onClick={() => (open ? close(false) : setOpen(true))}>
        <AiMark size={20} active={live} />
        <span className="ask-cm-title">{active?.title ?? "New chat"}</span>
        <ChevronDown size={14} aria-hidden />
      </button>
      {open && (
        <div className="ask-cm-pop" role="dialog" id={id} aria-label="Chat history">
          <label className="ask-cm-search">
            <Search size={15} aria-hidden />
            <input autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search chat history" aria-label="Search chat history" />
          </label>
          <div className="ask-cm-list">
            {none && <p className="ask-cm-none">{threads.length || past.length ? "No chat matches." : "No chats yet. Your questions are kept in this browser."}</p>}
            {shown.map(([label, list]) => (
              <div key={label} className="ask-cm-group" role="group" aria-label={label}>
                <p className="ask-cm-k" aria-hidden>{label}</p>
                {list.map((th) => (
                  <div key={th.id} className="ask-cm-item" aria-current={th.id === active?.id ? "page" : undefined}>
                    <button type="button" className="ask-cm-open" onClick={() => { onOpen(th.id); close(); }}>
                      <span>{th.title}</span><span className="ask-cm-age">{age(th.at)}</span>
                    </button>
                    <button type="button" className="ask-icon" aria-label={`Delete “${th.title}”`} title="Delete" onClick={() => onDelete(th.id)}>
                      <Trash2 size={14} aria-hidden />
                    </button>
                  </div>
                ))}
              </div>
            ))}
            {earlier.length > 0 && (
              <div className="ask-cm-group" role="group" aria-label="Earlier sessions">
                <p className="ask-cm-k" aria-hidden><HistoryIcon size={12} aria-hidden /> Earlier sessions · re-run as of the date asked</p>
                {earlier.map((x) => {
                  const stale = staleReason(x);
                  return (
                    <div key={x.id} className="ask-cm-item">
                      <button type="button" className="ask-cm-open" title={stale ? `Out of date: ${stale}` : "Re-run this question"}
                        onClick={() => { onRerun!(x.question, x.asOf); close(); }}>
                        <span>{x.question}</span>
                        <span className="ask-cm-age">{stale ? "out of date" : x.asOf ? fmtDay(x.asOf) : ""}</span>
                        <RotateCcw size={13} aria-hidden className="ask-cm-rerun" />
                      </button>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
