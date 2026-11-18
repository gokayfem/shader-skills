#!/usr/bin/env python3
"""Run the WebGL2 harness headless in Chromium (SwiftShader on machines without a GPU) and fail on any error.

  pip install playwright && python3 -m playwright install --with-deps chromium
  python3 tests/run_webgl.py [--save-gallery] [--benchmark] [--size 320x180]

Writes build/webgl-results.json. Exit code 0 only if every unit compiles, every property test passes and every
template renders without NaNs or a black frame.
"""

from __future__ import annotations

import json
import sys
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))

import build_harness  # noqa: E402
import serve  # noqa: E402


def main() -> int:
    save = "--save-gallery" in sys.argv
    benchmark = int("--benchmark" in sys.argv)
    size = "320x180"
    if "--size" in sys.argv:
        size = sys.argv[sys.argv.index("--size") + 1]
    width, height = (int(v) for v in size.split("x"))
    build_harness.build(save_gallery=save)

    port = 8799
    server = serve.serve(port)
    threading.Thread(target=server.serve_forever, daemon=True).start()

    from playwright.sync_api import sync_playwright

    with sync_playwright() as pw:
        browser = pw.chromium.launch(args=["--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"])
        page = browser.new_page()
        page.on("console", lambda msg: print("console:", msg.text) if msg.type == "error" else None)
        page.goto(f"http://127.0.0.1:{port}/build/harness.html?w={width}&h={height}&benchmark={benchmark}")
        page.wait_for_function("window.DONE === true", timeout=20 * 60 * 1000, polling=1000)
        results = page.evaluate("window.RESULTS")
        browser.close()
    server.shutdown()

    (ROOT / "build" / "webgl-results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
    print(f"renderer: {results.get('renderer')}")
    compile_fail = [c for c in results["compile"] if not c["ok"]]
    test_fail = [t for t in results["tests"] if not t["ok"]]
    render_fail = [r for r in results["renders"] if not r["ok"]]
    print(f"compiled {len(results['compile']) - len(compile_fail)}/{len(results['compile'])} units")
    print(f"property tests {len(results['tests']) - len(test_fail)}/{len(results['tests'])} passed")
    print(f"renders {len(results['renders']) - len(render_fail)}/{len(results['renders'])} passed")
    for c in compile_fail:
        print(f"COMPILE FAIL {c['where']}\n{c['log']}")
    for t in test_fail:
        print(f"TEST FAIL {t['name']}: {t.get('rates') or t.get('error')}")
    for r in render_fail:
        print(f"RENDER FAIL {r['name']}: {r.get('stats') or r.get('error')}")
    for render in results["renders"]:
        timing = render.get("stats", {}).get("warmMedianMs")
        if timing is not None:
            print(f"{render['name']}: {timing:.2f} ms warm median at {width}x{height}")
    if results.get("fatal"):
        print("FATAL:", results["fatal"])
        return 1
    return 1 if (compile_fail or test_fail or render_fail) else 0


if __name__ == "__main__":
    sys.exit(main())
