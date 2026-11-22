// FE-07 page script. Bundled by run.mjs with esbuild; loads the fixture into graphology + sigma 4
// and exposes window.bench for the driver. Everything is timed with performance.now() in the page.
import Graph from "graphology";
import Sigma from "sigma";

const q = new URLSearchParams(location.search);
const FILES = q.get("files") ?? "/fixture/files.json";
const SERVICES = q.get("services") ?? "/fixture/services.json";
const PALETTE = ["#4e79a7", "#f28e2b", "#e15759", "#76b7b2", "#59a14f", "#edc948", "#b07aa1", "#ff9da7", "#9c755f", "#bab0ac"];

// The next sigma render, or a 1 s timeout when nothing was scheduled (a render can also run synchronously).
const nextRender = (s) => new Promise((r) => { const t = setTimeout(r, 1000); s.once("afterRender", () => { clearTimeout(t); r(); }); });
const frame = () => new Promise((r) => requestAnimationFrame(r));
const stats = (a) => {
  if (!a.length) return null;
  const s = [...a].sort((x, y) => x - y), p = (k) => s[Math.min(s.length - 1, Math.floor(k * s.length))];
  return { n: s.length, median: +p(0.5).toFixed(2), p95: +p(0.95).toFixed(2), max: +s[s.length - 1].toFixed(2), mean: +(s.reduce((x, y) => x + y, 0) / s.length).toFixed(2) };
};

// GPU time per frame, when the browser exposes EXT_disjoint_timer_query_webgl2 (sigma logs it).
const gpu = [];
const log = console.log.bind(console);
console.log = (...a) => { const m = /GPU time = ([\d.]+)ms/.exec(String(a[0])); if (m) gpu.push(+m[1]); else log(...a); };

function buildFiles(d) {
  const g = new Graph({ type: "directed", multi: false, allowSelfLoops: false });
  const { id, x, y, cluster } = d.nodes;
  for (let i = 0; i < id.length; i++) g.addNode(i, { x: x[i], y: y[i], size: 1.5, color: PALETTE[cluster[i] % PALETTE.length], label: id[i] });
  const { from, to } = d.links;
  for (let i = 0; i < from.length; i++) g.addEdge(from[i], to[i], { size: 0.3, color: "rgba(120,120,140,0.25)" });
  return g;
}
function buildServices(d) {
  const g = new Graph({ type: "directed", multi: false, allowSelfLoops: false });
  const { id, x, y, size } = d.nodes;
  for (let i = 0; i < id.length; i++) g.addNode(i, { x: x[i], y: y[i], size: 3 + Math.sqrt(size[i]) / 3, color: PALETTE[i % PALETTE.length], label: id[i] });
  const { from, to, weight } = d.links;
  for (let i = 0; i < from.length; i++) g.addEdge(from[i], to[i], { size: Math.min(4, 0.5 + Math.log10(weight[i])), color: "rgba(120,120,140,0.4)" });
  return g;
}

async function load() {
  const t0 = performance.now();
  const res = await fetch(FILES);
  const text = await res.text();
  const t1 = performance.now();
  const data = JSON.parse(text);
  const t2 = performance.now();
  const graph = buildFiles(data);
  const t3 = performance.now();
  const container = document.getElementById("map");
  const sigma = new Sigma(graph, container, {
    renderEdgeLabels: false, // the lever the plan names: edge labels off
    hideEdgesOnMove: false,
    DEBUG_gpuTimerQueries: true,
  });
  const t4 = performance.now();
  // sigma may render inside the constructor; either way the first frame is on screen after two rAFs
  await frame(); await frame();
  const t5 = performance.now();
  const svcData = await (await fetch(SERVICES)).json();
  return {
    sigma, graph, svcData,
    load: { fetch_ms: t1 - t0, parse_ms: t2 - t1, build_ms: t3 - t2, sigma_ms: t4 - t3, first_render_ms: t5 - t4, total_ms: t5 - t0, payload_bytes: text.length },
  };
}

let S;
const ready = load().then((r) => { S = r; return r.load; });

