"""Compare the two alpine render paths on three camera positions in headless WebGL2.

python tests/compare_alpine_cache.py [--width 640 --height 360 --atlas 1536]
Requires the same Playwright installation as run_webgl.py. Writes comparison.png and timings.json in build/alpine-cache.
"""
import argparse
import base64
import json
import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "skills/shader-art-coding/scripts"))
import build_alpine_cache
import glsl_tools as gt

COMPARE = """
({sources, w, h, atlas}) => {
  if (!gl.getExtension('EXT_color_buffer_float')) throw Error('Floating-point render target required');
  const [original, bake, cached] = sources.map(src => {
    const result = program(src);
    if (!result.ok) throw Error(result.log);
    return result.p;
  });
  function atlasTarget() {
    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA16F, atlas, atlas);
    for (const key of [gl.TEXTURE_MIN_FILTER, gl.TEXTURE_MAG_FILTER])
      gl.texParameteri(gl.TEXTURE_2D, key, gl.LINEAR);
    for (const key of [gl.TEXTURE_WRAP_S, gl.TEXTURE_WRAP_T])
      gl.texParameteri(gl.TEXTURE_2D, key, gl.CLAMP_TO_EDGE);
    const fb = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw Error('Incomplete atlas');
    return {fb,tex};
  }
  const first = atlasTarget(), feedback = atlasTarget();
  const probe = new Float32Array(4);
  function bakePass(from, to, frame) {
    gl.bindTexture(gl.TEXTURE_2D, from.tex);
    gl.bindFramebuffer(gl.FRAMEBUFFER, to.fb);
    const start = performance.now();
    draw(bake, atlas, atlas, {iFrame:frame,iResolution:[atlas,atlas,1]});
    gl.readPixels(0,0,1,1,gl.RGBA,gl.FLOAT,probe);
    if (probe[1] !== atlas || probe[2] !== atlas) throw Error('Missing feedback metadata');
    return performance.now()-start;
  }
  const bakeMs = bakePass(feedback,first,0);
  const copyMs = bakePass(first,feedback,1);
  gl.bindFramebuffer(gl.FRAMEBUFFER,null);
  gl.bindTexture(gl.TEXTURE_2D,feedback.tex);
  canvas.width=w; canvas.height=h;
  const sheet=document.createElement('canvas');
  sheet.width=w*2; sheet.height=(h+25)*3;
  const ctx=sheet.getContext('2d');
  ctx.fillStyle='#151719'; ctx.fillRect(0,0,sheet.width,sheet.height);
  ctx.font='16px sans-serif'; ctx.fillStyle='white';
  const frames=[];
  for (const [row,time] of [0,10,20].entries()) {
    const ms=[[],[]], pixels=[new Uint8Array(w*h*4),new Uint8Array(w*h*4)];
    for (let pass=0;pass<6;pass++) for(let j=0;j<2;j++){
      const k=(pass+j)%2;
      const start=performance.now();
      draw([original,cached][k],w,h,{iResolution:[w,h,1],iTime:time,iFrame:0});
      gl.readPixels(0,0,w,h,gl.RGBA,gl.UNSIGNED_BYTE,pixels[k]);
      if (pass>0) ms[k].push(performance.now()-start);
      if (pass===5) {
        ctx.fillText((k?'Cached':'Procedural')+'   t = '+time+' s',k*w+8,row*(h+25)+18);
        ctx.drawImage(canvas,k*w,row*(h+25)+25);
      }
    }
    let error=0,luma=0;
    const hist=new Uint32Array(256);
    for(let i=0;i<pixels[0].length;i++) if (i%4!==3) {
      const delta=Math.abs(pixels[0][i]-pixels[1][i]);
      error+=delta; hist[delta]++; luma+=pixels[0][i];
    }
    const n=w*h*3;
    let cumulative=0, p95=0;
    for(let i=0;i<256;i++){cumulative+=hist[i];if(cumulative>=0.95*n){p95=i;break;}}
    if (gl.getError()!==gl.NO_ERROR || luma/n<8 || error/n>8) throw Error('Frame mismatch at t='+time);
    frames.push({time,meanAbsRGB:error/n,p95,medianMs:ms.map(a=>[...a].sort((x,y)=>x-y)[2]),samplesMs:ms});
  }
  return {renderer:results.renderer, width:w,height:h,atlas,bakeMs,copyMs,frames,png:sheet.toDataURL()};
}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name, default in [("width", 640), ("height", 360), ("atlas", 1536)]:
        parser.add_argument("--" + name, type=int, default=default)
    args = parser.parse_args()
    variants = build_alpine_cache.passes()
    baseline = gt.bundle_source(gt.Library(), (gt.TEMPLATE_DIR / "alpine-valley.glsl").read_text())
    prelude = gt.SHADERTOY_PRELUDE.replace("uniform sampler2D", "uniform highp sampler2D")
    shaders = [prelude + s + gt.MAIN_COMPLETE for s in (baseline, variants["buffer-a"], variants["image"])]
    empty = json.dumps(dict(units={}, renders=[], tests=[], lint=[]))
    html = (ROOT / "tests/harness_template.html").read_text().replace("/*__DATA__*/null", empty)
    with sync_playwright() as pw:
        browser = pw.chromium.launch(args=["--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"])
        page = browser.new_page()
        page.set_content(html)
        page.wait_for_function("window.DONE")
        result = page.evaluate(COMPARE, dict(sources=shaders, w=args.width, h=args.height, atlas=args.atlas))
    out = ROOT / "build/alpine-cache"
    out.mkdir(parents=True, exist_ok=True)
    (out / "comparison.png").write_bytes(base64.b64decode(result.pop("png").split(",")[1]))
    (out / "timings.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
