"""Compare the cost of a gallery image pass at two fixed resolutions."""
import argparse
import json
import math
import statistics
from pathlib import Path

from playwright.sync_api import sync_playwright

VIEWER = Path(__file__).resolve().parents[1] / "gallery/studies.html"


def summarize(values):
    return {"median_ms": round(statistics.median(values), 2),
            "p95_ms": round(sorted(values)[math.ceil(0.95 * len(values)) - 1], 2)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenes", nargs="*", default=["ribbed-wreck", "kelp-passage", "volumetric-clouds"])
    parser.add_argument("--samples", type=int, default=30)
    parser.add_argument("--chrome", action="store_true", help="use installed desktop Chrome with hardware acceleration")
    args = parser.parse_args()
    if args.samples < 2:
        parser.error("at least two samples are required")
    with sync_playwright() as pw:
        browser = (pw.chromium.launch(channel="chrome", headless=False) if args.chrome else
                   pw.chromium.launch(args=["--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"]))
        page = browser.new_page()
        page.goto(VIEWER.as_uri())
        page.click("#play")
        info = page.evaluate("""()=>{
            const ext=gl.getExtension('WEBGL_debug_renderer_info');
            return ext?gl.getParameter(ext.UNMASKED_RENDERER_WEBGL):gl.getParameter(gl.RENDERER);
        }""")
        results = {}
        for name in args.scenes:
            page.select_option("#scene", name)
            if page.locator("#error").inner_text():
                raise RuntimeError(page.locator("#error").inner_text())
            runs = [[], []]
            # Reverse block order on the second pass. Warm each size after reallocating.
            for level in (0, 1, 1, 0):
                page.select_option("#size", ("480", "800")[level])
                samples = page.evaluate("""count=>{
                    const times=[],pixel=new Uint8Array(4);
                    for(let i=-5;i<count;i++){
                        const start=performance.now();
                        show();
                        gl.readPixels(0,0,1,1,gl.RGBA,gl.UNSIGNED_BYTE,pixel);
                        if(i>=0)times.push(performance.now()-start);
                    }
                    if(gl.getError()!==gl.NO_ERROR)throw Error('Image pass failed');
                    return times;
                }""", args.samples)
                runs[level].extend(samples)
            results[name] = dict(zip(("480x270", "800x450"), map(summarize, runs)))
        print(json.dumps({"browser": browser.version, "renderer": info, "samples_per_size": 2 * args.samples,
                          "image_pass_and_readback": results}, indent=2))
        browser.close()


if __name__ == "__main__":
    main()
