import test from "node:test";
import assert from "node:assert/strict";
import { fieldHue, fieldMix, fieldScenes, fieldVariation } from "../src/field-motion.ts";

const { current, aurora, ribbons, weave, rain, dunes, contours, orbit, constellation, tide, prism, fan, interference } = fieldScenes;
const sequence = [current, aurora, ribbons, weave, rain, dunes, contours, orbit, constellation, tide, prism, fan, interference, current];
const sceneSeconds = 14;

test("the home field interleaves thirteen compositions before returning to its opening current", () => {
  assert.deepEqual(sequence.slice(0, -1).map((_, index) => fieldMix(index * sceneSeconds)),
    sequence.slice(0, -1).map((scene, index) => [scene, sequence[index + 1], 0]));
  assert.deepEqual(fieldMix(182), [current, aurora, 0]);
});

test("each composition has a distinct spatial profile and crossfades continuously into the next", () => {
  const samples = [[-.64, -.32], [.6, -.2], [.48, .34], [-.3, .38]];
  const profile = mix => samples.map(([u, v]) => fieldVariation(u, v, 2, mix));
  const profiles = sequence.map((_, index) => profile(fieldMix(index * sceneSeconds)));
  assert.equal(new Set(profiles.slice(0, -1).map(values => values.map(value => value.toFixed(3)).join())).size, 13);

  for (let index = 0; index < sequence.length - 1; index++) {
    const start = index * sceneSeconds;
    assert.deepEqual(fieldMix(start + 10), [sequence[index], sequence[index + 1], .5]);
    assert.ok(fieldMix(start + 7)[2] < .02 && fieldMix(start + 13)[2] > .98);
    const midpoint = profile(fieldMix(start + 10));
    assert.ok(midpoint.every((value, sample) => Math.abs(value - (profiles[index][sample] + profiles[index + 1][sample]) / 2) < 1e-8));
    for (const edge of [start + 6, start + sceneSeconds]) {
      const before = profile(fieldMix(edge - .01));
      const after = profile(fieldMix(edge + .01));
      assert.ok(before.every((value, sample) => Math.abs(value - after[sample]) < 1e-7));
    }
  }
});

test("warm and cool scenes keep distinct colors and meet without a hue jump", () => {
  const circularDistance = (a, b) => Math.abs(a - b - Math.round(a - b));
  assert.ok(circularDistance(fieldHue(fieldMix(0)), fieldHue(fieldMix(70))) > .4);
  assert.ok(fieldHue([rain, dunes, .5]) > .8, "Rain blends into amber through violet and red.");
  for (let index = 1; index < sequence.length; index++) {
    const edge = index * sceneSeconds;
    assert.ok(circularDistance(fieldHue(fieldMix(edge - .01)), fieldHue(fieldMix(edge + .01))) < 1e-7);
  }
});
