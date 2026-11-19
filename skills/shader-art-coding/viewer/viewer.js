const element = id => document.getElementById(id);
const canvas = element('view');
const gl = canvas.getContext('webgl2', {antialias:false, preserveDrawingBuffer:true});
const title = name => name.split('-').map(word=>word[0].toUpperCase()+word.slice(1)).join(' ');
const menu = element('scene'), modes = element('mode');
for (const name of Object.keys(DATA.scenes).sort()) menu.add(new Option(title(name),name));
menu.value = Object.hasOwn(DATA.scenes,location.hash.slice(1)) ? location.hash.slice(1) : 'lensed-disk';
let image, buffer, targets=[], frame=0, time=0, playing=true, previous=0, nextFrame=0;
let mouse=[0,0,0,0], scene, vs;
const adaptiveWidths=[320,480,640,800,960,1280];
let cadence=[], fastWindows=0, recoveryWindows=2;
function resetCadence() {
    cadence=[]; fastWindows=0; recoveryWindows=2;
}

function resizeOutput(width) {
    if (canvas.width===width) return;
    canvas.width=width;
    canvas.height=width*9/16;
    if (scene.kind==='transfer' && targets.length) {
        gl.deleteTexture(targets[0].tex);
        gl.deleteFramebuffer(targets[0].fb);
        targets[0]=target(canvas.width,canvas.height,true);
        refreshTransfer();
    }
}
function trackCadence(interval) {
    const mode=element('size').value;
    if (!mode.startsWith('auto') || document.hidden || interval<=0 || interval>=1000) return;
    cadence.push(interval);
    if (cadence.length<45) return;
    const median=cadence.sort((a,b)=>a-b)[22];
    cadence=[];
    const level=adaptiveWidths.indexOf(canvas.width);
    const slow=mode==='auto60'?19:39;
    const fast=mode==='auto60'?17.2:21;
    if (median>slow && level>0) {
        resizeOutput(adaptiveWidths[level-1]);
        fastWindows=0;
        recoveryWindows=Math.min(8,recoveryWindows*2);
    } else if (median<fast && level<adaptiveWidths.length-1) {
        if (++fastWindows>=recoveryWindows) {
            resizeOutput(adaptiveWidths[level+1]);
            fastWindows=0;
        }
    } else {
        fastWindows=0;
    }
    element('scale').value=canvas.width+' × '+canvas.height;
}

