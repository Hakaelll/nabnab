import sourcePoints from "./emblem-points.json";
import { createFieldRenderer, type Glyph, type Wave } from "./field";
import { COLOR_WAVE_SECONDS, waveDelay, isFullColor, onColorModeChange, spectrumAt, tintPacked } from "./graphics-color";
export { handleEmblemClick } from "./graphics-color";
type Point = { x: number; y: number; z: number; edge: boolean; face: number; hue: number };
const TAU = Math.PI * 2;
const alphabet = "01aexz{}[]<>/+=:;*";
let graphicsTime=0;

export function startGraphics(background: HTMLCanvasElement, emblem: HTMLCanvasElement): () => void {
  let gpu: ReturnType<typeof createFieldRenderer> = null;
  try { gpu = createFieldRenderer(background); }
  catch (error) {
    console.warn("Using the Canvas character field:", error);
    const fallback = background.cloneNode() as HTMLCanvasElement;
    background.replaceWith(fallback); background = fallback;
  }
  const field = gpu ? null : background.getContext("2d", { alpha: false, willReadFrequently: true });
  const mark = emblem.getContext("2d", { willReadFrequently: true });
  if ((!gpu && !field) || !mark) return () => {};
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  const forcedColors = matchMedia("(forced-colors: active)");
  const motionReduced = () => reduced.matches || forcedColors.matches || document.documentElement.classList.contains("reduced-motion");
  const atlas = document.createElement("canvas");
  const ink = atlas.getContext("2d", { willReadFrequently: true })!;
  const masks: { x: number; y: number; coverage: number }[][] = [];
  function prepareGlyphs(ratio: number) {
    const cellWidth = Math.ceil(12 * ratio), cellHeight = Math.ceil(14 * ratio);
    atlas.width = alphabet.length * cellWidth;
    atlas.height = cellHeight * 2;
    ink.fillStyle = "#fff";
    ink.textBaseline = "top";
    for (let row = 0; row < 2; row++) {
      ink.font = `${(row ? 9 : 7) * ratio}px Consolas, monospace`;
      for (let i = 0; i < alphabet.length; i++) ink.fillText(alphabet[i], i * cellWidth, row * cellHeight);
    }
    if (gpu) return;
    const atlasData = ink.getImageData(0, 0, atlas.width, atlas.height).data;
    masks.length = 0;
    for (let row = 0; row < 2; row++) for (let glyph = 0; glyph < alphabet.length; glyph++) {
      const mask = [];
      for (let y = 0; y < cellHeight; y++) for (let x = 0; x < cellWidth; x++) {
        const alpha = atlasData[((row * cellHeight + y) * atlas.width + glyph * cellWidth + x) * 4 + 3];
        if (alpha > 8) mask.push({ x, y, coverage: Math.max(1, alpha >>> 4) });
      }
      masks.push(mask);
    }
  }
  // Preblend glyph ink once; a frame needs two canvas uploads instead of thousands of draw calls.
  const colors = new Uint32Array(64 * 16);
  for (let level = 0; level < 64; level++) for (let coverage = 0; coverage < 16; coverage++) {
    const alpha = level / 128 * coverage / 15;
    const ink = Math.round(210 * alpha);
    colors[level * 16 + coverage] = (255 << 24) | ((12 + ink) << 16) | ((10 + ink) << 8) | (9 + ink);
  }
  let fieldImage: ImageData;
  let fieldPixels: Uint32Array;
  const markImage = mark.createImageData(360, 360);
  const markPixels = new Uint32Array(markImage.data.buffer);
  // Front and rear surfaces share the original mask; edge layers give the rays actual thickness.
  const points: Point[] = [];
  for (const [x, y, edge] of sourcePoints) {
    const radius = Math.hypot(x, y);
    const thickness = 0.085 + Math.max(0, 0.5 - radius) * 0.1;
    const hue=Math.floor(((Math.atan2(y,x)/TAU+1)%1)*512);
    points.push({ x, y, z: -thickness, edge: Boolean(edge), face: -1, hue });
    points.push({ x, y, z: thickness, edge: Boolean(edge), face: 1, hue });
    if (edge) for (const fraction of [-0.5, 0, 0.5]) {
      points.push({ x, y, z: thickness * fraction, edge: true, face: 0, hue });
    }
  }
  let width = 0, height = 0, frame = 0, previous = 0, elapsed = graphicsTime;
  let ratio = 0, pixelWidth = 0, pixelHeight = 0;
  let fieldInterval = 1 / 20;
  let nextWave = elapsed + (isFullColor()?0:waveDelay(false,.5));
  let lastField = -1;
  let glyphs: Glyph[] = [];
  let waves: Wave[] = [];
  let seed = 41;
  const symbolSize = 360;
  function random() { seed = (seed * 1664525 + 1013904223) >>> 0; return seed / 4294967296; }

  function drawField(time: number) {
    const reducedMotion=motionReduced(),fullColor=isFullColor();
    if (gpu) { gpu.draw(time, reducedMotion?[]:waves, fullColor); return; }
    if (!field) return;
    fieldPixels.fill(0xff0c0a09);
    const span = 680;
    const readingWidth = Math.min(width - 64, width >= 1500 ? 740 : width >= 900 ? 600 : 480);
    const drift = time * 0.16;
    for (const g of glyphs) {
      const u = (g.x - width * 0.5) / span, v = (g.y - height * 0.48) / span;
      const first = u * 0.62 + Math.sin(u * 2.3 + drift * 0.35) * 0.16 + Math.cos(time * 0.027) * 0.13 - 0.5;
      const second = -u * 0.28 + Math.sin(u * 2.4 - drift * 0.3) * 0.12 + 0.49;
      const third = .14 + Math.sin(u * 2.8 + drift * .46) * .21 - .39 * u;
      const ribbon = Math.max(Math.exp(-Math.pow((v - first) * 4.1, 2)), Math.exp(-Math.pow((v - second) * 4.7, 2)),
        .44 * Math.exp(-Math.pow((v - third) * 5.8, 2)));
      const filament = Math.max(Math.exp(-Math.pow(v - first, 2) * 370), Math.exp(-Math.pow(v - second, 2) * 470),
        .63 * Math.exp(-Math.pow(v - third, 2) * 630));
      const depth = 0.45 + g.depth * 0.35;
      let x = g.x + (Math.sin(v * 2.4 + drift) * 22 + Math.sin(u * 4.1 - v * 3.1 + drift * .8) * 6) * depth;
      let y = g.y + (Math.cos(u * 2.8 - drift * .7) * 17 + Math.sin(v * 4.8 + u * 2.1 - drift) * 5) * depth;
      let light = 0;
      for (const wave of waves) {
        const dx = x - wave.x, dy = (y - wave.y) / wave.ellipse;
        const distance = Math.hypot(dx, dy);
        const age = time - wave.born;
        const offset = distance - age * 47;
        const envelope = Math.exp(-offset * offset / 1500) * Math.pow(Math.max(0, 1 - age / 13), 1.5);
        const displacement = Math.sin(offset * 0.065) * envelope * wave.strength;
        x += dx / Math.max(1, distance) * displacement * 17;
        y += dy / Math.max(1, distance) * displacement * 12;
        light += envelope * wave.strength * (0.6 + Math.cos(offset * 0.05) * 0.4);
      }
      const breathe = .81 + Math.sin(time * .22 + u * 4.8 - v * 2.7) * .19;
      const twinkle = .5 + .5 * Math.sin(g.phase + time * .63 + u * .9);
      const crest = Math.pow(.5 + .5 * Math.cos(v * 7 - u * 4 + time * .41), 4);
      const eddyA = Math.hypot(u + .92, (v - .48) * 1.22), eddyB = Math.hypot(u - .88, (v + .44) * 1.16);
      const etch = Math.pow(.5 + .5 * Math.cos(eddyA * 25 - time * .74 + .83 * Math.sin(eddyB * 11 + drift)), 12) * Math.exp(-eddyA * 1.65)
        + Math.pow(.5 + .5 * Math.cos(eddyB * 30 + time * .67 + .71 * Math.sin(eddyA * 9 - drift)), 16) * Math.exp(-eddyB * 1.55);
      const quiet = 1 - .79 * Math.exp(-Math.pow((x - width * .56) / (readingWidth * .68), 4) - Math.pow((y - height * .50) / (height * .31), 4));
      const opacity = Math.min(.57, ((.025 + ribbon * (.32 + .08 * crest) + filament * .15) * depth * breathe * (.86 + .24 * twinkle) + light * .14 + etch * .33) * quiet);
      if (opacity < 0.025) continue;
      const level = Math.min(63, Math.round(opacity * 128));
      const palette = level * 16;
      const spectrum = fullColor||(!reducedMotion&&waves.length)?spectrumAt(time,x,y,width,height,waves,fullColor,reducedMotion):null;
      const tinted = spectrum!==null&&spectrum.saturation>.001;
      const tint = spectrum&&tinted?tintPacked(spectrum.hue,spectrum.saturation):0xffffff;
      const redInk=(tint&255)/255,greenInk=((tint>>>8)&255)/255,blueInk=((tint>>>16)&255)/255;
      const mask = masks[g.glyph + (g.depth > 0.55 ? alphabet.length : 0)];
      const left = Math.round(x * ratio), top = Math.round(y * ratio);
      for (const pixel of mask) {
        const px = left + pixel.x, py = top + pixel.y;
        if (px < 0 || py < 0 || px >= pixelWidth || py >= pixelHeight) continue;
        const index = py * pixelWidth + px;
        const color = colors[palette + pixel.coverage];
        if (!tinted) { if ((color & 255) > (fieldPixels[index] & 255)) fieldPixels[index] = color; }
        else {
          const ink=(color&255)-9,previous=fieldPixels[index];
          const red=Math.max(previous&255,Math.round(9+ink*redInk));
          const green=Math.max((previous>>>8)&255,Math.round(10+ink*greenInk));
          const blue=Math.max((previous>>>16)&255,Math.round(12+ink*blueInk));
          fieldPixels[index]=(255<<24)|(blue<<16)|(green<<8)|red;
        }
      }
    }
    field.putImageData(fieldImage, 0, 0);
  }

  function drawEmblem(time: number) {
    if (!mark) return;
    markPixels.fill(0);
    // Negative yaw moves the front face to the right through a continuous full turn.
    const angle = -0.28 - time * 0.15;
    const cy = Math.cos(angle), sy = Math.sin(angle);
    const tilt = -0.3 + Math.sin(time * 0.32) * 0.07;
    const cx = Math.cos(tilt), sx = Math.sin(tilt);
    const fullColor=isFullColor();
    const bounds=emblem.getBoundingClientRect();
    const spectrum=spectrumAt(time,bounds.left+bounds.width/2,bounds.top+bounds.height/2,width,height,waves,fullColor,motionReduced());
    const monochrome=tintPacked(spectrum.hue,spectrum.saturation,235);

    for (const p of points) {
      const rx = p.x * cy + p.z * sy;
      const rz = -p.x * sy + p.z * cy;
      const ry = p.y * cx - rz * sx;
      const z = p.y * sx + rz * cx;
      const perspective = 4.2 / (4.2 + z);
      const facing = p.face === 0 ? 0.5 + Math.abs(sy) * 0.4 : Math.max(0.1, -p.face * cy);
      const light = (p.edge ? 0.55 : 0.3) + facing * 0.34;
      const opacity = Math.min(0.94, light * (0.8 - z * 0.16));
      const x = 180 + (rx * 0.985 - ry * 0.17) * 163 * perspective;
      const y = 180 + (ry * 0.985 + rx * 0.17) * 163 * perspective;
      const left = Math.floor(x), top = Math.floor(y);
      const fx = x - left, fy = y - top;
      const color=monochrome;
      // Bilinear coverage keeps subpixel rotation smooth at the emblem's compact display size.
      for (let dy = 0; dy < 2; dy++) for (let dx = 0; dx < 2; dx++) {
        const px = left + dx, py = top + dy;
        if (px < 0 || py < 0 || px >= symbolSize || py >= symbolSize) continue;
        const index = py * symbolSize + px;
        const alpha = Math.min(255, Math.round(opacity * (dx ? fx : 1 - fx) * (dy ? fy : 1 - fy) * 460));
        if (alpha > (markPixels[index] >>> 24)) markPixels[index] = (alpha << 24) | color;
      }
    }
    mark.putImageData(markImage, 0, 0);
  }
  function draw() {
    if (lastField < 0 || elapsed - lastField >= fieldInterval || motionReduced()) {
      const start = performance.now();
      drawField(elapsed);
      // Adapt cadence rather than scaling the bitmap or enlarging the characters.
      fieldInterval = Math.min(1 / 12, Math.max(gpu ? 1 / 30 : 1 / 24, fieldInterval * 0.9 + (performance.now() - start) / 2500));
      lastField = elapsed;
    }
    drawEmblem(elapsed);
  }
  function resize() {
    // CPU fallback keeps CSS glyph size and density, with a lower physical-resolution budget.
    const nextRatio = Math.min(gpu ? 2 : 1.25, Math.max(1, window.devicePixelRatio || 1));
    if (width === background.clientWidth && height === background.clientHeight && ratio === nextRatio) return;
    width = background.clientWidth;
    height = background.clientHeight;
    ratio = nextRatio;
    pixelWidth = Math.max(1, Math.round(width * ratio));
    pixelHeight = Math.max(1, Math.round(height * ratio));
    background.width = pixelWidth;
    background.height = pixelHeight;
    if (field) {
      fieldImage = field.createImageData(pixelWidth, pixelHeight);
      fieldPixels = new Uint32Array(fieldImage.data.buffer);
    }
    prepareGlyphs(ratio);
    emblem.width = symbolSize;
    emblem.height = symbolSize;
    seed = 41;
    lastField = -1;
    const step = 12;
    glyphs = [];
    for (let y = -24; y < height + 24; y += step) {
      for (let x = -24; x < width + 24; x += step) {
        glyphs.push({ x: x + random() * 1.5, y: y + random() * 1.5,
          phase: random() * TAU, depth: random(), glyph: Math.floor(random() * alphabet.length) });
      }
    }
    gpu?.resize(width, height, ratio, glyphs, atlas, alphabet.length);
    draw();
  }
  function tick(now: number) {
    if (document.hidden || motionReduced()) { frame = 0; previous = 0; return; }
    frame = requestAnimationFrame(tick);
    if (now - previous < 1000 / 30) return;
    if (previous) elapsed += Math.min(0.1, (now - previous) / 1000);
    graphicsTime=elapsed;
    previous = now;
    if (elapsed > nextWave) {
      waves.push({ x: width * (0.12 + random() * 0.76), y: height * random(),
        born: elapsed, strength: 0.55 + random() * 0.3, ellipse: 0.7 + random() * 0.5 });
      nextWave = elapsed + waveDelay(isFullColor(),random());
    }
    waves = waves.filter(wave => elapsed - wave.born < COLOR_WAVE_SECONDS).slice(-3);
    draw();
  }
  function resume() {
    if (frame) cancelAnimationFrame(frame);
    frame = 0; previous = 0;
    if (!document.hidden && !motionReduced()) frame = requestAnimationFrame(tick);
    else if (!document.hidden) draw();
  }
  const observer = new ResizeObserver(resize);
  function contextLost(event: Event) { event.preventDefault(); }
  function contextRestored() { gpu = createFieldRenderer(background); ratio = 0; resize(); resume(); }
  background.addEventListener("webglcontextlost", contextLost);
  background.addEventListener("webglcontextrestored", contextRestored);
  observer.observe(background);
  window.addEventListener("resize", resize);
  document.addEventListener("visibilitychange", resume);
  reduced.addEventListener("change", resume);
  forcedColors.addEventListener("change", resume);
  const unsubscribeColor=onColorModeChange(()=>{nextWave=elapsed+(isFullColor()?0:waveDelay(false,random()));lastField=-1;if(!document.hidden)draw();});
  resize(); resume();
  return () => {
    cancelAnimationFrame(frame); observer.disconnect();
    gpu?.dispose();
    background.removeEventListener("webglcontextlost", contextLost);
    background.removeEventListener("webglcontextrestored", contextRestored);
    window.removeEventListener("resize", resize);
    document.removeEventListener("visibilitychange", resume);
    reduced.removeEventListener("change", resume);
    forcedColors.removeEventListener("change", resume);
    unsubscribeColor();
  };
}
