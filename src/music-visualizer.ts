import fragment from "./music-scenes.glsl?raw";
import type { PlaybackSignal } from "./playback-signal";

export const scenes = [
  ["ink", "Tinta", "Ink"], ["vortex", "Vórtice", "Vortex"], ["silk", "Seda", "Silk"],
  ["interference", "Interferencia", "Interference"], ["orbit", "Órbita", "Orbit"],
  ["prism", "Prisma", "Prism"], ["rain", "Lluvia", "Rain"], ["aurora", "Aurora", "Aurora"],
] as const;
export type Quality = "auto" | "high" | "low" | "off";

const vertex = `#version 300 es
out vec2 uv;
void main() {
  vec2 point = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  uv = point;
  gl_Position = vec4(2. * point - 1., 0., 1.);
}`;

const quiet: PlaybackSignal = { bass: 0, mids: 0, highs: 0, level: 0, attack: 0, stereo: 0, waveform: new Float32Array(48) };

function programFor(gl: WebGL2RenderingContext): WebGLProgram {
  const program = gl.createProgram();
  if (!program) throw Error("WebGL program unavailable");
  try {
    for (const [kind, code] of [[gl.VERTEX_SHADER, vertex], [gl.FRAGMENT_SHADER, fragment]] as const) {
      const shader = gl.createShader(kind);
      if (!shader) throw Error("WebGL shader unavailable");
      gl.shaderSource(shader, code);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        const message = gl.getShaderInfoLog(shader);
        gl.deleteShader(shader);
        throw Error(message || "WebGL shader compilation failed");
      }
      gl.attachShader(program, shader);
      gl.deleteShader(shader);
    }
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw Error(gl.getProgramInfoLog(program) || "WebGL link failed");
    return program;
  } catch (error) { gl.deleteProgram(program); throw error; }
}

function renderer(canvas: HTMLCanvasElement) {
  const context = canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false, powerPreference: "low-power" });
  if (!context) throw Error("WebGL 2 unavailable");
  const gl = context;
  const program = programFor(gl);
  const location = (name: string) => gl.getUniformLocation(program, name);
  const uniforms = {
    size: location("size"), signal: location("signal"), dynamics: location("dynamics"), wave: location("wave[0]"),
    clock: location("clock"), delta: location("delta"), blend: location("blend"), color: location("colorMode"),
    scene: location("scene"), previous: location("previous"),
  };
  const textures = [gl.createTexture(), gl.createTexture()];
  const buffers = [gl.createFramebuffer(), gl.createFramebuffer()];
  let current = 0;
  let allocated = false;

  function clear() {
    for (const buffer of buffers) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, buffer);
      gl.clear(gl.COLOR_BUFFER_BIT);
    }
  }

  function resize(width: number, height: number) {
    if (allocated && width === canvas.width && height === canvas.height) return;
    canvas.width = width; canvas.height = height;
    gl.viewport(0, 0, width, height);
    gl.useProgram(program);
    gl.uniform2f(uniforms.size, width, height);
    for (const [index, texture] of textures.entries()) {
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      gl.bindFramebuffer(gl.FRAMEBUFFER, buffers[index]);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw Error("WebGL scene buffer incomplete");
    }
    allocated = true;
    clear();
  }

  function draw(scene: number, previous: number, blend: number, color: boolean, intensity: number, clock: number, dt: number, signal: PlaybackSignal) {
    const next = 1 - current;
    gl.bindFramebuffer(gl.FRAMEBUFFER, buffers[next]);
    gl.bindTexture(gl.TEXTURE_2D, textures[current]);
    gl.uniform4f(uniforms.signal, intensity * signal.bass, intensity * signal.mids, intensity * signal.highs, intensity * signal.level);
    gl.uniform2f(uniforms.dynamics, intensity * signal.attack, signal.stereo);
    gl.uniform1fv(uniforms.wave, signal.waveform);
    gl.uniform1f(uniforms.clock, clock);
    gl.uniform1f(uniforms.delta, dt);
    gl.uniform1f(uniforms.blend, blend);
    gl.uniform1f(uniforms.color, Number(color));
    gl.uniform1i(uniforms.scene, scene);
    gl.uniform1i(uniforms.previous, previous);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
    gl.blitFramebuffer(0, 0, canvas.width, canvas.height, 0, 0, canvas.width, canvas.height, gl.COLOR_BUFFER_BIT, gl.NEAREST);
    current = next;
  }

  function release() {
    for (const texture of textures) gl.deleteTexture(texture);
    for (const buffer of buffers) gl.deleteFramebuffer(buffer);
    gl.deleteProgram(program);
  }

  return { resize, draw, clear, release };
}

