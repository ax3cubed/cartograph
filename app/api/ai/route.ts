// One provider-neutral endpoint. POST with `messages` streams a chat completion as plain text;
// POST without `messages` lists the provider's models. Upstream is any OpenAI-compatible base URL.

import { sseDeltas } from "@/lib/ai";

interface Body {
  baseUrl?: string;
  apiKey?: string;
  model?: string;
  effort?: string;
  messages?: { role: string; content: string }[];
}

const fail = (status: number, error: string) => Response.json({ error }, { status });

/** Which upstream to call. A server-side provider (env) always wins. A browser-supplied base URL makes
 *  this route fetch an arbitrary address, so it is only honoured in `next dev` or when the operator opts in.
 *  ponytail: opt-in switch, not an SSRF filter. A hosted multi-user deployment needs provider config stored
 *  server-side plus resolve-validate-pin on the outbound request (OWASP SSRF cheat sheet). */
function provider(body: Body): { base: string; key?: string } | string {
  const envBase = process.env.CARTOGRAPH_AI_BASE_URL;
  if (envBase) return { base: envBase, key: process.env.CARTOGRAPH_AI_API_KEY };
  const allowed = process.env.NODE_ENV === "development" || process.env.CARTOGRAPH_ALLOW_CLIENT_PROVIDER === "1";
  if (!allowed) return "This server does not accept a provider from the browser. Set CARTOGRAPH_AI_BASE_URL, or CARTOGRAPH_ALLOW_CLIENT_PROVIDER=1 for a single-user install.";
  if (!body.baseUrl || !URL.canParse(body.baseUrl) || !/^https?:$/.test(new URL(body.baseUrl).protocol)) {
    return "Add a provider base URL in Settings (http or https).";
  }
  return { base: body.baseUrl, key: body.apiKey };
}

export async function POST(req: Request) {
  const body: Body = await req.json().catch(() => ({}));
  const p = provider(body);
  if (typeof p === "string") return fail(403, p);
  const base = p.base.replace(/\/+$/, "");
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (p.key) headers.authorization = `Bearer ${p.key}`;

  try {
    if (!body.messages) {
      const up = await fetch(`${base}/models`, { headers, signal: req.signal });
      if (!up.ok) return fail(502, `Provider returned ${up.status}: ${(await up.text()).slice(0, 300)}`);
      const json = await up.json();
      const models = (json.data ?? json.models ?? []).map((m: { id?: string; name?: string }) => m.id ?? m.name).filter(Boolean).sort();
      return Response.json({ models });
    }

    if (!body.model) return fail(400, "Pick a model first.");
    const call = (effort?: string) =>
      fetch(`${base}/chat/completions`, {
        method: "POST", headers, signal: req.signal,
        body: JSON.stringify({ model: body.model, messages: body.messages, stream: true, ...(effort && { reasoning_effort: effort }) }),
      });
    let up = await call(body.effort);
    // Not every model or provider takes reasoning_effort (or this level of it); the answer matters more than the knob.
    if (up.status === 400 && body.effort) up = await call();
    if (!up.ok || !up.body) return fail(502, `Provider returned ${up.status}: ${(await up.text()).slice(0, 300)}`);

    let tail = "";
    const text = new TransformStream<string, string>({
      transform(chunk, ctl) {
        const { text, rest } = sseDeltas(tail + chunk);
        tail = rest;
        if (text) ctl.enqueue(text);
      },
    });
    return new Response(up.body.pipeThrough(new TextDecoderStream()).pipeThrough(text).pipeThrough(new TextEncoderStream()), {
      headers: { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" },
    });
  } catch (e) {
    if (req.signal.aborted) return new Response(null, { status: 499 });
    return fail(502, `Could not reach ${base}: ${e instanceof Error ? e.message : e}`);
  }
}
