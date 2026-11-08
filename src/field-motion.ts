export const fieldScenes = {
  current: 0, aurora: 1, weave: 2, rain: 3, dunes: 4, contours: 5,
  orbit: 6, constellation: 7, tide: 8, prism: 9, interference: 10,
} as const;
type FieldScene = typeof fieldScenes[keyof typeof fieldScenes];
export type FieldMix = readonly [from: FieldScene, to: FieldScene, blend: number];

const scenes = Object.values(fieldScenes);
const hold = 6;
const transition = 8;

export function fieldMix(seconds: number): FieldMix {
  const period = hold + transition;
  const index = Math.floor(seconds / period) % scenes.length;
  const position = Math.max(0, (seconds % period - hold) / transition);
  const blend = position ** 3 * (position * (position * 6 - 15) + 10);
  return [scenes[index], scenes[(index + 1) % scenes.length], blend];
}

// Keep the spatial profiles in sync with the WebGL vertex shader.
function fieldProfile(scene: FieldScene, u: number, v: number, time: number): number {
  switch (scene) {
    case fieldScenes.current:
      return 0;
    case fieldScenes.aurora: {
      const upper = v + .32 + .11 * Math.sin(u * 4.2 + time * .24) + .04 * Math.cos(u * 9 - time * .38);
      const lower = v - .40 + .10 * Math.sin(u * 3.5 - time * .21);
      const drapes = Math.exp(-upper * upper * 61) + .76 * Math.exp(-lower * lower * 82);
      const fold = .5 + .5 * Math.cos(u * 30 + v * 6 + 1.6 * Math.sin(u * 5 - time * .32));
      const glow = .68 + .32 * Math.sin(u * 4.6 - v * 2.1 + time * .51);
      return drapes * (.43 + .89 * fold ** 2) * glow;
    }
    case fieldScenes.rain: {
      const x = u - v * .42 + .04 * Math.sin(v * 5 - time * .21);
      const lane = (.5 + .5 * Math.cos(x * 48)) ** 4;
      const drop = .5 + .5 * Math.cos(v * 19 - time * 1.28 + 2.3 * Math.sin(x * 22));
      const streams = .49 + .51 * Math.cos(x * 8 + time * .28) ** 2;
      return lane * (.24 + 1.13 * drop ** 3) * streams;
    }
    case fieldScenes.contours: {
      const x = u + .09 * Math.sin(v * 5 + time * .21);
      const upper = Math.exp(-((x + .53) ** 2 * 3.3 + (v + .35) ** 2 * 7));
      const lower = Math.exp(-((x - .58) ** 2 * 3.1 + (v - .34) ** 2 * 9));
      const relief = Math.max(upper, lower * .92);
      const contour = (.5 + .5 * Math.cos(relief * 32 - time * .53)) ** 4;
      return 1.62 * relief * (1 - .36 * relief) * (.07 + contour);
    }
    case fieldScenes.constellation: {
      const x = u + .10 * Math.sin(v * 4.8 - time * .24);
      const y = v + .08 * Math.cos(u * 4.3 + time * .21);
      const a = (.5 + .5 * Math.cos(x * 18 + y * 27 + time * .49)) ** 5;
      const b = (.5 + .5 * Math.cos(x * 18 - y * 27 - time * .38)) ** 5;
      const glimmer = .43 + .57 * (.5 + .5 * Math.sin(x * 8 - y * 5 + time * .63));
      return (.43 * Math.max(a, b) + .90 * a * b) * glimmer;
    }
    case fieldScenes.dunes: {
      const ridge = v + .15 * Math.sin(u * 3.4 - time * .26) + .064 * Math.sin(u * 7 + v * 1.8 + time * .17);
      const crest = (.5 + .5 * Math.cos(ridge * 31 + u * 3.2 + v * v * 2.1 - time * .58)) ** 4;
      const upper = v + .37 + .12 * Math.cos(u * 2.2 - time * .12);
      const lower = v - .38 + .15 * Math.sin(u * 2 + time * .13);
      return crest * (Math.exp(-lower * lower * 10) + .82 * Math.exp(-upper * upper * 14));
    }
    case fieldScenes.orbit: {
      const x = u + .08 * Math.sin(time * .19), y = v * 1.7;
      const radius = Math.hypot(x, y);
      const contour = .72 + .08 * Math.cos(time * .22 + x * 3);
      const arcs = .38 + .62 * (.5 + .5 * Math.sin(x * 5 - y * 3 + time * .43));
      return Math.exp(-((radius - contour) ** 2) * 13)
        * (.26 + .74 * (.5 + .5 * Math.cos(radius * 35 - time * .65)) ** 5) * arcs;
    }
    case fieldScenes.interference: {
      const left = Math.hypot(u + .61, (v + .34 + .09 * Math.sin(time * .16)) * 1.18);
      const right = Math.hypot(u - .59, (v - .31 + .08 * Math.cos(time * .19)) * 1.2);
      const front = (.5 + .5 * Math.cos(left * 24 - time * .81)) ** 4;
      const back = (.5 + .5 * Math.cos(right * 24 - time * .67 + 1.4)) ** 4;
      const reach = front * Math.exp(-left * left * 1.25) + back * Math.exp(-right * right * 1.25);
      return .69 * reach + .31 * front * back;
    }
    case fieldScenes.weave: {
      const a = Math.sin(u * 7.3 - v * 4.6 + time * .29);
      const b = Math.sin(u * 5.2 + v * 7.4 - time * .39);
      return (.5 + .5 * Math.cos((a + b) * 5.7)) ** 5
        * (.34 + .66 * (.5 + .5 * Math.sin(u * 2.9 + v * 5 + time * .26)));
    }
    case fieldScenes.prism: {
      const x = u + .91 + .11 * Math.sin(time * .19), y = v + .52;
      const distance = Math.hypot(x, y);
      const angle = Math.atan2(y, Math.max(.04, x));
      const spread = angle - .46 - .12 * Math.sin(time * .18);
      const ribs = (.5 + .5 * Math.cos(angle * 38 + .73 * Math.sin(distance * 8 - time * .45))) ** 3;
      const light = .65 + .35 * (.5 + .5 * Math.cos(distance * 18 - time * 1.05));
      return (.12 + .96 * ribs) * light * Math.exp(-spread * spread * 4 - (distance - .93) ** 2 * 1.2);
    }
    case fieldScenes.tide: {
      const first = u + .38 + Math.sin(v * 5 + time * .34) * .25;
      const second = u - .44 + Math.cos(v * 4 - time * .29) * .22;
      const bands = Math.exp(-first * first * 17) + Math.exp(-second * second * 19);
      return bands * (.29 + .71 * (.5 + .5 * Math.cos(v * 15 + u * 3 - time * .58)) ** 3);
    }
  }
}

export function fieldVariation(u: number, v: number, time: number, [from, to, blend]: FieldMix): number {
  const first = fieldProfile(from, u, v, time);
  return blend ? first * (1 - blend) + fieldProfile(to, u, v, time) * blend : first;
}