// name, label, minimum, maximum, default, increment. Turn just these named constants into live uniforms.
const knobs={
    'patinated-brass':[['SHUFFLE','Stochastic grain',0,1,1,1]],
    'room-radiosity':[['PANEL_X','Lamp position',-0.7,0.7,0.15,0.025],['BOUNCE','Indirect contribution',0,3,1,0.05]],
    'kelp-passage':[['CURRENT','Current strength',0,1.5,0.7,0.05],['SHAFTS','Sun shafts',0,2,0.8,0.05]],
    'sponge-shelf':[['LANTERN','Rim fluorescence',0,3,1.2,0.05]],
    'ribbed-wreck':[['CAMERA_ARC','Orbit angle',0,6.28,0.63,0.02],['DIVE_DEPTH','Dive depth',0,1,0.45,0.02],['MURK','Water attenuation',0.03,0.6,0.16,0.01]],
    'pigment-chamber':[['BOUNCE','Wall bounce',0,3,1,0.05],['PANEL_X','Lamp position',-0.7,0.7,0.15,0.025]],
    'tide-observatory':[['HOUR','Solar hour',0,24,19.7,0.1],['OVERCAST','Cloud cover',0,1,0.32,0.01],['MOON_PHASE','Lunar phase',0,1,0.18,0.01]],
    'woodland-edge':[['CLEARING','Clearing radius',0,3.5,0.7,0.1],['BANK_WIDTH','Wet bank width',0.2,3,1.5,0.1]],
    'cloudline-forest':[['ERROR_PX','Branch error (px)',0.2,8,1.5,0.1],['WIND','Wind strength',0,1.8,0.55,0.05]],
    'lensed-disk':[['INCLINATION','Disk elevation',0.025,0.8,0.17,0.005],['EXPOSURE','Exposure',0.2,3,1.3,0.05]],
    'morpho':[['LAYER_PERIOD_NM','Layer period (nm)',145,184,172,1],['WING_BEAT','Wingbeat rate',0,2,0.62,0.02]],
    'comb-jelly':[['KEY_LIGHT','Side light',0,2,1,0.02],['BEAT_SPEED','Comb beat',0,6,2.6,0.1]],
    'streamline-press':[['RADIUS','Cylinder radius',0.18,0.7,0.37,0.01]],
    'reaction-diffusion':[['FEED','Feed',0.018,0.060,0.035,0.001],['KILL','Removal',0.045,0.070,0.061,0.001]]
};
const diagnosticNames={
    'patinated-brass':{2:'Relief normals',5:'Grain / recess / patina'},
    'room-radiosity':{6:'Indirect radiance'},
    'kelp-passage':{3:'Kelp / wildlife / shafts'}, 'sponge-shelf':{2:'Surface normals'},
    'ribbed-wreck':{4:'Trace cost'}, 'pigment-chamber':{6:'Direct / left / right bounce'},
    'tide-observatory':{3:'Clouds / moon / coast'}, 'cloudline-forest':{4:'Branch refinement'},
    'woodland-edge':{3:'Fern / edge shrub / meadow flowers',5:'Root cover / moisture / crown silhouette'},
    'morpho':{3:'Scale return / pigment / veins'},
    'comb-jelly':{3:'Front / rear / thickness'},
    'lensed-disk':{3:'Direct / secondary disk'},
    'streamline-press':{2:'Flow direction and speed'},
    'reaction-diffusion':{3:'Concentrations U and V'}
};
let parameters={};
const useTransfer=()=>scene?.kind==='transfer' && element('reuse').checked;
function refreshTransfer(){
    if(useTransfer()) draw(buffer,targets[0],null,0);
}
function displaySource(){
    const src=useTransfer()?scene.cachedImage:scene.image;
    return src.replace(/(#define\s+DEBUG_MODE\s+)\d+/,(_,prefix)=>prefix+modes.value);
}
function controls(){
    parameters={};
    const holder=element('controls');holder.replaceChildren();
    for(const [name,desc,min,max,initial,step] of knobs[menu.value]||[]){
        const label=document.createElement('label'), slider=document.createElement('input'), value=document.createElement('output');
        slider.type='range'; Object.assign(slider,{min,max,step,value:initial});
        value.value=String(initial);parameters[name]=initial;
        slider.oninput=()=>{parameters[name]=Number(slider.value);value.value=slider.value;if(name==="INCLINATION")refreshTransfer();if(name==="PANEL_X" && scene.kind==="radiosity")solveRoom();show();};
        label.append(desc,slider,value);holder.append(label);
    }
    holder.hidden=!holder.childElementCount;
}

function compile(type, code) {
    const shader=gl.createShader(type);
    gl.shaderSource(shader,code); gl.compileShader(shader);
    if (!gl.getShaderParameter(shader,gl.COMPILE_STATUS)) {
        const message=gl.getShaderInfoLog(shader); gl.deleteShader(shader); throw Error(message);
    }
    return shader;
}
function link(source,preserveAlpha=false) {
    for(const name of Object.keys(parameters)) {
        const declaration=new RegExp('const\\s+float\\s+'+name+'\\s*=\\s*[0-9.]+\\s*;');
        source=source.replace(declaration,'uniform float '+name+';');
    }
    const p=gl.createProgram(), fs=compile(gl.FRAGMENT_SHADER,DATA.prelude+source+(preserveAlpha?"\nvoid main(){mainImage(st_FragColor,gl_FragCoord.xy);}":DATA.main));
    gl.attachShader(p,vs); gl.attachShader(p,fs); gl.linkProgram(p); gl.deleteShader(fs);
    if (!gl.getProgramParameter(p,gl.LINK_STATUS)) {
        const message=gl.getProgramInfoLog(p); gl.deleteProgram(p); throw Error(message);
    }
    const loc = name=>gl.getUniformLocation(p,name);
    return {p,knobs:Object.keys(parameters).map(k=>[k,loc(k)]),res:loc('iResolution'),frame:loc('iFrame'),time:loc('iTime'),mouse:loc('iMouse')};
}
function target(w,h,fullPrecision=false) {
    const tex=gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D,tex);
    gl.texStorage2D(gl.TEXTURE_2D,1,fullPrecision?gl.RGBA32F:gl.RGBA16F,w,h);
    for (const k of [gl.TEXTURE_MIN_FILTER,gl.TEXTURE_MAG_FILTER]) gl.texParameteri(gl.TEXTURE_2D,k,fullPrecision?gl.NEAREST:gl.LINEAR);
    for (const k of [gl.TEXTURE_WRAP_S,gl.TEXTURE_WRAP_T]) gl.texParameteri(gl.TEXTURE_2D,k,gl.CLAMP_TO_EDGE);
    const fb=gl.createFramebuffer(); gl.bindFramebuffer(gl.FRAMEBUFFER,fb);
    gl.framebufferTexture2D(gl.FRAMEBUFFER,gl.COLOR_ATTACHMENT0,gl.TEXTURE_2D,tex,0);
    if(gl.checkFramebufferStatus(gl.FRAMEBUFFER)!==gl.FRAMEBUFFER_COMPLETE) throw Error('Floating-point framebuffer unavailable.');
    return {tex,fb,w,h};
}
function draw(shader,destination,texture,index) {
    const w=destination?.w??canvas.width, h=destination?.h??canvas.height;
    gl.bindFramebuffer(gl.FRAMEBUFFER,destination?.fb??null); gl.bindTexture(gl.TEXTURE_2D,texture);
    gl.viewport(0,0,w,h); gl.useProgram(shader.p);
    for(const [name,location] of shader.knobs) gl.uniform1f(location,parameters[name]);
    gl.uniform3f(shader.res,w,h,1); gl.uniform1f(shader.time,time); gl.uniform1i(shader.frame,index);
    gl.uniform4f(shader.mouse,mouse[0]*w,mouse[1]*h,mouse[2]*w,mouse[3]*h);
    gl.drawArrays(gl.TRIANGLES,0,3);
}
function show() {
    draw(image,null,targets.length?targets[0].tex:null,frame);
    element('clock').value=scene.kind==='simulation' ? 'Step '+frame : time.toFixed(1)+' s';
}
function advanceSimulation(steps) {
    for(let i=0;i<steps;i++) {
        draw(buffer,targets[1],targets[0].tex,frame++);
        targets.reverse();
    }
}
function solveRoom() {
    for(let bounce=0;bounce<4;bounce++){
        draw(buffer,targets[1],targets[0].tex,bounce);
        targets.reverse();
    }
}
function reset() {
    time=scene.time??0; frame=0; previous=0;
    if(scene.kind==='simulation') {
        advanceSimulation(1);
    }
    show();
}
function load() {
    cancelAnimationFrame(nextFrame);
    if (image) gl.deleteProgram(image.p);
    if (buffer) gl.deleteProgram(buffer.p);
    for(const t of targets){gl.deleteTexture(t.tex);gl.deleteFramebuffer(t.fb);}
    image=null; buffer=null; targets=[];
    resetCadence();
    scene=DATA.scenes[menu.value];
    modes.replaceChildren(...scene.modes.map(n=>new Option(n===0?'Final':(diagnosticNames[menu.value]?.[n]??'Debug '+n),n)));
    element('modeLabel').hidden=scene.modes.length<2;
    element('step').hidden=playing;
    element('error').textContent='';
    element('hint').textContent=scene.kind==='simulation'?'Drag across the image to seed the reaction.':
        scene.kind==='cache'?'Terrain height is baked once into a 1536 × 1536 texture.':'';
    element('reuseLabel').hidden=scene.kind!=='transfer';
    if(scene.kind==='transfer' && !gl.getExtension('EXT_color_buffer_float')){
        element('reuse').checked=false;element('reuse').disabled=true;
    }
    controls(); image=link(displaySource());
    if(scene.buffer && (scene.kind!=='transfer' || !element('reuse').disabled)){
        if(!gl.getExtension('EXT_color_buffer_float')) throw Error('This study requires floating-point render targets.');
        buffer=link(scene.buffer,scene.kind==='transfer');
        if(scene.kind==='transfer') {
            targets.push(target(canvas.width,canvas.height,true));refreshTransfer();
        } else if(scene.kind==='radiosity'){
            targets.push(target(20,4),target(20,4));
            solveRoom();
        } else {
            targets.push(target(scene.kind==='cache'?1536:320,scene.kind==='cache'?1536:180));
            if(scene.kind==='simulation') targets.push(target(320,180));
            else draw(buffer,targets[0],null,0);
        }
    }
    mouse=[0,0,0,0]; reset(); nextFrame=requestAnimationFrame(tick);
}
function tick(now) {
    if(playing){
        if(previous) trackCadence(now-previous);
        time+=previous?Math.min((now-previous)/1000,0.1):0;
        if(scene.kind==='simulation') advanceSimulation(5); else frame++;
        show();
    }
    previous=now; nextFrame=requestAnimationFrame(tick);
}
function attempt(fn){try{fn();}catch(err){element('error').textContent=String(err);}}
menu.onchange=()=>attempt(()=>{location.hash=menu.value;load();});
modes.onchange=()=>attempt(()=>{
    const replacement=link(displaySource());gl.deleteProgram(image.p);image=replacement;show();
});
element('play').onclick=()=>{
    playing=!playing;element('step').hidden=playing;
    element('play').textContent=playing?'Pause':'Play';
};
element('step').onclick=()=>{
    time+=1/60;
    if(scene.kind==='simulation') advanceSimulation(1); else frame++;
    show();
};
element('save').onclick=()=>{
    show();canvas.toBlob(blob=>{
        if(!blob)return;
        const url=URL.createObjectURL(blob), link=document.createElement('a');
        link.href=url;link.download=menu.value+'-'+frame+'.png';link.click();
        setTimeout(()=>URL.revokeObjectURL(url),5000);
    },'image/png');
};
element('reset').onclick=()=>attempt(reset);
element('reuse').onchange=()=>attempt(()=>{
    const replacement=link(displaySource());gl.deleteProgram(image.p);image=replacement;
    refreshTransfer();show();
});
element('size').onchange=()=>attempt(()=>{
    const auto=element('size').value.startsWith('auto');
    resetCadence();
    resizeOutput(auto?640:Number(element('size').value));
    element('scale').hidden=!auto;
    element('scale').value=canvas.width+' × '+canvas.height;
    show();
});
function point(event) {
    const box=canvas.getBoundingClientRect();
    return [(event.clientX-box.left)/box.width,1-(event.clientY-box.top)/box.height];
}
canvas.onpointerdown=e=>{const p=point(e);mouse=[...p,Math.max(p[0],1e-5),p[1]];canvas.setPointerCapture(e.pointerId);};
canvas.onpointermove=e=>{const p=point(e);mouse[0]=p[0];mouse[1]=p[1];};
canvas.onpointerup=canvas.onpointercancel=()=>{mouse[2]=-Math.abs(mouse[2]);mouse[3]=-Math.abs(mouse[3]);};
canvas.addEventListener('webglcontextlost',e=>{e.preventDefault();cancelAnimationFrame(nextFrame);element('error').textContent='WebGL context lost. Waiting for restore.';});
canvas.addEventListener('webglcontextrestored',()=>location.reload());
attempt(()=>{
    if(!gl)throw Error('WebGL2 is unavailable in this browser.');
    vs=compile(gl.VERTEX_SHADER,'#version 300 es\nvoid main(){vec2 p=vec2((gl_VertexID<<1)&2,gl_VertexID&2);gl_Position=vec4(2.*p-1.,0,1);}');
    load();
});
