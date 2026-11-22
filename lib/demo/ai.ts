import type { AskTurn, McpTool, Provider } from "../model.ts";

export const MCP_TOOLS: McpTool[] = [
  { name: "status", summary: "Index state, version and how far behind HEAD it is.", access: "read" },
  { name: "find", summary: "Search services, modules, files and operations by name or meaning.", access: "read" },
  { name: "describe", summary: "What a node does, who owns it and how sure the index is.", access: "read" },
  { name: "neighbors", summary: "What a node depends on and what uses it.", access: "read" },
  { name: "path", summary: "The shortest chain of links between two nodes.", access: "read" },
  { name: "impact", summary: "What a change to a node or commit can reach.", access: "read" },
  { name: "sync", summary: "Catch the index up with the repos.", access: "write" },
  { name: "correct", summary: "Write a correction to the override file, with a reason.", access: "write" },
];

// Demo catalogue only. Settings > Models reads the real state from loadAi(), so keySaved here is never shown.
export const PROVIDERS: Provider[] = [
  { id: "anthropic", name: "Anthropic", keySaved: false, models: ["claude-sonnet-5-5", "claude-haiku-4-5"], local: false },
  { id: "openai", name: "OpenAI", keySaved: false, models: ["gpt-5.1"], local: false },
  { id: "openrouter", name: "OpenRouter", keySaved: false, models: [], local: false },
  { id: "ollama", name: "Ollama", keySaved: false, models: ["llama3.1:8b"], local: true },
];

export const ASK_HISTORY: AskTurn[] = [{
  id: "t1", question: "What breaks if I change schema.ts?", model: "claude-sonnet-5-5",
  answer: "Every service that publishes or consumes events: billing publishes payment.succeeded, notifications consumes it, and auth emits user.created. [1] [2]",
  toolCalls: [
    { tool: "find", args: "schema.ts", result: "shared/events/schema.ts" },
    { tool: "impact", args: "shared/events/schema.ts", result: "9 files in 3 services" },
  ],
  citations: [
    { handle: "[1]", node: "billing/events/publisher.ts", checked: true },
    { handle: "[2]", node: "notify/consumers/paymentEvents.ts", commit: "b25e7c4", checked: true },
  ],
}];