export interface VisualSettings { scene: number; color: boolean; intensity: number; quality: Quality }

export function createMusicVisualizer(canvas: HTMLCanvasElement, read: (dt: number, playing: boolean) => PlaybackSignal | undefined,
  reducedMotion: () => boolean, failure: (failed: boolean) => void, tick: (seconds: number) => void) {
  let surface: ReturnType<typeof renderer> | undefined;
  let frame = 0, last = 0, nextDue = 0, clock = 0, elapsed = 4, tail = 0, previous = 0, slowFrames = 0;
  let active = false, playing = false, blocked = false, adaptive = false, windowVisible = true;
  let bounds = { width: 0, height: 0 };
  let settings: VisualSettings = { scene: 0, color: false, intensity: 0.78, quality: "auto" };
  const motion = matchMedia("(prefers-reduced-motion: reduce)");
  const still = () => motion.matches || reducedMotion();

  function stop() { if (frame) cancelAnimationFrame(frame); frame = last = nextDue = 0; }

  function schedule() {
    if (active && windowVisible && !blocked && !frame && !document.hidden) frame = requestAnimationFrame(draw);
  }

  function draw(now: number) {
    frame = 0;
    const staticFrame = still();
    const saving = settings.quality === "low" || settings.quality === "auto" && adaptive;
    const interval = 1000 / (saving ? 30 : 60);
    if (now + 2.5 < nextDue) { schedule(); return; }
    const delay = last ? now - last : interval;
    const dt = Math.min(0.1, delay / 1000);
    last = now;
    nextDue = Math.max(now + interval * .56, nextDue ? nextDue + interval : now + interval);
    if (playing && delay > 27) slowFrames++; else slowFrames = Math.max(0, slowFrames - 1);
    if (slowFrames > 18) adaptive = true;

    try {
      surface ??= renderer(canvas);
      const maxWidth = staticFrame || saving ? 700 : settings.quality === "high" ? 1536 : 960;
      const maxHeight = staticFrame || saving ? 420 : settings.quality === "high" ? 864 : 540;
      const scale = Math.min(1, maxWidth / bounds.width, maxHeight / bounds.height);
      surface.resize(Math.max(2, Math.round(bounds.width * scale)), Math.max(2, Math.round(bounds.height * scale)));
      const input = staticFrame ? quiet : read(dt, playing) ?? quiet;
      clock += staticFrame ? 0 : dt * (playing ? .24 + .56 * input.level + .20 * input.mids : tail * .30);
      elapsed = Math.min(3.6, elapsed + dt);
      const progress = staticFrame ? 1 : elapsed / 3.6;
      surface.draw(settings.scene, progress === 1 ? settings.scene : previous,
        progress * progress * (3 - 2 * progress), settings.color, settings.intensity, clock, dt, input);
      failure(false);
      if (playing) tick(dt);
      tail = Math.max(0, tail - dt);
      if (!staticFrame && (playing || tail > 0)) schedule();
      else last = nextDue = 0;
    } catch {
      blocked = true;
      stop();
      failure(true);
    }
  }

  new ResizeObserver(entries => { bounds = entries[0].contentRect; schedule(); }).observe(canvas);
  canvas.addEventListener("webglcontextlost", event => {
    event.preventDefault(); stop(); blocked = true; surface = undefined; failure(true);
  });
  canvas.addEventListener("webglcontextrestored", () => { blocked = false; schedule(); });
  document.addEventListener("visibilitychange", () => { if (document.hidden) stop(); else schedule(); });
  motion.addEventListener("change", schedule);

  return {
    update(next: VisualSettings) {
      if (next.scene !== settings.scene) {
        previous = settings.scene;
        elapsed = playing && !still() ? 0 : 3.6;
      }
      if (next.quality !== settings.quality) { slowFrames = 0; adaptive = false; nextDue = 0; }
      settings = { ...next };
      schedule();
    },
    show() { active = true; schedule(); },
    hide() { active = false; stop(); },
    release() { active = false; stop(); surface?.release(); surface = undefined; },
    visibility(visible: boolean) {
      if (visible === windowVisible) return;
      windowVisible = visible;
      if (visible) schedule(); else stop();
    },
    playing(next: boolean) {
      if (next === playing) return;
      playing = next;
      if (!next) { tail = 0.55; elapsed = 3.6; }
      schedule();
    },
    reset() { if (surface) surface.clear(); tail = 0; schedule(); },
  };
}
