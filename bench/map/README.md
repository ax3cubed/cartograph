# bench/map: the sigma benchmark (FE-07, decision I1)

A standalone harness, not wired into the app. It draws a seeded 60,000-node, 200,000-link graph in
62 clusters with sigma.js 4 and graphology, drives Chrome over the DevTools protocol, and measures
load time, frame time while the camera moves, hover-pick latency, JS heap and the time to swap levels.

Keep installs and outputs outside the repo. `WORK` is any scratch folder:

```sh
WORK=/tmp/fe07
mkdir -p $WORK/deps && cp bench/map/package.json $WORK/deps/
npm install --prefix $WORK/deps --no-audit --no-fund        # sigma, graphology, esbuild (pinned)
node bench/map/generate.mjs $WORK/fixture                     # 60k/200k + 62-node service level, seed 7
node bench/map/run.mjs --work $WORK --runs 10                 # headless Chrome, 1440x900 at DPR 2
node bench/map/run.mjs --work $WORK --runs 10 --dpr 1         # the same at DPR 1
```

`run.mjs` writes `$WORK/results/<mode>-dpr<n>.json` with every run and a summary. It needs Node 22
(built-in WebSocket) and Chrome; pass `--chrome <path>` if Chrome isn't at the macOS default path.
`--mode headed` opens a real window. Check the `renderer` field: if it names SwiftShader, the numbers
are software rendering and don't count against the gate.

What each number is:

- **load**: fetch and `JSON.parse` of the columnar payload, graphology build, `new Sigma`, two frames.
- **frame**: intervals between `requestAnimationFrame` callbacks during a 2.4 s `camera.animate`
  sequence. `workspace` stays near ratio 1 (all 60k nodes drawn); `explore` pans and zooms to 0.3.
  Run with `hideEdgesOnMove` off and on. Edge labels are always off.
- **finished frame**: each frame sigma draws in the `workspace` sequence, timed from `beforeRender`
  to a synchronous one-pixel `readPixels` after `afterRender`, so the GPU work is inside the timing
  and the number isn't rounded to vsync. (`gl.finish()` returns early under Chrome's Metal ANGLE.)
- **pick**: from the browser's `mousemove` (sent by CDP) to sigma's `enterNode`, at workspace zoom.
  sigma 4 picks on the GPU with an async read-back, so this spans one or two frames.
- **heap**: `Runtime.getHeapUsage` after a forced GC. GPU buffers are not in the JS heap; the Chrome
  process-tree RSS is recorded beside it.
- **swap**: `setGraph` to the 62-node service graph and back to the 60k graph, each to its next render.
