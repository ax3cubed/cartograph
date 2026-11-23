// FE-13 and FE-21: the settings pages' reads and writes over the local API. The YAML editing is in yaml.ts.
import type { AgentInstall, AgentStatus, ConfigFile, FileDiff, McpTool } from "./types.gen.ts";
import { authHeaders, get, OFFLINE, RequestError, send } from "./client.ts";
import { expire, setUp } from "./session.ts";

export { readExcludes, setExcludes, setService, type ServiceEntry } from "./yaml.ts";

// ---------- routes ----------

export const ws = (id: string, tail: string) => `/api/v1/w/${encodeURIComponent(id)}/${tail}`;
export type { AgentInstall, AgentStatus, ConfigFile, FileDiff, McpTool };
export const AGENTS = "/api/v1/agents";
export const MCP_TOOLS = "/api/v1/mcp/tools";

// ---------- cartograph.yaml over the API (FE-13): the browser sends values as YAML text, never a path ----------


export const getConfig = (id: string) => get<ConfigFile>(ws(id, "config")).then((r) => r.data);
export const previewConfig = (id: string, yaml: string) => send<FileDiff>("POST", ws(id, "config/preview"), { yaml });

/** PUT with If-Match. A 412 (`precondition_failed`) means the file changed on disk since `etag` was read. */
export async function putConfig(id: string, yaml: string, etag: string): Promise<FileDiff> {
  let res: Response;
  try {
    res = await fetch(ws(id, "config"), {
      method: "PUT", credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer",
      headers: { ...authHeaders(), "X-Cartograph": "1", "Content-Type": "application/json", "If-Match": etag },
      body: JSON.stringify({ yaml }),
    });
  } catch { setUp(false); throw OFFLINE; }
  setUp(true);
  const body = await res.json().catch(() => null);
  if (res.ok && body) return body.data as FileDiff;
  if (res.status === 401) expire();
  const e = body?.error;
  throw e ? new RequestError(e.code, e.what, e.why, e.try, res.status) : new RequestError("internal", `the server answered ${res.status}`, "the response wasn't the API's error shape", "retry, then check the server log", res.status);
}
