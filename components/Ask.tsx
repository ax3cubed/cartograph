"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import Link from "next/link";
import { BookOpen, ChevronDown, GitCompareArrows, Info, List, Route, Server, SquarePen, Zap } from "lucide-react";
import { SUGGESTIONS, mentions, type Answer, type Part, type ToolId } from "@/lib/ask";
import { DEFAULT_AI, PROVIDER_NAMES, canAsk, loadAi, modelName, saveAi, type AiSettings } from "@/lib/ai";
import { getAskHistory } from "@/lib/source";
import { AiMark } from "@/components/brand/AiMark";
import { ModelPicker, defaultModelProviders, type ModelPickerProvider, type ThinkingEffort } from "./ui/model-picker";
import History from "./ask/History";
import Composer from "./ask/Composer";
import Inspector from "./ask/Inspector";
import { TurnView, type Nav } from "./ask/Message";
import ModeSwitch, { useMoveAsk } from "./ask/ModeSwitch";
import { activeThread, askStore, focusComposer, run, stop, useAsk, type Thread } from "./ask/store";
import { LINKS, WORKSPACE, byId, commitBySha, serviceColor } from "@/lib/workspace";
import "./ask/ask.css";

const TIER_OF: Record<ThinkingEffort, number> = { none: 0, low: 0, medium: 1, high: 2, max: 4 };
const EFFORT_OF: ThinkingEffort[] = ["low", "medium", "high", "high", "max"];
/** The composer's id, so the sidebar and the floating panel can focus it. Only one composer is mounted at a time. */
export const ASK_INPUT_ID = "ask-input";

/** Renders engine answer text with nodes and commits as clickable evidence. Only used for text the graph engine wrote. */
export function Parts({ parts, onOpen }: { parts: Part[]; onOpen: (id: string) => void }) {
  return (
    <>
      {parts.map((p, i) => {
        if (typeof p === "string") return <span key={i}>{p}</span>;
        if ("node" in p) {
          const n = byId.get(p.node)!;
          return (
            <button type="button" key={i} className="ref" style={{ ["--c" as string]: serviceColor(n.service) }} onClick={() => onOpen(n.id)} title={n.path}>
              {n.name}
            </button>
          );
        }
        const c = commitBySha.get(p.commit)!;
        return <code key={i} className="ref-commit" title={`${c.message} · ${c.author} · ${c.date}`}>{c.sha}</code>;
      })}
    </>
  );
}

export interface AskProps {
  t: number;
  /** The answer whose evidence is lit on the map. */
  answer: Answer | null;
  onAnswer: (a: Answer | null) => void;
  onOpen: (id: string) => void;
  /** A citation was clicked. The host selects the node and switches to Details. Falls back to onOpen. */
  onCitation?: (id: string) => void;
  /** The selected node id. Shown as a removable @name chip and used as the subject when the question names none. */
  context?: string;
  /** A question handed over from search (`#ask=`). Fills the composer and focuses it; the person presses Enter to send. */
  initialQuestion?: string;
  /** Off the map: map URLs for a node and for an answer's evidence. Evidence then links to the map instead of lighting it. */
  hrefFor?: (id: string) => string;
  answerHref?: (turnId: string) => string;
}

type Starter = { q: string; ids?: string[]; icon: typeof Zap };
const SUGGESTION_ICONS = [Zap, Route, GitCompareArrows, BookOpen];

/** Four questions about the selected node, or the general ones when nothing is selected. */
function startersFor(id: string | null): Starter[] {
  const n = id ? byId.get(id) : null;
  if (!n) return SUGGESTIONS.map((q, i) => ({ q, icon: SUGGESTION_ICONS[i] ?? BookOpen }));
  const inside = (x: string) => x === n.id || x.startsWith(`${n.id}/`);
  const out = LINKS.find((l) => !l.removed && inside(l.from) && !inside(l.to) && byId.has(l.to));
  const to = out ? byId.get(out.to)! : null;
  return [
    { q: `What does ${n.name} do?`, ids: [n.id], icon: BookOpen },
    { q: `What breaks if I change ${n.name}?`, ids: [n.id], icon: Zap },
    to ? { q: `How does ${n.name} reach ${to.name}?`, ids: [n.id, to.id], icon: Route } : { q: SUGGESTIONS[1], icon: Route },
    { q: `What changed in ${n.name} in the last 3 months?`, ids: [n.id], icon: GitCompareArrows },
  ];
}