// Run a camera animation sequence and time every animation frame and every sigma render.
async function frames({ hide, kind }) {
  const { sigma } = S;
  sigma.setSettings({ hideEdgesOnMove: hide });
  const cam = sigma.getCamera();
  cam.setState({ x: 0.5, y: 0.5, ratio: 1, angle: 0 });
  await frame(); await frame();
  const deltas = [], renders = [];
  let rb = 0;
  const onB = () => { rb = performance.now(); }, onA = () => { renders.push(performance.now() - rb); };
  sigma.on("beforeRender", onB); sigma.on("afterRender", onA);
  gpu.length = 0;
  let running = true, last = performance.now();
  const loop = (t) => { if (!running) return; deltas.push(t - last); last = t; requestAnimationFrame(loop); };
  requestAnimationFrame((t) => { last = t; requestAnimationFrame(loop); });
  // "workspace": stays at whole-workspace zoom (ratio around 1), so all 60k nodes stay drawn.
  // "explore": pans across and zooms in to 0.3 and back, as a user exploring.
  const steps = kind === "workspace"
    ? [{ x: 0.52, y: 0.5, ratio: 0.95 }, { x: 0.48, y: 0.52, ratio: 1.05 }, { x: 0.5, y: 0.48, ratio: 0.97 }, { x: 0.5, y: 0.5, ratio: 1 }]
    : [{ x: 0.75, y: 0.5, ratio: 1 }, { x: 0.25, y: 0.55, ratio: 1 }, { x: 0.4, y: 0.45, ratio: 0.3 }, { x: 0.5, y: 0.5, ratio: 1 }];
  for (const st of steps) await cam.animate(st, { duration: 600, easing: "linear" });
  running = false;
  sigma.off("beforeRender", onB); sigma.off("afterRender", onA);
  await frame(); await frame();
  return { hide, kind, frame_ms: stats(deltas.slice(1)), render_cpu_ms: stats(renders), gpu_ms: stats(gpu.slice()) };
}

// CPU plus GPU cost of each frame sigma draws during the "workspace" camera sequence: a synchronous
// one-pixel readPixels in afterRender waits for the GPU to finish the frame (gl.finish() returns
// early under Chrome's Metal ANGLE). Unlike rAF intervals this isn't rounded up to vsync, so it shows
// the headroom under 33 ms. An upper bound: the forced sync defeats pipelining.
async function gpuFrames(hide) {
  const { sigma } = S;
  sigma.setSettings({ hideEdgesOnMove: hide });
  const gl = sigma.getWebGLContext(), cam = sigma.getCamera(), out = [], px = new Uint8Array(4);
  cam.setState({ x: 0.5, y: 0.5, ratio: 1, angle: 0 });
  await frame(); await frame();
  let t0 = 0;
  const onB = () => { t0 = performance.now(); }, onA = () => { gl.bindFramebuffer(gl.FRAMEBUFFER, null); gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px); out.push(performance.now() - t0); };
  sigma.on("beforeRender", onB); sigma.on("afterRender", onA);
  for (const st of [{ x: 0.52, y: 0.5, ratio: 0.95 }, { x: 0.48, y: 0.52, ratio: 1.05 }, { x: 0.5, y: 0.5, ratio: 1 }]) await cam.animate(st, { duration: 600, easing: "linear" });
  sigma.off("beforeRender", onB); sigma.off("afterRender", onA);
  return { hide, finished_frame_ms: stats(out) };
}

// Viewport positions of k random nodes, for the driver's hover test.
function targets(k, seed = 1) {
  const { sigma, graph } = S;
  const { width, height } = sigma.getDimensions();
  const out = [];
  let s = seed;
  const r = () => ((s = (s * 1103515245 + 12345) % 2147483648) / 2147483648);
  while (out.length < k) {
    const n = Math.floor(r() * graph.order);
    const a = graph.getNodeAttributes(n);
    const p = sigma.graphToViewport({ x: a.x, y: a.y });
    if (p.x > 40 && p.y > 40 && p.x < width - 40 && p.y < height - 40) out.push({ node: n, x: p.x, y: p.y });
  }
  return out;
}

// Hover latency: from the browser's mousemove event to sigma's enterNode (GPU picking is async).
let lastMove = 0, picks = [];
window.addEventListener("mousemove", (e) => { lastMove = e.timeStamp; }, true);
ready.then(() => S.sigma.on("enterNode", () => { picks.push(performance.now() - lastMove); }));
const takePicks = () => { const p = picks; picks = []; return p; };

// Swap the drawn graph for another level's graph, and back, each timed to the next render.
async function swap() {
  const { sigma, graph, svcData } = S;
  const t0 = performance.now();
  const svc = buildServices(svcData);
  let p = nextRender(sigma); // listen first: setGraph may render synchronously
  sigma.setGraph(svc);
  await p; await frame();
  const t1 = performance.now();
  p = nextRender(sigma);
  sigma.setGraph(graph);
  await p; await frame();
  const t2 = performance.now();
  return { to_service_ms: t1 - t0, back_to_files_ms: t2 - t1 };
}

function info() {
  const gl = S.sigma.getWebGLContext();
  const ext = gl.getExtension("WEBGL_debug_renderer_info");
  return {
    renderer: ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
    vendor: ext ? gl.getParameter(ext.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR),
    gpu_timer: !!gl.getExtension("EXT_disjoint_timer_query_webgl2"),
    dpr: devicePixelRatio, width: innerWidth, height: innerHeight,
    nodes: S.graph.order, links: S.graph.size,
    heap_used_bytes: performance.memory?.usedJSHeapSize ?? null,
    user_agent: navigator.userAgent,
  };
}

window.bench = { ready, frames, gpuFrames, targets, takePicks, swap, info };
