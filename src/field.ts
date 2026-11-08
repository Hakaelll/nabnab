import { fieldScenes, type FieldMix } from "./field-motion";
import { COLOR_CYCLE_SECONDS, COLOR_WAVE_SECONDS, CONTINUOUS_COLOR_CYCLE_SECONDS } from "./graphics-color";

export type Glyph = { x: number; y: number; phase: number; depth: number; glyph: number };
export type Wave = { x: number; y: number; born: number; strength: number; ellipse: number };

// One point per glyph: geometry runs on the GPU, and the atlas is uploaded only on resize.
export function createFieldRenderer(canvas: HTMLCanvasElement) {
  const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false, preserveDrawingBuffer: true });
  if (!gl) return null;
  const vertex = `#version 300 es
    precision highp float;
    layout(location=0) in vec2 origin;
    layout(location=1) in vec3 detail;
    uniform vec2 size;
    uniform float ratio;
    uniform float time;
    uniform float readingWidth;
    uniform int waveCount;
    uniform vec4 waves[3];
    uniform vec3 ellipses;
    uniform float fullColor;
    uniform vec3 morph;
    out float opacity;
    flat out vec2 cell;
    flat out vec3 ink;
    // Keep the spatial profiles in sync with field-motion.ts.
    float fieldProfile(int scene, float u, float v) {
      switch (scene) {
        case ${fieldScenes.dunes}: {
          float ridge=v+.15*sin(u*3.4-time*.26)+.064*sin(u*7.+v*1.8+time*.17);
          float crest=pow(.5+.5*cos(ridge*31.+u*3.2+v*v*2.1-time*.58),4.);
          float upper=v+.37+.12*cos(u*2.2-time*.12), lower=v-.38+.15*sin(u*2.+time*.13);
          return crest*(exp(-lower*lower*10.)+.82*exp(-upper*upper*14.));
        }
        case ${fieldScenes.orbit}: {
          float x=u+.08*sin(time*.19), y=v*1.7;
          float radius=length(vec2(x,y)), contour=.72+.08*cos(time*.22+x*3.);
          float arcs=.38+.62*(.5+.5*sin(x*5.-y*3.+time*.43));
          return exp(-pow(abs(radius-contour),2.)*13.)
            *(.26+.74*pow(.5+.5*cos(radius*35.-time*.65),5.))*arcs;
        }
        case ${fieldScenes.interference}: {
          float left=length(vec2(u+.61,(v+.34+.09*sin(time*.16))*1.18));
          float right=length(vec2(u-.59,(v-.31+.08*cos(time*.19))*1.2));
          float front=pow(.5+.5*cos(left*24.-time*.81),4.);
          float back=pow(.5+.5*cos(right*24.-time*.67+1.4),4.);
          float reach=front*exp(-left*left*1.25)+back*exp(-right*right*1.25);
          return .69*reach+.31*front*back;
        }
        case ${fieldScenes.weave}: {
          float a=sin(u*7.3-v*4.6+time*.29), b=sin(u*5.2+v*7.4-time*.39);
          return pow(.5+.5*cos((a+b)*5.7),5.)*(.34+.66*(.5+.5*sin(u*2.9+v*5.+time*.26)));
        }
        case ${fieldScenes.prism}: {
          float x=u+.91+.11*sin(time*.19), y=v+.52;
          float radius=length(vec2(x,y)), angle=atan(y,max(.04,x));
          float spread=angle-.46-.12*sin(time*.18);
          float ribs=pow(.5+.5*cos(angle*38.+.73*sin(radius*8.-time*.45)),3.);
          float light=.65+.35*(.5+.5*cos(radius*18.-time*1.05));
          return (.12+.96*ribs)*light*exp(-spread*spread*4.-pow(abs(radius-.93),2.)*1.2);
        }
        case ${fieldScenes.tide}: {
          float a=u+.38+sin(v*5.+time*.34)*.25, b=u-.44+cos(v*4.-time*.29)*.22;
          float bands=exp(-a*a*17.)+exp(-b*b*19.);
          return bands*(.29+.71*pow(.5+.5*cos(v*15.+u*3.-time*.58),3.));
        }
        default: return 0.;
      }
    }
    void main() {
      vec2 uv = (origin - size * vec2(.5, .48)) / 680.;
      float u = uv.x, v = uv.y, drift = time * .16;
      float first = u*.62 + sin(u*2.3+drift*.75)*.16 + cos(time*.04)*.13 - .5;
      float second = -u*.28 + sin(u*2.4-drift*.55)*.16 + .49;
      float third = .14 + sin(u*2.8+drift*.8)*.21 - .39*u;
      float ribbon = max(exp(-pow(abs((v-first)*4.1),2.)),exp(-pow(abs((v-second)*4.7),2.)));
      ribbon = max(ribbon, .44*exp(-pow(abs((v-third)*5.8),2.)));
      float filament = max(exp(-pow(abs(v-first),2.)*370.),exp(-pow(abs(v-second),2.)*470.));
      filament = max(filament, .63*exp(-pow(abs(v-third),2.)*630.));
      float depth = .45+detail.y*.35;
      vec2 p = origin + vec2(sin(v*2.4+drift)*22.+sin(u*4.1-v*3.1+drift*.8)*6.,
        cos(u*2.8-drift*.7)*17.+sin(v*4.8+u*2.1-drift)*5.)*depth;
      float light = 0.;
      float chroma = 0.;
      for (int i=0;i<3;i++) {
        if (i>=waveCount) break;
        vec2 delta = (p-waves[i].xy)/vec2(1.,ellipses[i]);
        float distance = length(delta), age = time-waves[i].z;
        float offset = distance-age*47.;
        chroma = max(chroma,exp(-offset*offset/11000.)*sin(3.14159265*clamp(age/${COLOR_WAVE_SECONDS.toFixed(1)},0.,1.))*waves[i].w);
        float envelope = exp(-offset*offset/1500.)*pow(max(0.,1.-age/13.),1.5);
        float displacement = sin(offset*.065)*envelope*waves[i].w;
        p += delta/max(1.,distance)*displacement*vec2(17.,12.);
        light += envelope*waves[i].w*(.6+cos(offset*.05)*.4);
      }
      float breathe = .81+sin(time*.22+u*4.8-v*2.7)*.19;
      float twinkle = .5+.5*sin(detail.x+time*.63+u*.9);
      float crest = pow(.5+.5*cos(v*7.-u*4.+time*.41),4.);
      vec2 eddies = vec2(length(vec2(u+.92+.12*sin(drift), (v-.48)*1.22)),
        length(vec2(u-.88+.14*cos(drift*.8), (v+.44)*1.16)));
      float etch = pow(.5+.5*cos(eddies.x*25.-time*.74+.83*sin(eddies.y*11.+drift)),12.) * exp(-eddies.x*1.65);
      etch += pow(.5+.5*cos(eddies.y*30.+time*.67+.71*sin(eddies.x*9.-drift)),16.) * exp(-eddies.y*1.55);
      float fold = sin(u*4.4-v*2.6+drift*1.4) + .7*sin(v*5.6+u*2.7-drift);
      float lattice = pow(.5+.5*cos(fold*9.+drift*1.6),14.);
      float sweep = pow(.5+.5*cos(u*4.8+v*2.4-time*.67),5.);
      float glints = filament*lattice*(.35+.65*sweep);
      float quiet = 1.-.83*exp(-pow(abs((p.x-size.x*.56)/(readingWidth*.68)),4.)-pow(abs((p.y-size.y*.50)/(size.y*.31)),4.));
      int from=int(morph.x), to=int(morph.y);
      float blend=morph.z;
      float current=from==${fieldScenes.current} ? 1.-blend : to==${fieldScenes.current} ? blend : 0.;
      float variation=fieldProfile(from,u,v);
      if (blend>0.) variation=mix(variation,fieldProfile(to,u,v),blend);
      opacity = min(.58,((.027+current*(ribbon*(.39+.12*crest)+filament*.17)+variation*.68)*depth*breathe*(.86+.24*twinkle)
        +light*.19+etch*(.14+.21*current)+glints*.15*current)*quiet);
      float hue = time/mix(${COLOR_CYCLE_SECONDS.toFixed(1)},${CONTINUOUS_COLOR_CYCLE_SECONDS.toFixed(1)},fullColor) + dot(p/size,vec2(.025,.02));
      hue += .075*fold + (1.-fullColor)*time/120.;
      float ambient = max((.09+.30*sweep)*max(ribbon*current,etch),(.20+.28*sweep)*variation);
      float saturation = mix(max(ambient,min(.72,chroma*.87)),max(.19+ambient,min(.85,chroma)),fullColor);
      vec3 spectrum = clamp(abs(fract(hue+vec3(0.,2./3.,1./3.))*6.-3.)-1.,0.,1.);
      ink = mix(vec3(1.),spectrum,saturation);
      cell = vec2(detail.z,detail.y>.55 ? 1. : 0.);
      vec2 position = (floor(p*ratio+.5)/ratio+7.)/size;
      gl_Position = vec4(position.x*2.-1.,1.-position.y*2.,0.,1.);
      gl_PointSize = 14.*ratio;
    }`;
  const fragment = `#version 300 es
    precision mediump float;
    uniform sampler2D atlas;
    uniform float letters;
    in float opacity;
    flat in vec2 cell;
    flat in vec3 ink;
    out vec4 color;
    void main() {
      vec2 p = gl_PointCoord*14.;
      if (p.x>=12. || opacity<.025) discard;
      vec2 uv = (cell*vec2(12.,14.)+p)/vec2(letters*12.,28.);
      float coverage = texture(atlas,uv).a;
      color = vec4(vec3(9.,10.,12.)/255.+ink*(210./255.)*opacity*coverage,1.);
    }`;
  function compile(type: number, source: string) {
    const shader = gl!.createShader(type)!;
    gl!.shaderSource(shader, source); gl!.compileShader(shader);
    if (!gl!.getShaderParameter(shader, gl!.COMPILE_STATUS)) {
      const reason = gl!.getShaderInfoLog(shader);
      gl!.deleteShader(shader);
      throw new Error(reason || "Character field shader could not compile.");
    }
    return shader;
  }
  const program = gl.createProgram()!;
  const buffer = gl.createBuffer()!;
  const texture = gl.createTexture()!;
  const shaders: WebGLShader[] = [];
  function dispose() {
    shaders.forEach(shader=>gl!.deleteShader(shader));
    gl!.deleteProgram(program); gl!.deleteBuffer(buffer); gl!.deleteTexture(texture);
  }
  try {
    shaders.push(compile(gl.VERTEX_SHADER, vertex));
    shaders.push(compile(gl.FRAGMENT_SHADER, fragment));
    shaders.forEach(shader=>gl.attachShader(program,shader));
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program,gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) || "Character field could not link.");
  } catch (error) { dispose(); throw error; }
  const uniform = (name: string) => gl.getUniformLocation(program,name);
  const uniforms = {size:uniform("size"),ratio:uniform("ratio"),time:uniform("time"),readingWidth:uniform("readingWidth"),waveCount:uniform("waveCount"),waves:uniform("waves[0]"),ellipses:uniform("ellipses"),letters:uniform("letters"),fullColor:uniform("fullColor"),morph:uniform("morph")};
  const waveData = new Float32Array(12), ellipseData = new Float32Array(3);
  let count = 0;
  return {
    resize(width: number, height: number, ratio: number, glyphs: Glyph[], atlas: HTMLCanvasElement, letters: number) {
      gl.useProgram(program);
      gl.viewport(0,0,canvas.width,canvas.height);
      gl.bindBuffer(gl.ARRAY_BUFFER,buffer);
      const data = new Float32Array(glyphs.length*5);
      glyphs.forEach((g,i)=>data.set([g.x,g.y,g.phase,g.depth,g.glyph],i*5));
      gl.bufferData(gl.ARRAY_BUFFER,data,gl.STATIC_DRAW);
      gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0,2,gl.FLOAT,false,20,0);
      gl.enableVertexAttribArray(1); gl.vertexAttribPointer(1,3,gl.FLOAT,false,20,8);
      gl.bindTexture(gl.TEXTURE_2D,texture);
      gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA,gl.RGBA,gl.UNSIGNED_BYTE,atlas);
      gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);
      gl.uniform2f(uniforms.size,width,height); gl.uniform1f(uniforms.ratio,ratio);
      gl.uniform1f(uniforms.readingWidth,Math.min(width-64,width>=1500 ? 740 : width>=900 ? 600 : 480));
      gl.uniform1f(uniforms.letters,letters);
      gl.enable(gl.BLEND); gl.blendEquation(gl.MAX); gl.blendFunc(gl.ONE,gl.ONE);
      count = glyphs.length;
    },
    draw(time: number, waves: Wave[], fullColor: boolean, morph: FieldMix) {
      if (gl.isContextLost()) return;
      for (let i = 0; i < Math.min(3, waves.length); i++) {
        const wave = waves[i];
        waveData[i*4] = wave.x; waveData[i*4+1] = wave.y;
        waveData[i*4+2] = wave.born; waveData[i*4+3] = wave.strength;
        ellipseData[i] = wave.ellipse;
      }
      gl.uniform1f(uniforms.time,time); gl.uniform1i(uniforms.waveCount,Math.min(3,waves.length));
      gl.uniform1f(uniforms.fullColor,fullColor?1:0);
      gl.uniform3f(uniforms.morph,...morph);
      gl.uniform4fv(uniforms.waves,waveData); gl.uniform3fv(uniforms.ellipses,ellipseData);
      gl.clearColor(9/255,10/255,12/255,1); gl.clear(gl.COLOR_BUFFER_BIT);
      gl.drawArrays(gl.POINTS,0,count);
    },
    dispose,
  };
}
