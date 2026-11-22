// FE-07 driver: bundles the page, serves it on 127.0.0.1, drives Chrome over CDP and writes JSON.
//
//   node bench/map/run.mjs --work <dir> [--mode headless|headless-gpu|headed] [--runs 10] [--dpr 2] [--chrome <path>]
//
// <dir> must hold deps/node_modules (sigma, graphology, esbuild from package.json) and fixture/
// (from generate.mjs). Results go to <dir>/results/<mode>-dpr<n>.json. Nothing is written to the repo.
import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { readFileSync, existsSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";

const args = process.argv.slice(2);
const opt = (n, d) => { const i = args.indexOf("--" + n); return i < 0 ? d : args[i + 1]; };
const WORK = opt("work");
if (!WORK) throw new Error("--work <dir> is required");
const MODE = opt("mode", "headless");
const RUNS = Number(opt("runs", 10));
const CHROME = opt("chrome", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
const HERE = dirname(fileURLToPath(import.meta.url));
const W = 1440, H = 900, DPR = Number(opt("dpr", 2));
const NAME = `${MODE}-dpr${DPR}`;
const UPTIME_BEFORE = execFileSync("uptime", { encoding: "utf8" }).trim();

// 1. bundle
const esbuild = await import(pathToFileURL(join(WORK, "deps/node_modules/esbuild/lib/main.js")).href);
const bundle = join(WORK, "bundle.js");
await esbuild.build({
  entryPoints: [join(HERE, "bench.mjs")], bundle: true, format: "iife", minify: true, outfile: bundle,
  nodePaths: [join(WORK, "deps/node_modules")], define: { "process.env.NODE_ENV": '"production"' }, logLevel: "warning",
});

// 2. serve
const files = {
  "/": [join(HERE, "index.html"), "text/html"],
  "/bundle.js": [bundle, "text/javascript"],
  "/fixture/files.json": [join(WORK, "fixture/files-60000-200000.json"), "application/json"],
  "/fixture/services.json": [join(WORK, "fixture/services-62.json"), "application/json"],
};
const server = createServer((req, res) => {
  const f = files[new URL(req.url, "http://x").pathname];
  if (!f) { res.writeHead(404).end(); return; }
  res.writeHead(200, { "content-type": f[1], "cache-control": "no-store" }).end(readFileSync(f[0]));
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const url = `http://127.0.0.1:${server.address().port}/`;

// 3. launch Chrome with its own throwaway profile
const profile = join(WORK, `profile-${MODE}`);
rmSync(profile, { recursive: true, force: true }); mkdirSync(profile, { recursive: true });
const flags = [
  "--remote-debugging-port=0", `--user-data-dir=${profile}`, "--no-first-run", "--no-default-browser-check",
  `--window-size=${W},${H}`, "--enable-precise-memory-info", "--disable-renderer-backgrounding",
  "--disable-background-timer-throttling", "--disable-backgrounding-occluded-windows", "--disable-extensions",
];
if (MODE === "headless") flags.push("--headless=new");
if (MODE === "headless-gpu") flags.push("--headless=new", "--enable-gpu", "--use-angle=metal", "--ignore-gpu-blocklist");
if (MODE === "headed") flags.push("--window-position=0,0");
const chrome = spawn(CHROME, [...flags, "about:blank"], { stdio: "ignore" });
const cleanup = () => { try { chrome.kill("SIGTERM"); } catch {} server.close(); };
process.on("exit", cleanup);
process.on("uncaughtException", (e) => { console.error(e); cleanup(); process.exit(1); });
process.on("unhandledRejection", (e) => { console.error(e); cleanup(); process.exit(1); });

let port;
for (let i = 0; i < 100 && !port; i++) {
  await sleep(100);
  const f = join(profile, "DevToolsActivePort");
  if (existsSync(f)) port = readFileSync(f, "utf8").split("\n")[0];
}
if (!port) throw new Error("Chrome did not start");
const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = targets.find((t) => t.type === "page");
const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();

// 4. minimal CDP client
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
let seq = 0; const pending = new Map(); const waiters = [];
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && pending.has(d.id)) { const { r, j } = pending.get(d.id); pending.delete(d.id); d.error ? j(new Error(JSON.stringify(d.error))) : r(d.result); }
  else if (d.method) for (const w of waiters.splice(0)) w(d);
};
const send = (method, params = {}) => new Promise((r, j) => { const id = ++seq; pending.set(id, { r, j }); ws.send(JSON.stringify({ id, method, params })); });
const evaluate = async (expr) => {
  const r = await Promise.race([
    send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true }),
    sleep(120000).then(() => { throw new Error("timed out: " + expr); }),
  ]);
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  return r.result.value;
};

await send("Page.enable"); await send("Runtime.enable");
await send("Emulation.setDeviceMetricsOverride", { width: W, height: H, deviceScaleFactor: DPR, mobile: false });

// RSS of the Chrome process tree we started (kB -> MB). Includes the GPU process's memory.
function treeRssMb() {
  try {
    const rows = execFileSync("ps", ["-A", "-o", "pid=,ppid=,rss="], { encoding: "utf8" }).trim().split("\n").map((l) => l.trim().split(/\s+/).map(Number));
    const kids = new Map(); for (const [p, pp] of rows) { if (!kids.has(pp)) kids.set(pp, []); kids.get(pp).push(p); }
    const rss = new Map(rows.map(([p, , r]) => [p, r]));
    let sum = 0; const stack = [chrome.pid];
    while (stack.length) { const p = stack.pop(); sum += rss.get(p) ?? 0; stack.push(...(kids.get(p) ?? [])); }
    return +(sum / 1024).toFixed(1);
  } catch { return null; }
}

