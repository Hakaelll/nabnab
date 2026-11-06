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
    out float opacity;
    flat out vec2 cell;
    flat out vec3 ink;
    void main() {
      vec2 uv = (origin - size * vec2(.5, .48)) / 680.;
      float turn = sin(time*.027)*.14;
      uv = vec2(uv.x*cos(turn)-uv.y*sin(turn),uv.x*sin(turn)+uv.y*cos(turn));
      float u = uv.x, v = uv.y, drift = time * .065;
      float fold = sin(u*8.5 + sin(v*5.2-drift*.42)*(2.1+sin(time*.041)*.45) + drift*.55);
      float cross = cos(v*7.2-u*2.8 + sin(u*3.6+drift*.3) + sin(time*.035)*.75);
      float ribbon = exp(-pow(fold-.15,2.)*4.8);
      float hollow = .3 + .7*min(1.,length(vec2(u*2.3,(v+.21)*2.)));
      float depth = .35+(cross+1.)*.25;
      vec2 p = origin + vec2(sin(v*4.8+fold*1.6+drift)*23.,cos(u*5.4+cross*1.3-drift*.7)*19.)*depth;
      p += vec2(fold*cross*15.,sin(u*6.+v*3.+drift*.4)*11.);
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
      float breathe = .85+sin(time*.14+detail.x)*.15;
      float quiet = 1.-.38*exp(-pow((p.x-size.x*.5)/(readingWidth*.7),4.));
      opacity = min(.46,((.045+ribbon*.32)*depth*hollow*breathe+light*.2)*quiet);
      float hue = time/${COLOR_CYCLE_SECONDS.toFixed(1)} + dot(p/size,mix(vec2(.025,.02),vec2(.55,.45),fullColor));
      float saturation = mix(min(.62,chroma*.72),.85,fullColor);
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
      color = vec4(vec3(9./255.)+ink*(194./255.)*opacity*coverage,1.);
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
  const uniforms = {size:uniform("size"),ratio:uniform("ratio"),time:uniform("time"),readingWidth:uniform("readingWidth"),waveCount:uniform("waveCount"),waves:uniform("waves[0]"),ellipses:uniform("ellipses"),letters:uniform("letters"),fullColor:uniform("fullColor")};
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
    draw(time: number, waves: Wave[], fullColor: boolean) {
      if (gl.isContextLost()) return;
      for (let i = 0; i < Math.min(3, waves.length); i++) {
        const wave = waves[i];
        waveData[i*4] = wave.x; waveData[i*4+1] = wave.y;
        waveData[i*4+2] = wave.born; waveData[i*4+3] = wave.strength;
        ellipseData[i] = wave.ellipse;
      }
      gl.uniform1f(uniforms.time,time); gl.uniform1i(uniforms.waveCount,Math.min(3,waves.length));
      gl.uniform1f(uniforms.fullColor,fullColor?1:0);
      gl.uniform4fv(uniforms.waves,waveData); gl.uniform3fv(uniforms.ellipses,ellipseData);
      gl.clearColor(9/255,9/255,9/255,1); gl.clear(gl.COLOR_BUFFER_BIT);
      gl.drawArrays(gl.POINTS,0,count);
    },
    dispose,
  };
}
import { COLOR_CYCLE_SECONDS, COLOR_WAVE_SECONDS } from "./graphics-color";
