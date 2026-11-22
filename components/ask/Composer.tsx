"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { AssistantRuntimeProvider, ComposerPrimitive, useAui, useExternalStoreRuntime, type AppendMessage, type ThreadMessage } from "@assistant-ui/react";
import { ArrowUp, AtSign, Slash, Square, X } from "lucide-react";
import { parseComposer, type Ref, type ToolId } from "@/lib/ask";
import { useMentions } from "./MentionInput";
import "./composer.css";

export interface ComposerProps {
  onSubmit: (text: string, opts: { ids?: string[]; tool?: ToolId }) => void;
  /** An answer is streaming: show Stop instead of Send. */
  busy?: boolean;
  onStop?: () => void;
  /** A removable @name chip for the selected node. */
  context?: { id: string; name: string } | null;
  onClearContext?: () => void;
  /** When it changes to a non-empty value: prefill and focus, don't send. */
  initialQuestion?: string;
  placeholder?: string;
  /** The model picker, rendered in the action row. */
  picker?: ReactNode;
  /** Default "ask-input" (other code focuses #ask-input). */
  inputId?: string;
  /** Controlled draft, so it survives the chat moving between places. Uncontrolled when omitted. */
  value?: string;
  refs?: Ref[];
  onDraftChange?: (value: string, refs: Ref[]) => void;
}

const NO_MESSAGES: readonly ThreadMessage[] = [];
const textOf = (m: AppendMessage) => m.content.map((p) => (p.type === "text" ? p.text : "")).join("");

/**
 * The Ask composer, built on assistant-ui's ComposerPrimitive. Cartograph keeps its own thread (components/ask/store.ts),
 * so this wraps only the composer in a minimal external-store runtime: no messages, `isRunning` from `busy`,
 * send goes to `onSubmit` and cancel to `onStop`.
 */
export default function Composer(props: ComposerProps) {
  const { onSubmit, onStop, busy = false } = props;
  const [ownText, setOwnText] = useState("");
  const [ownRefs, setOwnRefs] = useState<Ref[]>([]);
  const text = props.value ?? ownText;
  const refs = props.refs ?? ownRefs;

  // The runtime's callbacks read the latest props through this ref.
  const live = useRef({ refs, onSubmit, onStop, onDraftChange: props.onDraftChange });
  live.current = { refs, onSubmit, onStop, onDraftChange: props.onDraftChange };

  const setDraft = (v: string, r: Ref[]) => {
    setOwnText(v);
    setOwnRefs(r);
    live.current.onDraftChange?.(v, r);
  };
  const setDraftRef = useRef(setDraft);
  setDraftRef.current = setDraft;

  const runtime = useExternalStoreRuntime({
    messages: NO_MESSAGES,
    isRunning: busy,
    onNew: async (m) => {
      const q = textOf(m);
      if (!q.trim()) return;
      const { refs: r, onSubmit: send } = live.current;
      const p = parseComposer(q, r);
      setDraftRef.current("", []);
      send(q, { ids: p.ids, tool: p.tool });
    },
    onCancel: async () => live.current.onStop?.(),
  });

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <ComposerBody {...props} busy={busy} text={text} refs={refs} setDraft={setDraft} />
    </AssistantRuntimeProvider>
  );
}

function ComposerBody({
  busy, context, onClearContext, initialQuestion, placeholder, picker, inputId = "ask-input", text, refs, setDraft,
}: ComposerProps & { busy: boolean; text: string; refs: Ref[]; setDraft: (v: string, r: Ref[]) => void }) {
  const aui = useAui();
  const field = useRef<HTMLTextAreaElement>(null);

  // Writes text into the assistant-ui composer and reports it to the owner of the draft.
  const setText = (v: string, r: Ref[]) => {
    if (aui.composer().getState().text !== v) aui.composer().setText(v);
    setDraft(v, r);
  };
  const mention = useMentions({ field, text, refs, setText });

  // The draft is owned outside (store or local state): push outside changes into the composer.
  useEffect(() => {
    if (aui.composer().getState().text !== text) aui.composer().setText(text);
  }, [aui, text]);

  // A question handed in from elsewhere: prefill and focus, never send.
  useEffect(() => {
    if (!initialQuestion?.trim()) return;
    setText(initialQuestion, []);
    requestAnimationFrame(() => {
      const el = field.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialQuestion]);

  return (
    <ComposerPrimitive.Root className="ask-cmp" data-busy={busy || undefined}>
      {mention.list}
      {context && (
        <div className="ask-cmp-ctx">
          <span className="ask-cmp-chip">
            <AtSign size={12} aria-hidden />{context.name}
            <button type="button" aria-label={`Remove @${context.name} from the question`} onClick={onClearContext}><X size={12} aria-hidden /></button>
          </span>
          <span className="ask-cmp-ctx-hint">Used when your question names no file</span>
        </div>
      )}
      <label htmlFor={inputId} className="ask-cmp-sr">Ask a question about the codebase</label>
      <ComposerPrimitive.Input
        id={inputId}
        ref={field}
        className="ask-cmp-input"
        placeholder={placeholder}
        rows={1}
        maxRows={6}
        cancelOnEscape={false}
        unstable_focusOnRunStart={false}
        unstable_focusOnScrollToBottom={false}
        unstable_focusOnThreadSwitched={false}
        {...mention.fieldProps}
        onKeyDown={(e) => {
          mention.fieldProps.onKeyDown(e);
          // While an answer streams, Enter does nothing (no stray newline); Stop is the button.
          if (!e.defaultPrevented && busy && e.key === "Enter" && !e.shiftKey) e.preventDefault();
        }}
        onChange={(e) => {
          const v = e.target.value;
          setDraft(v, refs);
          mention.sync(v, e.target.selectionStart ?? v.length);
        }}
      />
      <div className="ask-cmp-actions">
        <button type="button" className="ask-cmp-tool" aria-label="Name a file, module or service" title="Name a file, module or service" onMouseDown={(e) => e.preventDefault()} onClick={() => mention.insert("@")}>
          <AtSign size={15} aria-hidden />
        </button>
        <button type="button" className="ask-cmp-tool" aria-label="Run a tool" title="Run a tool" onMouseDown={(e) => e.preventDefault()} onClick={() => mention.insert("/")}>
          <Slash size={15} aria-hidden />
        </button>
        <span className="ask-cmp-spacer" />
        {picker && <span className="ask-cmp-picker">{picker}</span>}
        {busy ? (
          <ComposerPrimitive.Cancel className="ask-cmp-send is-stop" aria-label="Stop the answer" title="Stop">
            <Square size={12} fill="currentColor" aria-hidden />
          </ComposerPrimitive.Cancel>
        ) : (
          <ComposerPrimitive.Send className="ask-cmp-send" aria-label="Ask" title="Ask">
            <ArrowUp size={16} aria-hidden />
          </ComposerPrimitive.Send>
        )}
      </div>
    </ComposerPrimitive.Root>
  );
}