const mouse = (x, y) => send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
const runs = [];
for (let run = 0; run < RUNS; run++) {
  await send("Page.navigate", { url: url + `?files=/fixture/files.json&services=/fixture/services.json&r=${run}` });
  await sleep(300);
  for (let i = 0; i < 200; i++) { if (await evaluate("typeof window.bench === 'object'").catch(() => false)) break; await sleep(50); }
  const load = await evaluate("bench.ready");
  await sleep(500);
  const info = await evaluate("bench.info()");
  await send("HeapProfiler.collectGarbage");
  const heap = await send("Runtime.getHeapUsage");
  const frames = [];
  for (const kind of ["workspace", "explore"]) for (const hide of [false, true]) frames.push(await evaluate(`bench.frames(${JSON.stringify({ hide, kind })})`));
  const finished_frames = [await evaluate("bench.gpuFrames(false)"), await evaluate("bench.gpuFrames(true)")];
  // hover picks at whole-workspace zoom
  const pts = await evaluate(`bench.targets(20, ${run + 1})`);
  await evaluate("bench.takePicks()");
  let hits = 0;
  for (const p of pts) {
    await mouse(4, 4); await sleep(120);
    await mouse(p.x, p.y); await sleep(150);
  }
  const picks = await evaluate("bench.takePicks()");
  hits = picks.length;
  const swap = await evaluate("bench.swap()");
  const rss_mb = treeRssMb();
  runs.push({ run, load, info, heap_after_gc: heap, frames, finished_frames, picks, pick_attempts: pts.length, pick_hits: hits, swap, chrome_tree_rss_mb: rss_mb });
  console.error(`${MODE} run ${run}: load ${load.total_ms.toFixed(0)} ms, heap ${(heap.usedSize / 1048576).toFixed(0)} MB, ` +
    frames.map((f) => `${f.kind}/${f.hide ? "hide" : "show"} p95 ${f.frame_ms?.p95}`).join(", ") + `, finished frame p95 show ${finished_frames[0].finished_frame_ms.p95} hide ${finished_frames[1].finished_frame_ms.p95}, picks ${hits}/${pts.length}`);
}

// 5. summarise
const q = (a, k) => { const s = [...a].sort((x, y) => x - y); return +s[Math.min(s.length - 1, Math.floor(k * s.length))].toFixed(2); };
const summary = {
  load_total_ms: { median: q(runs.map((r) => r.load.total_ms), 0.5), max: q(runs.map((r) => r.load.total_ms), 1) },
  heap_used_after_gc_mb: { median: q(runs.map((r) => r.heap_after_gc.usedSize / 1048576), 0.5), max: q(runs.map((r) => r.heap_after_gc.usedSize / 1048576), 1) },
  chrome_tree_rss_mb: { median: q(runs.map((r) => r.chrome_tree_rss_mb ?? 0), 0.5) },
  frames: {},
  pick_ms: (() => { const all = runs.flatMap((r) => r.picks); return all.length ? { n: all.length, median: q(all, 0.5), p95: q(all, 0.95), max: q(all, 1) } : null; })(),
  finished_frame_ms: Object.fromEntries([0, 1].map((k) => [k ? "hideEdgesOnMove" : "edges shown", {
    median: q(runs.map((r) => r.finished_frames[k].finished_frame_ms.median), 0.5),
    p95_median_of_runs: q(runs.map((r) => r.finished_frames[k].finished_frame_ms.p95), 0.5),
    p95_worst_run: q(runs.map((r) => r.finished_frames[k].finished_frame_ms.p95), 1),
  }])),
  pick_hit_rate: +(runs.reduce((a, r) => a + r.pick_hits, 0) / runs.reduce((a, r) => a + r.pick_attempts, 0)).toFixed(2),
  swap_to_service_ms: { median: q(runs.map((r) => r.swap.to_service_ms), 0.5) },
  swap_back_to_files_ms: { median: q(runs.map((r) => r.swap.back_to_files_ms), 0.5) },
};
for (const kind of ["workspace", "explore"]) for (const hide of [false, true]) {
  const fs = runs.map((r) => r.frames.find((f) => f.kind === kind && f.hide === hide));
  summary.frames[`${kind}/${hide ? "hideEdgesOnMove" : "edges shown"}`] = {
    frame_p95_ms_median_of_runs: q(fs.map((f) => f.frame_ms.p95), 0.5),
    frame_p95_ms_worst_run: q(fs.map((f) => f.frame_ms.p95), 1),
    frame_median_ms: q(fs.map((f) => f.frame_ms.median), 0.5),
    render_cpu_p95_ms: q(fs.map((f) => f.render_cpu_ms?.p95 ?? 0), 0.5),
    gpu_p95_ms: fs[0].gpu_ms ? q(fs.map((f) => f.gpu_ms?.p95 ?? 0), 0.5) : null,
  };
}
const loadavg = execFileSync("uptime", { encoding: "utf8" }).trim();
const out = { mode: MODE, flags, chrome: version.Browser, viewport: { width: W, height: H, dpr: DPR }, renderer: runs[0].info.renderer, vendor: runs[0].info.vendor, gpu_timer: runs[0].info.gpu_timer, uptime_before: UPTIME_BEFORE, uptime_after: loadavg, summary, runs };
mkdirSync(join(WORK, "results"), { recursive: true });
writeFileSync(join(WORK, "results", `${NAME}.json`), JSON.stringify(out, null, 1));
console.log(JSON.stringify({ mode: MODE, chrome: version.Browser, renderer: out.renderer, gpu_timer: out.gpu_timer, uptime_after: loadavg, summary }, null, 1));
ws.close();
cleanup();
await sleep(500);
process.exit(0);
