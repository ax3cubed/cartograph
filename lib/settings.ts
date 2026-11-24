// The fixed settings menu (GLOSSARY.md). Slugs are the /settings/[section] routes.
export const SETTINGS_SECTIONS = [
  { slug: "connections", label: "Sources and connections", group: "workspace" },
  { slug: "repositories", label: "Repositories", group: "workspace" },
  { slug: "indexing", label: "Indexing", group: "workspace" },
  { slug: "services", label: "Services and domains", group: "workspace" },
  { slug: "contracts", label: "Call matching", group: "workspace" },
  { slug: "models", label: "Models", group: "ai" },
  { slug: "editor", label: "Use from your editor", group: "ai" },
  { slug: "retrieval", label: "Retrieval and privacy", group: "ai" },
  { slug: "appearance", label: "Appearance", group: "app" },
  { slug: "keyboard", label: "Keyboard", group: "app" },
] as const;
export type SettingsSlug = (typeof SETTINGS_SECTIONS)[number]["slug"];
export const SETTINGS_GROUPS = { workspace: "Workspace", ai: "AI", app: "This app" } as const;

// ---------- keyboard ----------
export interface Shortcut { id: string; group: string; what: string; keys: string[]; set?: boolean }
/** `set` rows are a family of keys (arrows, 1 2 3). They are listed but not rebindable one by one. */
export const DEFAULT_SHORTCUTS: Shortcut[] = [
  { id: "search", group: "Find and move", what: "Search nodes", keys: ["⌘", "K"] },
  { id: "move", group: "Find and move", what: "Move between nodes", keys: ["←", "→", "↑", "↓"], set: true },
  { id: "focus", group: "Find and move", what: "Open focus view", keys: ["↵"] },
  { id: "back", group: "Find and move", what: "Back to the map", keys: ["Esc"] },
  { id: "modes", group: "Map", what: "Network, Adaptive, Ontology", keys: ["1", "2", "3"], set: true },
  { id: "zoom", group: "Map", what: "Zoom in and out", keys: ["+", "−"], set: true },
  { id: "pin", group: "Map", what: "Pin the selected node", keys: ["P"] },
  { id: "link", group: "Map", what: "Inspect the selected link", keys: ["L"] },
  { id: "rail", group: "Layout", what: "Show or hide the service tree", keys: ["["] },
  { id: "sidebar", group: "Layout", what: "Show or hide the sidebar", keys: ["]"] },
  { id: "now", group: "Time", what: "Jump to now", keys: ["T"] },
  { id: "impact", group: "Time", what: "Open commit impact", keys: ["I"] },
  { id: "ask", group: "Ask", what: "Ask the map", keys: ["/"] },
  { id: "stop", group: "Ask", what: "Stop an answer", keys: ["⌘", "."] },
  { id: "inspector", group: "Ask", what: "Open retrieval inspector", keys: ["⌘", "I"] },
  { id: "help", group: "Ask", what: "Show this list", keys: ["?"] },
];
/** Chords the browser keeps for itself, so binding them is a conflict. ⌘ K is not here: the app captures it for search. */
export const BROWSER_KEYS: Record<string, string> = { "⌘ L": "the address bar", "⌘ T": "new tab", "⌘ W": "close tab", "⌘ R": "reload", "⌘ F": "page search", "⌘ N": "new window" };

export const KEYMAP_KEY = "cartograph:keys";
/** Shortcuts with the user's changes applied. Overrides are stored as { id: keys }. */
export function loadKeymap(): Shortcut[] {
  let over: Record<string, string[]> = {};
  try { over = JSON.parse(localStorage.getItem(KEYMAP_KEY) ?? "{}"); } catch { /* storage unavailable */ }
  return DEFAULT_SHORTCUTS.map((s) => (over[s.id] ? { ...s, keys: over[s.id] } : s));
}
export function saveKeymap(list: Shortcut[]) {
  const over: Record<string, string[]> = {};
  for (const s of list) if (s.keys.join() !== DEFAULT_SHORTCUTS.find((d) => d.id === s.id)!.keys.join()) over[s.id] = s.keys;
  try { localStorage.setItem(KEYMAP_KEY, JSON.stringify(over)); } catch { /* storage unavailable */ }
}

// ---------- service detection (IDX-20, board S04) ----------
/** The C6 rule that found a service, as `Service.detected_by` names it, in words. */
const DETECTED_BY: Record<string, string> = {
  manifest: "A services entry in cartograph.yaml",
  scope: "A scope in the workspace file",
  backstage: "Backstage catalog-info.yaml",
  pnpm: "pnpm workspace package",
  "npm-workspaces": "npm workspaces package",
  "go.work": "go.work module",
  nx: "Nx project",
  cargo: "Cargo workspace member",
  composer: "Composer package",
  repo: "The whole repository",
};
export const foundVia = (rule: string | null | undefined, library = false) =>
  `${(rule && DETECTED_BY[rule]) || rule || "Not recorded"}${library ? ", no deployable: a library" : ""}`;

// ---------- extractors (IDX-21) ----------
/** What each built-in extractor reads, by `ExtractorInfo.name`. */
export const EXTRACTOR_LABEL: Record<string, { name: string; note: string }> = {
  "ts-imports": { name: "TypeScript and JavaScript", note: "imports and exports" },
  "go-imports": { name: "Go", note: "imports" },
  "py-imports": { name: "Python", note: "imports" },
  "php-imports": { name: "PHP", note: "use, require and include" },
  "manifest-js": { name: "package.json and tsconfig", note: "packages and path aliases" },
  "manifest-go": { name: "go.mod and go.work", note: "modules" },
  "manifest-py": { name: "pyproject.toml and setup.cfg", note: "packages" },
  "manifest-php": { name: "composer.json", note: "packages and autoload paths" },
};
