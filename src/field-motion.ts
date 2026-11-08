type FieldMix = readonly [orbit: number, weave: number, tide: number];

const scenes: readonly FieldMix[] = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]];
const hold = 8;
const transition = 6;

export function fieldMix(seconds: number): FieldMix {
  const period = hold + transition;
  const index = Math.floor(seconds / period) % scenes.length;
  const current = scenes[index];
  const position = Math.max(0, (seconds % period - hold) / transition);
  if (!position) return current;
  const next = scenes[(index + 1) % scenes.length];
  const blend = position * position * (3 - 2 * position);
  return [
    current[0] + (next[0] - current[0]) * blend,
    current[1] + (next[1] - current[1]) * blend,
    current[2] + (next[2] - current[2]) * blend,
  ];
}

// Keep the spatial profiles in sync with the WebGL vertex shader.
export function fieldVariation(u: number, v: number, time: number, [orbit, weave, tide]: FieldMix): number {
  let glow = 0;
  if (orbit) {
    const x = u + .08 * Math.sin(time * .19), y = v * 1.7;
    const radius = Math.hypot(x, y);
    const contour = .72 + .08 * Math.cos(time * .22 + x * 3);
    const arcs = .38 + .62 * (.5 + .5 * Math.sin(x * 5 - y * 3 + time * .43));
    glow += orbit * Math.exp(-((radius - contour) ** 2) * 13)
      * (.26 + .74 * (.5 + .5 * Math.cos(radius * 35 - time * .65)) ** 5) * arcs;
  }
  if (weave) {
    const a = Math.sin(u * 7.3 - v * 4.6 + time * .29);
    const b = Math.sin(u * 5.2 + v * 7.4 - time * .39);
    glow += weave * (.5 + .5 * Math.cos((a + b) * 5.7)) ** 5
      * (.34 + .66 * (.5 + .5 * Math.sin(u * 2.9 + v * 5 + time * .26)));
  }
  if (tide) {
    const first = u + .38 + Math.sin(v * 5 + time * .34) * .25;
    const second = u - .44 + Math.cos(v * 4 - time * .29) * .22;
    const bands = Math.exp(-first * first * 17) + Math.exp(-second * second * 19);
    glow += tide * bands * (.29 + .71 * (.5 + .5 * Math.cos(v * 15 + u * 3 - time * .58)) ** 3);
  }
  return glow;
}
