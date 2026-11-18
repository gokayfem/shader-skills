"""Exercise the standalone gallery, debug selection, feedback simulation and deterministic reset in Chromium."""
import argparse
import base64
import json
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--save-simulation", action="store_true")
    parser.add_argument("--disk-frames", type=int, default=9, help="paired animation frames including warm-up")
    args = parser.parse_args()
    if args.disk_frames < 3:
        parser.error("--disk-frames must be at least 3")
    with sync_playwright() as pw:
        browser = pw.chromium.launch(args=["--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"])
        page = browser.new_page()
        errors = []
        page.on("pageerror", lambda error: errors.append(str(error)))
        page.goto((ROOT / "gallery/studies.html").as_uri())
        page.click("#play")
        page.select_option("#size", "480")
        for mode, steady, slow in [("auto", 32, 50), ("auto60", 18, 25)]:
            page.select_option("#size", mode)
            for interval, windows, expected in [(slow, 1, 480), (steady, 1, 480),
                                                 (16, 3, 480), (16, 1, 640)]:
                page.evaluate("""([ms, windows])=>{
                    for(let i=0;i<45*windows;i++)trackCadence(ms);
                    show();
                }""", [interval, windows])
                assert page.locator("#view").evaluate("c=>c.width") == expected
        page.select_option("#size", "480")
        scenes = page.locator("#scene option").evaluate_all("options => options.map(o=>o.value)")
        checks = {}
        for name in scenes:
            page.select_option("#scene", name)
            page.wait_for_function("element('error').textContent || image !== null")
            assert not page.locator("#error").inner_text(), name + ": " + page.locator("#error").inner_text()
            modes = page.locator("#mode option").evaluate_all("options => options.map(o=>o.value)")
            if len(modes) > 1:
                page.select_option("#mode", modes[-1])
                page.select_option("#mode", "0")
            mean = page.evaluate("""() => {
                show(); const pixels=new Uint8Array(canvas.width*canvas.height*4);
                gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,pixels);
                if(gl.getError()!==gl.NO_ERROR) throw Error('WebGL drawing error');
                let sum=0; for(let i=0;i<pixels.length;i+=4) sum+=pixels[i]+pixels[i+1]+pixels[i+2];
                return sum/(3*canvas.width*canvas.height);
            }""")
            assert 2 < mean < 250, (name, mean)
            assert not page.locator("#error").inner_text(), name
            live = page.evaluate("""() => Object.keys(parameters).every(name => [image, buffer].some(shader =>
                shader && gl.getUniformLocation(shader.p,name)!==null))""")
            assert live, name + ": inactive art control"
            if name == "comb-jelly":
                before = page.locator("#view").evaluate("c=>c.toDataURL()")
                slider = page.get_by_label("Side light", exact=False)
                slider.fill("0")
                assert page.locator("#view").evaluate("c=>c.toDataURL()") != before
                slider.fill("1")
                assert page.locator("#view").evaluate("c=>c.toDataURL()") == before
            checks[name] = round(mean, 2)
        page.select_option("#scene", "room-radiosity")
        radiance = page.evaluate("""() => {
            function read() {
                const t=targets[0], rgb=new Float32Array(t.w*t.h*4);
                gl.bindFramebuffer(gl.FRAMEBUFFER,t.fb);
                gl.readPixels(0,0,t.w,t.h,gl.RGBA,gl.FLOAT,rgb);
                if(gl.getError()!==gl.NO_ERROR) throw Error('Radiosity readback failed');
                return rgb.filter((_,i)=>i%4!==3);
            }
            draw(buffer,targets[1],targets[0].tex,0);
            targets.reverse();
            const direct=read();
            solveRoom();
            const solved=read();
            solveRoom();
            const repeated=read();
            return {finite:solved.every(Number.isFinite), brighter:solved.some((v,i)=>v>direct[i]+0.01),
                    repeat:solved.every((v,i)=>v===repeated[i])};
        }""")
        assert all(radiance.values()), radiance
        lamp = page.get_by_label("Lamp position", exact=False)
        before = page.locator("#view").evaluate("c=>c.toDataURL()")
        lamp.fill("-0.7")
        assert page.locator("#view").evaluate("c=>c.toDataURL()") != before
        page.select_option("#scene", "lensed-disk")
        comparison = page.evaluate("""count => {
            const direct=link(scene.image), bytes=canvas.width*canvas.height*4;
            const frames=[], samples=[[],[]];
            const start=performance.now();refreshTransfer();
            const probe=new Float32Array(4);
            gl.readPixels(0,0,1,1,gl.RGBA,gl.FLOAT,probe);
            const bakeMs=performance.now()-start;
            for (let i=0;i<count;i++) {
                time=0.25+0.65*i;
                const images=[new Uint8Array(bytes),new Uint8Array(bytes)];
                for(let j=0;j<2;j++){
                    const k=(i+j)%2, begin=performance.now();
                    draw(k===0?direct:image,null,k===0?null:targets[0].tex,i);
                    gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,images[k]);
                    if(i>0)samples[k].push(performance.now()-begin);
                }
                let sum=0;
                for(let n=0;n<bytes;n++)sum+=Math.abs(images[0][n]-images[1][n]);
                frames.push(sum/(bytes*0.75));
            }
            gl.deleteProgram(direct.p);
            const median=a=>{a.sort((x,y)=>x-y);const m=(a.length-1)/2;return 0.5*(a[Math.floor(m)]+a[Math.ceil(m)]);};
            return {width:canvas.width,height:canvas.height,frames:count,bakeMs,
                    maxFrameMeanRGB:Math.max(...frames),medianMs:samples.map(median)};
        }""", args.disk_frames)
        assert comparison["maxFrameMeanRGB"] < 0.2, comparison
        page.select_option("#scene", "reaction-diffusion")
        page.click("#reset")
        page.click("#step")
        assert page.locator("#clock").inner_text() == "Step 2"
        with page.expect_download() as download:
            page.click("#save")
        assert download.value.suggested_filename == "reaction-diffusion-2.png"
        data = page.evaluate("""() => {
            function read() {
                const t=targets[0]; gl.bindFramebuffer(gl.FRAMEBUFFER,t.fb);
                const v=new Float32Array(t.w*t.h*4);
                gl.readPixels(0,0,t.w,t.h,gl.RGBA,gl.FLOAT,v);
                if(gl.getError()!==gl.NO_ERROR) throw Error('Simulation readback failed');
                return v;
            }
            reset();const initial=read();advanceSimulation(400);const evolved=read();
            let changed=0, low=1, high=0;
            for(let i=0;i<evolved.length;i+=4){
                const v=evolved[i+1]; if(!Number.isFinite(v)) throw Error('Nonfinite concentration');
                if(Math.abs(v-initial[i+1])>0.08)changed++;
                low=Math.min(low,v);high=Math.max(high,v);
            }
            show();const img=canvas.toDataURL();
            reset();const replay=read();
            return {changed:changed/(initial.length/4),low,high,img,repeat:initial.every((v,i)=>v===replay[i])};
        }""")
        assert data["repeat"] and data["changed"] > 0.1 and 0 <= data["low"] < 0.1 < data["high"] < 1, data.keys()
        img = data.pop("img")
        if args.save_simulation:
            (ROOT / "gallery/reaction-diffusion.png").write_bytes(base64.b64decode(img.split(",")[1]))
        assert not errors, errors
        print(json.dumps({"scenes": len(checks), "meanRGB": checks, "simulation": data, "diskTransfer": comparison}, indent=2))


if __name__ == "__main__":
    main()
