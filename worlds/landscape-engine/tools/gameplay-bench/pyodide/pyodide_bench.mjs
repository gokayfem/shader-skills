import { pathToFileURL } from "node:url";
const dir = process.argv[2];
const { loadPyodide } = await import(pathToFileURL(dir + "/pyodide.mjs").href);
const t0 = performance.now();
const py = await loadPyodide({ indexURL: dir + "/" });
const startup = performance.now() - t0;
let x = 0;
globalThis.get_x = () => x;
globalThis.set_x = (v) => { x = v; };
py.runPython(`
from js import get_x, set_x
def noop():
    pass
def tick(dt):
    x = get_x()
    set_x(x + dt)
def work(n):
    s = 0.0
    for i in range(n):
        s += i * 0.5
    return s
`);
const noop = py.globals.get("noop"), tick = py.globals.get("tick"), work = py.globals.get("work");
const best = (f) => { let b = 1e9; for (let r = 0; r < 3; r++) { const t = performance.now(); f(); b = Math.min(b, performance.now() - t); } return b; };
const N = 100000, L = 1000000;
const tn = best(() => { for (let i = 0; i < N; i++) noop(); });
const tt = best(() => { for (let i = 0; i < N; i++) tick(0.016); });
const tl = best(() => work(L));
console.log(`pyodide ${py.version}: startup ${startup.toFixed(0)} ms, noop ${(tn*1e6/N).toFixed(0)} ns/call, tick(2 host calls) ${(tt*1e6/N).toFixed(0)} ns/call, loop ${(tl*1e6/L).toFixed(1)} ns/iter`);