/** The thread picker in the docked and floating headers: recent chats, and the page that lists them all. */
function ThreadMenu({ threads, active }: { threads: Thread[]; active: Thread | null }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false); };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);
  const onKey = (e: ReactKeyboardEvent) => {
    if (e.key !== "Escape" || !open) return;
    e.preventDefault(); e.stopPropagation(); // close the menu, not the floating panel
    setOpen(false);
    ref.current?.querySelector<HTMLElement>("[aria-haspopup]")?.focus();
  };
  return (
    <div className="ask-tmenu" ref={ref} onKeyDown={onKey}>
      <button type="button" className="ask-tmenu-btn" aria-haspopup="true" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <span>{active?.title ?? "New chat"}</span><ChevronDown size={14} aria-hidden />
      </button>
      {open && (
        <div className="ask-tmenu-pop">
          <p className="ask-tmenu-k">Recent chats</p>
          {threads.length === 0 && <p className="ask-tmenu-none">No chats yet.</p>}
          {threads.slice(0, 8).map((th) => (
            <button key={th.id} type="button" className="ask-tmenu-row" aria-current={th.id === active?.id || undefined}
              onClick={() => { askStore.openThread(th.id); setOpen(false); }}>
              <span>{th.title}</span><span className="ask-tmenu-n">{th.turns.length}</span>
            </button>
          ))}
          <Link href="/ask" className="ask-tmenu-all" onClick={() => askStore.setReturnTo(location.pathname + location.hash)}><List size={14} aria-hidden />All chats</Link>
        </div>
      )}
    </div>
  );
}

let appliedQuestion: string | undefined; // a remount after a mode switch must not refill the composer

/**
 * The chat. One thread state (components/ask/store.ts) shown in three places: `docked` in the map's sidebar,
 * `floating` over any route, and `page` at /ask. Answers come from the graph engine first, then a model
 * words them with checked citations if one is set up.
 */
export default function Ask({ t, answer, onAnswer, onOpen, onCitation, context, initialQuestion, hrefFor, answerHref, variant = "docked" }: AskProps & { variant?: "docked" | "floating" | "page" }) {
  const s = useAsk();
  const thread = activeThread(s);
  const turns = useMemo(() => thread?.turns ?? [], [thread]);
  const scroll = useRef<HTMLDivElement>(null);
  const [ai, setAi] = useState<AiSettings>(DEFAULT_AI);
  const [inspecting, setInspecting] = useState<Answer | null>(null);
  const [removed, setRemoved] = useState<string | null>(null); // the context chip the person dismissed
  // a remount after a mode switch must not refill the composer with a question already handed over
  const handed = initialQuestion?.trim() && initialQuestion !== appliedQuestion ? initialQuestion : undefined;

  useEffect(() => setAi(loadAi()), []);
  useEffect(() => { if (handed) appliedQuestion = handed; }, [handed]);
  useEffect(() => { if (!initialQuestion) appliedQuestion = undefined; }, [initialQuestion]);
  const update = useCallback((patch: Partial<AiSettings>) => setAi((x) => { const next = { ...x, ...patch }; saveAi(next); return next; }), []);
  const pick = useCallback((model: string, provider: string, effort?: ThinkingEffort) =>
    update({ model, provider, ...(effort ? { tier: TIER_OF[effort] } : {}) }), [update]);
  // Claude, OpenAI, Gemini and Grok are always offered; models added in settings appear as one more provider.
  const providers = useMemo<ModelPickerProvider[]>(() => ai.models.length
    ? [...defaultModelProviders, { id: "custom", name: "Your endpoint", icon: <Server aria-hidden />, models: ai.models.map((id) => ({ id, name: modelName(id), description: ai.baseUrl })) }]
    : [...defaultModelProviders], [ai.models, ai.baseUrl]);
  const ready = canAsk(ai);
  const past = useMemo(() => getAskHistory(), []);
  const ctx = context && context !== WORKSPACE && context !== removed && byId.has(context) ? byId.get(context)! : null;
  const nav: Nav = { onOpen: onCitation ?? onOpen, hrefFor };
  const working = !!s.live && turns.some((x) => x.id === s.live);

  // A new question pins to the top of the view, and its answer streams in below it.
  useEffect(() => {
    const el = scroll.current, last = el?.querySelector<HTMLElement>(".ask-thread > li:last-child");
    if (el && last) el.scrollTop = last.offsetTop - 12;
  }, [turns.length, thread?.id]);

  const ask = (text: string, asOf = t, opts: { ids?: string[]; tool?: ToolId } = {}) =>
    run(text, asOf, opts, ai, onAnswer);
  const submit = (text: string, opts: { ids?: string[]; tool?: ToolId }) => {
    // The context chip is the subject only when the question names nothing itself.
    const ids = opts.ids?.length || !ctx || mentions(text).length ? opts.ids : [ctx.id];
    ask(text, t, { ids, tool: opts.tool });
  };
  const empty = turns.length === 0;
  const starters = startersFor(ctx?.id ?? null);
  const page = variant === "page";

  return (
    <div className={`ask is-${variant}`}>
      {!page && (
        <div className="ask-head">
          <ThreadMenu threads={s.threads} active={thread} />
          <button type="button" className="ask-icon" aria-label="New chat" title="New chat" disabled={empty} onClick={() => { askStore.newChat(); onAnswer(null); focusComposer(); }}>
            <SquarePen size={15} aria-hidden />
          </button>
          {variant === "docked" && <ModeSwitch current="docked" />}
        </div>
      )}
      <div className="ask-scroll" ref={scroll}>
        <div className="ask-col">
          {!page && <History turns={past} t={t} onRerun={(question, asOf) => ask(question, asOf)} />}
          {empty ? (
            <div className="ask-empty">
              <AiMark size={page ? 36 : 28} />
              <h2>{ctx ? `Ask about ${ctx.name}` : "Ask about this codebase"}</h2>
              <p>Answers come from the map and cite the files they use, so you can check each one. Type @ to name a file, or / for a tool.</p>
              <div className="ask-starters" aria-label="Suggested questions" role="group">
                {starters.map(({ q, ids, icon: Icon }) => (
                  <button type="button" key={q} className="ask-starter" onClick={() => ask(q, t, ids ? { ids } : {})}>
                    <span className="ask-starter-ic"><Icon size={15} aria-hidden /></span>{q}
                  </button>
                ))}
              </div>
            </div>
          ) : (
            <ol className="ask-thread" aria-live="polite">
              {turns.map((x) => (
                <TurnView
                  key={x.id} a={x.a} model={x.model} working={s.live === x.id} lit={answer === x.a} t={t} nav={nav}
                  onShow={() => onAnswer(x.a)} showHref={answerHref?.(x.id)} onInspect={() => setInspecting(x.a)}
                  onReask={(asOf) => ask(x.a.q, asOf)}
                />
              ))}
            </ol>
          )}
        </div>
      </div>
      {inspecting?.context && <Inspector answer={inspecting} onClose={() => setInspecting(null)} />}

      <div className="ask-composer">
        <div className="ask-col">
          {!empty && <p className="ask-label">Follow-up</p>}
          <Composer
            inputId={ASK_INPUT_ID}
            onSubmit={submit}
            busy={working}
            onStop={stop}
            context={ctx ? { id: ctx.id, name: ctx.name } : null}
            onClearContext={() => ctx && setRemoved(ctx.id)}
            initialQuestion={handed}
            placeholder={empty ? "Ask the map · @ a file · / a tool" : "Ask a follow-up · @ a file · / a tool"}
            value={s.draft}
            refs={s.refs}
            onDraftChange={(v, r) => askStore.setDraft(v, r)}
            picker={
              <ModelPicker
                providers={providers}
                value={ai.model}
                thinking={EFFORT_OF[ai.tier]}
                onValueChange={pick}
                side="top"
                align="end"
                className="ask-model"
                footer={<>
                  <button type="button" role="switch" aria-checked={ai.ultra} className="ask-switch" onClick={() => update({ ultra: !ai.ultra })}>
                    <span className="ask-switch-track" aria-hidden /> Ultra <span className="ask-switch-hint">sends more of the map</span>
                  </button>
                  <Link href="/settings/models" className="ask-manage">{ready ? "Manage models" : "Add a key"}</Link>
                  {!ready && <p className="ask-keynote">No key for {PROVIDER_NAMES[ai.provider] ?? "this endpoint"} yet, so answers come from the map only.</p>}
                </>}
              />
            }
          />
          {page && <p className="ask-foot"><Info size={12} aria-hidden />Answers are drawn from the map as of the date shown on each one. Check the cited files.</p>}
        </div>
      </div>
    </div>
  );
}

/** The sidebar's Ask tab. While the chat floats, the tab says so and offers to dock it back. */
export function AskDocked(props: AskProps) {
  const s = useAsk();
  const move = useMoveAsk();
  if (s.mode !== "floating") return <Ask {...props} variant="docked" />;
  return (
    <div className="ask ask-away">
      <AiMark size={28} />
      <h2>The chat is floating</h2>
      <p>It sits in a panel over the map. Dock it here to keep the evidence it lights in view.</p>
      <div className="ask-away-acts">
        <button type="button" className="btn primary" onClick={() => move("docked")}>Dock it here</button>
        {s.float !== "open" && <button type="button" className="btn" onClick={() => { askStore.setFloat("open"); focusComposer(); }}>Show the panel</button>}
      </div>
    </div>
  );
}
